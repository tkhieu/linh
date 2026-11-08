use std::sync::Mutex;
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager};

use crate::platform::linux::logging;
use crate::state::AppState;

/// A pending prompt URL, plus the moment its URL was received (for
/// `receive->prompt_shown` logging) and whether that log line has
/// already been emitted for it.
#[derive(Debug, Clone, PartialEq)]
struct PendingPrompt {
    url: String,
    received_at: Instant,
    shown_logged: bool,
}

/// The pure state machine behind the prompt window: at most one URL can
/// be pending at a time. Kept free of any Tauri type so it is directly
/// unit-testable.
#[derive(Debug, Default, Clone, PartialEq)]
struct PromptMachine {
    pending: Option<PendingPrompt>,
}

/// The window-level side effect that matches a given pending state;
/// always re-derived fresh from the *current* state by
/// [`reconcile_action`], never carried forward from whatever request
/// originally triggered a reconcile. That is what makes reconciling
/// idempotent and convergent regardless of the order in which several
/// overlapping requests end up actually running on the main thread.
#[derive(Debug, Clone, PartialEq)]
enum PromptAction {
    Show(String),
    Hide,
}

impl PromptMachine {
    /// Sets `url` pending, replacing any previous pending URL (the
    /// latest `set_pending` always wins). The new pending URL has not
    /// logged its "shown" event yet, even if the one it replaced had.
    fn set_pending(&mut self, url: String, received_at: Instant) {
        self.pending = Some(PendingPrompt {
            url,
            received_at,
            shown_logged: false,
        });
    }

    /// Clears the pending URL, if any, and returns it. Idempotent:
    /// taking from an already-empty machine is a safe no-op.
    fn take(&mut self) -> Option<String> {
        self.pending.take().map(|p| p.url)
    }

    fn peek(&self) -> Option<&str> {
        self.pending.as_ref().map(|p| p.url.as_str())
    }

    /// If there is a pending URL that hasn't logged its "shown" event
    /// yet, marks it logged and returns the moment it was received, so
    /// the caller can log `receive->prompt_shown` exactly once per
    /// pending URL (only after the window has actually shown it).
    /// Returns `None` if nothing is pending, or it was already logged.
    fn mark_shown_logged(&mut self) -> Option<Instant> {
        let pending = self.pending.as_mut()?;
        if pending.shown_logged {
            return None;
        }
        pending.shown_logged = true;
        Some(pending.received_at)
    }
}

/// The reconcile decision for a given pending state: show (with that
/// URL) iff one is pending, hide otherwise. Pure and total — it only
/// ever looks at the state handed to it, never at history, so the same
/// state always reconciles to the same action no matter which of
/// several queued requests happens to be the one that computed it.
fn reconcile_action(pending: Option<String>) -> PromptAction {
    match pending {
        Some(url) => PromptAction::Show(url),
        None => PromptAction::Hide,
    }
}

/// Guards every transition between "no pending URL / prompt hidden" and
/// "pending URL set / prompt shown" with one lock, so the two can never
/// end up out of sync. Every actual window operation is reconciled from
/// the *current* state at the moment it runs on the main thread (see
/// [`PromptState::reconcile`]), rather than applying whatever action a
/// caller happened to request — so a reconcile that runs late, or out
/// of the order its request was made in, still converges the window to
/// the true current state instead of a stale one.
#[derive(Default)]
pub struct PromptState {
    machine: Mutex<PromptMachine>,
}

impl PromptState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently pending URL, if any.
    pub fn pending_url(&self) -> Option<String> {
        self.machine.lock().unwrap().peek().map(str::to_string)
    }

    /// Sets `url` pending, replacing any previous pending URL, and
    /// schedules a window reconcile. `received_at` is when the URL was
    /// actually received (see [`crate::routing::route_url`]); once the
    /// window has actually shown it, the reconcile logs
    /// `receive->prompt_shown` timed from this moment.
    pub fn show(&self, app: &AppHandle, url: String, received_at: Instant) {
        self.machine.lock().unwrap().set_pending(url, received_at);
        schedule_reconcile(app);
    }

    /// Takes the pending URL (if any), schedules a window reconcile,
    /// and returns the URL that was pending.
    pub fn take_and_hide(&self, app: &AppHandle) -> Option<String> {
        let taken = self.machine.lock().unwrap().take();
        schedule_reconcile(app);
        taken
    }

    /// Re-reads the current pending state under the lock and applies
    /// the matching window action. Must run on the main thread — window
    /// operations require it, and reconciling here (rather than in
    /// `show`/`take_and_hide` directly) is what makes a reconcile that
    /// runs after a later one has already landed a no-op instead of a
    /// regression: it re-derives the action from whatever is true *now*.
    ///
    /// Logs `receive->prompt_shown` right after `window.show()` actually
    /// succeeds, for the first successful show of that pending URL only
    /// (a later reconcile that finds the same URL still pending does not
    /// log again). Logs an error line instead if showing the window
    /// fails.
    fn reconcile(&self, app: &AppHandle, dirs: &crate::platform::linux::dirs::Dirs) {
        let pending = self.machine.lock().unwrap().peek().map(str::to_string);
        let Some(window) = app.get_webview_window("prompt") else {
            logging::log_line(dirs, "failed to reconcile prompt window: window not found");
            return;
        };
        match reconcile_action(pending) {
            PromptAction::Show(url) => match window.show() {
                Ok(()) => {
                    let _ = window.set_focus();
                    let _ = window.emit("prompt-url-changed", &url);
                    if let Some(received_at) = self.machine.lock().unwrap().mark_shown_logged() {
                        logging::log_line(
                            dirs,
                            &format!(
                                "prompt shown for {url} (receive->prompt_shown {}ms)",
                                received_at.elapsed().as_millis()
                            ),
                        );
                    }
                }
                Err(err) => {
                    logging::log_line(
                        dirs,
                        &format!("failed to show prompt window for {url}: {err}"),
                    );
                }
            },
            PromptAction::Hide => {
                let _ = window.hide();
            }
        }
    }
}

/// Schedules [`PromptState::reconcile`] on Tauri's main thread (required
/// for window operations). Looks `AppState` back up from the cloned
/// `AppHandle` inside the closure, rather than capturing `&PromptState`
/// directly, since the closure must be `'static` and outlive this call.
/// Logs an error line if scheduling itself fails.
fn schedule_reconcile(app: &AppHandle) {
    let app_for_closure = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Some(state) = app_for_closure.try_state::<AppState>() {
            state.prompt.reconcile(&app_for_closure, &state.dirs);
        }
    });
    if let Err(err) = result {
        if let Some(state) = app.try_state::<AppState>() {
            logging::log_line(
                &state.dirs,
                &format!("failed to schedule prompt reconcile: {err}"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_pending_twice_latest_wins() {
        let mut machine = PromptMachine::default();

        machine.set_pending("https://a.example".to_string(), Instant::now());
        machine.set_pending("https://b.example".to_string(), Instant::now());

        assert_eq!(machine.peek(), Some("https://b.example"));
    }

    #[test]
    fn take_clears_pending_and_returns_it() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string(), Instant::now());

        let taken = machine.take();

        assert_eq!(taken, Some("https://a.example".to_string()));
        assert_eq!(machine.peek(), None);
    }

    #[test]
    fn take_after_take_is_idempotent() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string(), Instant::now());

        let first = machine.take();
        let second = machine.take();

        assert_eq!(first, Some("https://a.example".to_string()));
        assert_eq!(second, None);
    }

    #[test]
    fn mark_shown_logged_returns_received_at_once_then_none() {
        let mut machine = PromptMachine::default();
        let received_at = Instant::now();
        machine.set_pending("https://a.example".to_string(), received_at);

        let first = machine.mark_shown_logged();
        let second = machine.mark_shown_logged();

        assert_eq!(first, Some(received_at));
        assert_eq!(
            second, None,
            "must log the shown event only once per pending URL"
        );
    }

    #[test]
    fn mark_shown_logged_on_an_empty_machine_returns_none() {
        let mut machine = PromptMachine::default();
        assert_eq!(machine.mark_shown_logged(), None);
    }

    #[test]
    fn set_pending_again_resets_shown_logged_for_the_new_url() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string(), Instant::now());
        assert!(machine.mark_shown_logged().is_some());

        machine.set_pending("https://b.example".to_string(), Instant::now());

        assert!(
            machine.mark_shown_logged().is_some(),
            "a newly-pending URL must be loggable again"
        );
    }

    #[test]
    fn reconcile_action_shows_when_a_url_is_pending() {
        assert_eq!(
            reconcile_action(Some("https://a.example".to_string())),
            PromptAction::Show("https://a.example".to_string())
        );
    }

    #[test]
    fn reconcile_action_hides_when_nothing_is_pending() {
        assert_eq!(reconcile_action(None), PromptAction::Hide);
    }

    #[test]
    fn reconcile_action_depends_only_on_the_current_state_not_request_order() {
        // Simulate two requests ("show a", then "hide") landing on the
        // main thread in *reverse* order: whichever reconcile runs
        // last must reflect whatever the state actually is at that
        // moment, not the request that triggered it.
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string(), Instant::now());
        machine.take(); // "hide" request's state change happens first here

        // The "show a" reconcile, running *after* the hide, must still
        // see the current (now-empty) state and decide to hide — not
        // replay a stale "show a" it was originally scheduled for.
        let current = machine.peek().map(str::to_string);
        assert_eq!(reconcile_action(current), PromptAction::Hide);
    }
}
