//! Detects whether this process is running from an installed package
//! (MSIX / Microsoft Store) rather than the unpackaged GitHub NSIS build.
//!
//! `GetCurrentPackageFullName` returns `APPMODEL_ERROR_NO_PACKAGE` for a
//! process with no package identity, and some other result (typically
//! `ERROR_INSUFFICIENT_BUFFER`, since we pass a zero-length buffer) for a
//! packaged one — the pattern Microsoft documents for this exact check.
//! <https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getcurrentpackagefullname>
//! <https://devblogs.microsoft.com/insidemsix/is-this-a-packaged-process/>

use windows_sys::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

/// True when this process has a package identity (MSIX/Store install),
/// false for the unpackaged NSIS build.
pub fn is_packaged() -> bool {
    let mut length: u32 = 0;
    // SAFETY: a null buffer only asks Windows for the required length;
    // no memory is written through `packagefullname`.
    let result = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    is_packaged_result(result)
}

/// Pure decision behind [`is_packaged`], taking the raw `WIN32_ERROR` so
/// it can be unit-tested without calling the real Windows API.
fn is_packaged_result(win32_error: u32) -> bool {
    win32_error != APPMODEL_ERROR_NO_PACKAGE
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;

    #[test]
    fn no_package_error_means_unpackaged() {
        assert!(!is_packaged_result(APPMODEL_ERROR_NO_PACKAGE));
    }

    #[test]
    fn any_other_result_means_packaged() {
        assert!(is_packaged_result(ERROR_INSUFFICIENT_BUFFER));
        assert!(is_packaged_result(0));
    }
}
