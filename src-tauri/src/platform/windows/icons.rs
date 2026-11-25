//! Icon resolution for the `get_icon` Tauri command. Not yet implemented
//! on Windows (icons there are extracted from `.exe`/`.dll` resources,
//! not freedesktop theme directories).

use super::dirs::Dirs;

// ported in a later change.
pub fn resolve_icon(_dirs: &Dirs, _icon: &str) -> Option<String> {
    None
}
