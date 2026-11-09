//! Resolves an icon name (as it appears in a browser's or app's `Icon=`
//! value) or an absolute path to inline image bytes, for the `get_icon`
//! Tauri command. Never touches Tauri's asset protocol or
//! `tauri.conf.json`'s CSP: the file is read directly here and handed
//! back as a `data:` URL.

use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use super::dirs::Dirs;

/// Icons larger than this are refused rather than inlined.
const MAX_ICON_BYTES: u64 = 512 * 1024;

/// Hicolor theme sizes to try, including the scalable (SVG) variant.
/// Order doesn't affect correctness (every installed size/category pair
/// is a plain file-exists check), only which equally-valid match wins
/// when more than one size happens to exist.
const ICON_SIZES: &[&str] = &[
    "scalable", "512x512", "256x256", "192x192", "128x128", "96x96", "72x72", "64x64", "48x48",
    "36x36", "32x32", "24x24", "22x22", "16x16",
];

/// Hicolor theme contexts most app/browser icons are filed under.
const ICON_CATEGORIES: &[&str] = &["apps", "mimetypes", "categories", "status", "devices"];

const EXTENSIONS: &[&str] = &["png", "svg"];

/// Resolves `icon` to a `data:image/png;base64,...` or
/// `data:image/svg+xml;base64,...` URL. `icon` is either an absolute
/// path, or a freedesktop icon *name* (no extension) looked up across
/// the hicolor theme (every size, including `scalable`) in
/// `~/.local/share/icons`, `/usr/share/icons`, the Flatpak export icon
/// dirs, then flat `/usr/share/pixmaps`. Returns `None` if nothing is
/// found, the file isn't a PNG/SVG by magic bytes (regardless of its
/// extension), or it exceeds 512 KiB.
pub fn resolve_icon(dirs: &Dirs, icon: &str) -> Option<String> {
    let path = if Path::new(icon).is_absolute() {
        PathBuf::from(icon)
    } else {
        find_icon_path(dirs, icon)?
    };
    read_as_data_url(&path)
}

fn icon_theme_base_dirs(dirs: &Dirs) -> Vec<PathBuf> {
    vec![
        dirs.home.join(".local/share/icons"),
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/var/lib/flatpak/exports/share/icons"),
        dirs.home.join(".local/share/flatpak/exports/share/icons"),
    ]
}

fn find_icon_path(dirs: &Dirs, icon: &str) -> Option<PathBuf> {
    for base in icon_theme_base_dirs(dirs) {
        let hicolor = base.join("hicolor");
        for size in ICON_SIZES {
            for category in ICON_CATEGORIES {
                for ext in EXTENSIONS {
                    let candidate = hicolor
                        .join(size)
                        .join(category)
                        .join(format!("{icon}.{ext}"));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    for ext in EXTENSIONS {
        let candidate = PathBuf::from("/usr/share/pixmaps").join(format!("{icon}.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn read_as_data_url(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
        return None;
    }
    let data = fs::read(path).ok()?;
    let mime = detect_image_mime(&data)?;
    let encoded = BASE64.encode(&data);
    Some(format!("data:{mime};base64,{encoded}"))
}

/// Identifies PNG by magic bytes and SVG by a crude text sniff — never
/// by the file's extension, so a mislabeled or malicious file can't
/// smuggle an unexpected type through as a trusted image.
fn detect_image_mime(data: &[u8]) -> Option<&'static str> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data.starts_with(&PNG_MAGIC) {
        return Some("image/png");
    }
    let head = &data[..data.len().min(1024)];
    if String::from_utf8_lossy(head)
        .to_lowercase()
        .contains("<svg")
    {
        return Some("image/svg+xml");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-icons-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_dirs(root: &Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: Vec::new(),
        }
    }

    const PNG_BYTES: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    fn write(path: &Path, data: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    #[test]
    fn finds_png_in_hicolor_apps_dir() {
        let root = temp_root("hicolor-png");
        let dirs = test_dirs(&root);
        let icon_path = dirs
            .home
            .join(".local/share/icons/hicolor/48x48/apps/google-chrome.png");
        write(&icon_path, &PNG_BYTES);

        let result = resolve_icon(&dirs, "google-chrome").expect("resolved");
        assert!(result.starts_with("data:image/png;base64,"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn finds_svg_in_scalable_dir() {
        let root = temp_root("scalable-svg");
        let dirs = test_dirs(&root);
        let icon_path = dirs
            .home
            .join(".local/share/icons/hicolor/scalable/apps/zoom.svg");
        write(icon_path.as_path(), b"<?xml version=\"1.0\"?><svg></svg>");

        let result = resolve_icon(&dirs, "zoom").expect("resolved");
        assert!(result.starts_with("data:image/svg+xml;base64,"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lookup_falls_through_every_dir_to_none_when_nothing_matches() {
        // `/usr/share/pixmaps` is a fixed system path, not under `root`,
        // so this only proves the lookup tries every hicolor dir this
        // test controls and still returns `None` rather than panicking
        // or matching something unrelated — a real pixmaps hit is
        // exercised by hand on a real system, not hermetically here.
        let root = temp_root("fallthrough");
        let dirs = test_dirs(&root);
        assert_eq!(resolve_icon(&dirs, "some-nonexistent-icon-xyz"), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn absolute_path_is_used_directly() {
        let root = temp_root("absolute");
        let icon_path = root.join("custom/teams.png");
        write(&icon_path, &PNG_BYTES);
        let dirs = test_dirs(&root);

        let result = resolve_icon(&dirs, icon_path.to_str().unwrap()).expect("resolved");
        assert!(result.starts_with("data:image/png;base64,"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_icon_returns_none() {
        let root = temp_root("missing");
        let dirs = test_dirs(&root);
        assert_eq!(resolve_icon(&dirs, "does-not-exist"), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn non_image_content_is_rejected_despite_png_extension() {
        let root = temp_root("fake-png");
        let dirs = test_dirs(&root);
        let icon_path = dirs
            .home
            .join(".local/share/icons/hicolor/48x48/apps/fake.png");
        write(&icon_path, b"not actually a png");

        assert_eq!(resolve_icon(&dirs, "fake"), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn oversized_icon_is_rejected() {
        let root = temp_root("oversized");
        let dirs = test_dirs(&root);
        let icon_path = dirs
            .home
            .join(".local/share/icons/hicolor/512x512/apps/huge.png");
        let mut data = PNG_BYTES.to_vec();
        data.extend(std::iter::repeat_n(0u8, MAX_ICON_BYTES as usize));
        write(&icon_path, &data);

        assert_eq!(resolve_icon(&dirs, "huge"), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn absolute_path_still_requires_valid_image_bytes() {
        let root = temp_root("absolute-invalid");
        let icon_path = root.join("custom/evil.png");
        write(&icon_path, b"#!/bin/sh\necho hi\n");
        let dirs = test_dirs(&root);

        assert_eq!(resolve_icon(&dirs, icon_path.to_str().unwrap()), None);
        fs::remove_dir_all(&root).ok();
    }
}
