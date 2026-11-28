//! Icon resolution for the `get_icon` Tauri command. `icon` is the `.app`
//! bundle path `browsers::detect_browsers` put in `Browser::icon`;
//! `sips`(1) (built into macOS) converts the bundle's `.icns` into a PNG
//! we can base64-encode, since decoding `.icns` ourselves would need a
//! dedicated image codec this project doesn't otherwise need.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use crate::updates::{current_dist, Dist};

use super::bundle;
use super::dirs::Dirs;

const SIPS: &str = "/usr/bin/sips";
const SIZE: &str = "128";
const MAX_ICON_BYTES: usize = 512 * 1024;

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `icon` must look like an `.app` bundle path — a bare file path (e.g.
/// one meant for Windows' `EXE,index` convention) must never be handed
/// to `sips`/`Info.plist` parsing below.
fn is_app_bundle_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("app")
}

pub fn resolve_icon(_dirs: &Dirs, icon: &str) -> Option<String> {
    if current_dist() == Dist::Mas {
        // `sips`(1) can't be exec'd from the sandbox (no exec rights
        // over arbitrary system binaries). The sandbox-safe alternative,
        // `NSWorkspace.icon(forFile:)`, only hands back an in-memory
        // `NSImage` — turning that into the PNG data URL this command's
        // contract promises would need an `NSBitmapImageRep` re-encode
        // this project has no other reason to carry, for a feature
        // (browser icons in pickers) the UI already degrades
        // gracefully without: `BrowserIcon` falls back to a generic
        // glyph whenever this returns `None`, same as any browser whose
        // icon file is missing on disk today.
        return None;
    }
    let bundle_path = Path::new(icon);
    if !is_app_bundle_path(bundle_path) || !bundle_path.is_dir() {
        return None;
    }
    let info = bundle::read_info_plist(bundle_path)?;
    let icon_file = info.icon_file?;
    let icon_path = bundle_path
        .join("Contents/Resources")
        .join(bundle::icns_file_name(&icon_file));
    if !icon_path.is_file() {
        return None;
    }
    convert_to_png_data_url(&icon_path)
}

fn convert_to_png_data_url(icon_path: &Path) -> Option<String> {
    let unique = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let out_path =
        std::env::temp_dir().join(format!("linh-icon-{}-{unique}.png", std::process::id()));

    let status = Command::new(SIPS)
        .args(["-s", "format", "png", "-Z", SIZE])
        .arg(icon_path)
        .arg("--out")
        .arg(&out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let result = match status {
        Ok(status) if status.success() => fs::read(&out_path).ok(),
        _ => None,
    };
    let _ = fs::remove_file(&out_path);

    let bytes = result?;
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
        return None;
    }
    Some(format!("data:image/png;base64,{}", BASE64.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_app_bundle_paths() {
        assert!(is_app_bundle_path(Path::new(
            "/Applications/Google Chrome.app"
        )));
        assert!(!is_app_bundle_path(Path::new(
            "/Applications/Example.app/Contents/Resources/app.icns"
        )));
        assert!(!is_app_bundle_path(Path::new("google-chrome")));
    }
}
