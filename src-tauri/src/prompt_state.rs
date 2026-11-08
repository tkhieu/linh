use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

/// The pure state machine behind the prompt window: at most one URL can
/// be pending at a time. Kept free of any Tauri type so it is directly
/// unit-testable.
#[derive(Debug, Default, Clone, PartialEq)]
struct PromptMachine {
    pending_url: Option<String>,
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
    /// latest `set_pending` always wins).
    fn set_pending(&mut self, url: String) {
        self.pending_url = Some(url);
    }

    /// Clears the pending URL, if any, and returns it. Idempotent:
    /// taking from an already-empty machine is a safe no-op.
    fn take(&mut self) -> Option<String> {
        self.pending_url.take()
    }

    fn peek(&self) -> Option<&String> {
        self.pending_url.as_ref()
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
        self.machine.lock().unwrap().peek().cloned()
    }

    /// Sets `url` pending, replacing any previous pending URL, and
    /// schedules a window reconcile.
    pub fn show(&self, app: &AppHandle, url: String) {
        self.machine.lock().unwrap().set_pending(url);
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
    fn reconcile(&self, app: &AppHandle) {
        let pending = self.machine.lock().unwrap().peek().cloned();
        apply(app, reconcile_action(pending));
    }
}

/// Schedules [`PromptState::reconcile`] on Tauri's main thread (required
/// for window operations). Looks the `AppState` back up from the cloned
/// `AppHandle` inside the closure, rather than capturing `&PromptState`
/// directly, since the closure must be `'static` and outlive this call.
fn schedule_reconcile(app: &AppHandle) {
    let app_for_closure = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(state) = app_for_closure.try_state::<AppState>() {
            state.prompt.reconcile(&app_for_closure);
        }
    });
}

fn apply(app: &AppHandle, action: PromptAction) {
    let Some(window) = app.get_webview_window("prompt") else {
        return;
    };
    match action {
        PromptAction::Show(url) => {
            let _ = window.show();
            let _ = window.set_focus();
            let _ = window.emit("prompt-url-changed", url);
        }
        PromptAction::Hide => {
            let _ = window.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_pending_twice_latest_wins() {
        let mut machine = PromptMachine::default();

        machine.set_pending("https://a.example".to_string());
        machine.set_pending("https://b.example".to_string());

        assert_eq!(machine.peek(), Some(&"https://b.example".to_string()));
    }

    #[test]
    fn take_clears_pending_and_returns_it() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string());

        let taken = machine.take();

        assert_eq!(taken, Some("https://a.example".to_string()));
        assert_eq!(machine.peek(), None);
    }

    #[test]
    fn take_after_take_is_idempotent() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string());

        let first = machine.take();
        let second = machine.take();

        assert_eq!(first, Some("https://a.example".to_string()));
        assert_eq!(second, None);
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
        machine.set_pending("https://a.example".to_string());
        machine.take(); // "hide" request's state change happens first here

        // The "show a" reconcile, running *after* the hide, must still
        // see the current (now-empty) state and decide to hide — not
        // replay a stale "show a" it was originally scheduled for.
        let current = machine.peek().cloned();
        assert_eq!(reconcile_action(current), PromptAction::Hide);
    }
}
