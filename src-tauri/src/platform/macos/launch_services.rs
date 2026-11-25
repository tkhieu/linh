//! Thin wrappers around the LaunchServices C API (`CoreServices.framework`)
//! used for browser/app discovery and the default-handler get/set. Kept
//! separate from `browsers.rs`/`apps.rs`/`default_browser.rs` so the
//! `unsafe` FFI surface lives in one place; everything here touches real
//! system state and so isn't unit-tested (there is no macOS host to run
//! it on in this repo's CI/dev setup either — only the `macos-15` CI job
//! actually exercises it).

use std::path::PathBuf;

use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::error::CFError;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;

#[allow(non_snake_case)]
#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn LSCopyAllHandlersForURLScheme(
        inURLScheme: CFStringRef,
    ) -> core_foundation::array::CFArrayRef;
    fn LSCopyApplicationURLsForBundleIdentifier(
        inBundleIdentifier: CFStringRef,
        outError: *mut core_foundation::error::CFErrorRef,
    ) -> core_foundation::array::CFArrayRef;
    fn LSCopyDefaultHandlerForURLScheme(inURLScheme: CFStringRef) -> CFStringRef;
    fn LSSetDefaultHandlerForURLScheme(
        inURLScheme: CFStringRef,
        inHandlerBundleID: CFStringRef,
    ) -> i32;
}

/// Bundle identifiers registered to handle `scheme` (no `://`), e.g.
/// `"https"` or `"msteams"` — exactly as LaunchServices reports them
/// (historically lowercased, but callers should not rely on that).
pub fn handlers_for_scheme(scheme: &str) -> Vec<String> {
    let scheme = CFString::new(scheme);
    unsafe {
        let array_ref = LSCopyAllHandlersForURLScheme(scheme.as_concrete_TypeRef());
        if array_ref.is_null() {
            return Vec::new();
        }
        let array: CFArray<CFString> = CFArray::wrap_under_create_rule(array_ref);
        array.iter().map(|item| item.to_string()).collect()
    }
}

/// Installed `.app` bundle paths registered under `bundle_id`, most
/// preferred first. Usually zero or one; LaunchServices can list more
/// than one when the same bundle id is installed in multiple locations.
pub fn app_urls_for_bundle_id(bundle_id: &str) -> Vec<PathBuf> {
    let bundle_id = CFString::new(bundle_id);
    unsafe {
        let mut err: core_foundation::error::CFErrorRef = std::ptr::null_mut();
        let array_ref =
            LSCopyApplicationURLsForBundleIdentifier(bundle_id.as_concrete_TypeRef(), &mut err);
        if !err.is_null() {
            // Release the (+1, "Copy"-rule) error object we were handed;
            // there's nothing actionable to do with it here beyond
            // treating the lookup as having found nothing.
            drop(CFError::wrap_under_create_rule(err));
        }
        if array_ref.is_null() {
            return Vec::new();
        }
        let array: CFArray<CFURL> = CFArray::wrap_under_create_rule(array_ref);
        array.iter().filter_map(|url| url.to_path()).collect()
    }
}

/// The bundle id currently registered as `scheme`'s default handler, if
/// any.
pub fn default_handler_for_scheme(scheme: &str) -> Option<String> {
    let scheme = CFString::new(scheme);
    unsafe {
        let result = LSCopyDefaultHandlerForURLScheme(scheme.as_concrete_TypeRef());
        if result.is_null() {
            return None;
        }
        Some(CFString::wrap_under_create_rule(result).to_string())
    }
}

/// Registers `bundle_id` as `scheme`'s default handler. On macOS 12+ this
/// only *requests* the change — the OS shows its own confirmation UI
/// before the handler actually switches, so a successful return here
/// means the request was accepted, not that the switch has happened yet;
/// callers must re-check [`default_handler_for_scheme`] for the real
/// status rather than assume it from this call succeeding.
pub fn set_default_handler_for_scheme(scheme: &str, bundle_id: &str) -> Result<(), String> {
    let scheme_cf = CFString::new(scheme);
    let bundle_cf = CFString::new(bundle_id);
    let status = unsafe {
        LSSetDefaultHandlerForURLScheme(
            scheme_cf.as_concrete_TypeRef(),
            bundle_cf.as_concrete_TypeRef(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "LSSetDefaultHandlerForURLScheme({scheme}, {bundle_id}) failed: OSStatus {status}"
        ))
    }
}
