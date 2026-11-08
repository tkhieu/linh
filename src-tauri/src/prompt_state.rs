use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

/// The pure state machine behind the prompt window: at most one URL can
/// be pending at a time, and a transition always tells the caller which
/// window action to apply. Kept free of any Tauri type so it is directly
/// unit-testable.
#[derive(Debug, Default, Clone, PartialEq)]
struct PromptMachine {
    pending_url: Option<String>,
}

/// The window-level side effect a [`PromptMachine`] transition requires;
/// applied by [`PromptState`] while still holding the machine's lock, so
/// the logical state and the window can never drift apart.
#[derive(Debug, Clone, PartialEq)]
enum PromptAction {
    Show(String),
    Hide,
}

impl PromptMachine {
    /// Sets `url` pending, replacing any previous pending URL (the
    /// latest `set_pending` always wins).
    fn set_pending(&mut self, url: String) -> PromptAction {
        self.pending_url = Some(url.clone());
        PromptAction::Show(url)
    }

    /// Clears the pending URL, if any, and returns it. Hiding is always
    /// the right action afterwards, whether or not a URL was pending
    /// (an idempotent cancel/choose on an already-empty prompt must
    /// still hide, never error).
    fn take(&mut self) -> (Option<String>, PromptAction) {
        (self.pending_url.take(), PromptAction::Hide)
    }

    fn peek(&self) -> Option<&String> {
        self.pending_url.as_ref()
    }
}

/// Guards every transition between "no pending URL / prompt hidden" and
/// "pending URL set / prompt shown" with one lock, so the two can never
/// end up out of sync: there is no window where a pending URL exists
/// with a hidden prompt, or a shown prompt is hidden while its URL
/// remains pending.
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

    /// Sets `url` pending and shows+focuses the prompt window, emitting
    /// `prompt-url-changed` to it, all under the machine's lock so no
    /// other prompt transition can interleave between the state change
    /// and the window update.
    pub fn show(&self, app: &AppHandle, url: String) {
        let mut machine = self.machine.lock().unwrap();
        let action = machine.set_pending(url);
        apply(app, action);
    }

    /// Takes the pending URL (if any) and hides the prompt window,
    /// atomically. Returns the URL that was pending.
    pub fn take_and_hide(&self, app: &AppHandle) -> Option<String> {
        let mut machine = self.machine.lock().unwrap();
        let (url, action) = machine.take();
        apply(app, action);
        url
    }
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

        assert_eq!(
            machine.set_pending("https://a.example".to_string()),
            PromptAction::Show("https://a.example".to_string())
        );
        assert_eq!(
            machine.set_pending("https://b.example".to_string()),
            PromptAction::Show("https://b.example".to_string())
        );

        assert_eq!(machine.peek(), Some(&"https://b.example".to_string()));
    }

    #[test]
    fn take_clears_pending_and_reports_hide() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string());

        let (taken, action) = machine.take();

        assert_eq!(taken, Some("https://a.example".to_string()));
        assert_eq!(action, PromptAction::Hide);
        assert_eq!(machine.peek(), None);
    }

    #[test]
    fn take_on_an_empty_machine_still_reports_hide() {
        // An idempotent cancel/choose with nothing pending must still
        // hide the window, not error or no-op silently.
        let mut machine = PromptMachine::default();

        let (taken, action) = machine.take();

        assert_eq!(taken, None);
        assert_eq!(action, PromptAction::Hide);
    }

    #[test]
    fn take_after_take_is_idempotent() {
        let mut machine = PromptMachine::default();
        machine.set_pending("https://a.example".to_string());

        let (first, _) = machine.take();
        let (second, _) = machine.take();

        assert_eq!(first, Some("https://a.example".to_string()));
        assert_eq!(second, None);
    }
}
