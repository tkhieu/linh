use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
use winreg::RegKey;

pub fn read_string(root: &RegKey, path: &str, name: &str) -> Option<String> {
    root.open_subkey_with_flags(path, KEY_READ)
        .ok()?
        .get_value(name)
        .ok()
}

pub fn current_user() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

pub fn local_machine() -> RegKey {
    RegKey::predef(HKEY_LOCAL_MACHINE)
}

pub fn classes_root() -> RegKey {
    RegKey::predef(HKEY_CLASSES_ROOT)
}

pub fn expand_env(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = &after[..end];
        if let Some(value) = std::env::var_os(key) {
            out.push_str(&value.to_string_lossy());
        } else {
            out.push_str(&rest[start..start + end + 2]);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}
