//! Reads the handful of `Info.plist` keys Linh needs out of an `.app`
//! bundle. Pure parsing (no LaunchServices calls), so it's testable with
//! hand-written fixture plists even though this module only compiles on
//! macOS.

use std::path::Path;

use plist::Value;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BundleInfo {
    /// `CFBundleDisplayName`, falling back to `CFBundleName`.
    pub display_name: Option<String>,
    /// `CFBundleIconFile`, exactly as declared (may be missing its
    /// `.icns` extension — Apple allows that and expects readers to
    /// append it).
    pub icon_file: Option<String>,
}

/// Parses `<bundle_path>/Contents/Info.plist`. `None` if it's missing,
/// unreadable, or not a dictionary at the top level.
pub fn read_info_plist(bundle_path: &Path) -> Option<BundleInfo> {
    let plist_path = bundle_path.join("Contents/Info.plist");
    let value = Value::from_file(&plist_path).ok()?;
    let dict = value.as_dictionary()?;
    let display_name = dict
        .get("CFBundleDisplayName")
        .or_else(|| dict.get("CFBundleName"))
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let icon_file = dict
        .get("CFBundleIconFile")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(BundleInfo {
        display_name,
        icon_file,
    })
}

/// `icon_file` with a `.icns` extension appended if it doesn't already
/// have one (Apple allows `CFBundleIconFile` either way).
pub fn icns_file_name(icon_file: &str) -> String {
    if Path::new(icon_file).extension().is_some() {
        icon_file.to_string()
    } else {
        format!("{icon_file}.icns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_bundle(root: &Path, info_plist_xml: &str) {
        let contents = root.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), info_plist_xml).unwrap();
    }

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-bundle-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn reads_display_name_and_icon_file() {
        let root = temp_root("full");
        let bundle = root.join("Google Chrome.app");
        write_bundle(
            &bundle,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>Google Chrome</string>
  <key>CFBundleIconFile</key>
  <string>app.icns</string>
</dict>
</plist>"#,
        );
        let info = read_info_plist(&bundle).unwrap();
        assert_eq!(info.display_name.as_deref(), Some("Google Chrome"));
        assert_eq!(info.icon_file.as_deref(), Some("app.icns"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn falls_back_to_bundle_name_when_display_name_is_absent() {
        let root = temp_root("fallback");
        let bundle = root.join("Firefox.app");
        write_bundle(
            &bundle,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Firefox</string>
</dict>
</plist>"#,
        );
        let info = read_info_plist(&bundle).unwrap();
        assert_eq!(info.display_name.as_deref(), Some("Firefox"));
        assert_eq!(info.icon_file, None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_info_plist_yields_none() {
        let root = temp_root("missing");
        assert!(read_info_plist(&root.join("Nothing.app")).is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_info_plist_yields_none() {
        let root = temp_root("malformed");
        let bundle = root.join("Broken.app");
        write_bundle(&bundle, "not a plist");
        assert!(read_info_plist(&bundle).is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn icns_file_name_appends_extension_only_when_missing() {
        assert_eq!(icns_file_name("app"), "app.icns");
        assert_eq!(icns_file_name("app.icns"), "app.icns");
        assert_eq!(icns_file_name("app.tiff"), "app.tiff");
    }
}
