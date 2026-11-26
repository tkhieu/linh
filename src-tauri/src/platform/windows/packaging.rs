//! Detects whether this process is running from an installed package
//! (MSIX / Microsoft Store) rather than the unpackaged GitHub NSIS build.
//!
//! Calling `GetCurrentPackageFullName` with no buffer, per Microsoft's
//! own documented check:
//! - `ERROR_INSUFFICIENT_BUFFER` means the process has package identity.
//! - `APPMODEL_ERROR_NO_PACKAGE` means it doesn't.
//! - Any other value is an error, not a third kind of identity — treated
//!   as unpackaged (the safer default: it skips Linh's own HKCU writes
//!   only when we are sure a packaged install's manifest covers them).
//! <https://learn.microsoft.com/en-us/windows/msix/detect-package-identity>
//! <https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getcurrentpackagefullname>

use windows_sys::Win32::Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER};
use windows_sys::Win32::Storage::Packaging::Appx::{
    GetCurrentApplicationUserModelId, GetCurrentPackageFullName,
};

/// True when this process has a package identity (MSIX/Store install),
/// false for the unpackaged NSIS build.
pub fn is_packaged() -> bool {
    let mut length: u32 = 0;
    // SAFETY: a null buffer only asks Windows for the required length;
    // no memory is written through `packagefullname`.
    let result = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    match is_packaged_result(result) {
        PackageCheck::Packaged => true,
        PackageCheck::Unpackaged => false,
        PackageCheck::UnexpectedError(err) => {
            eprintln!(
                "Linh: GetCurrentPackageFullName returned unexpected error {err}; treating as unpackaged"
            );
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageCheck {
    Packaged,
    Unpackaged,
    /// Neither of the two documented results — an actual error, not a
    /// third identity state. See the module doc comment.
    UnexpectedError(u32),
}

/// Pure decision behind [`is_packaged`], taking the raw `WIN32_ERROR` so
/// it can be unit-tested without calling the real Windows API.
fn is_packaged_result(win32_error: u32) -> PackageCheck {
    match win32_error {
        ERROR_INSUFFICIENT_BUFFER => PackageCheck::Packaged,
        APPMODEL_ERROR_NO_PACKAGE => PackageCheck::Unpackaged,
        other => PackageCheck::UnexpectedError(other),
    }
}

/// This process' own Application User Model ID (`<PackageFamilyName>!
/// <AppId>` for a packaged/Desktop Bridge process), or `None` if it has
/// no package identity or Windows otherwise can't report one.
/// <https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getcurrentapplicationusermodelid>
pub fn current_application_user_model_id() -> Option<String> {
    let mut length: u32 = 0;
    // SAFETY: a null buffer only asks Windows for the required length.
    let err = unsafe { GetCurrentApplicationUserModelId(&mut length, std::ptr::null_mut()) };
    if err != ERROR_INSUFFICIENT_BUFFER {
        return None;
    }
    let mut buf = vec![0u16; length as usize];
    // SAFETY: `buf` is exactly the length Windows just reported.
    let err = unsafe { GetCurrentApplicationUserModelId(&mut length, buf.as_mut_ptr()) };
    if err != 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_buffer_means_packaged() {
        assert_eq!(
            is_packaged_result(ERROR_INSUFFICIENT_BUFFER),
            PackageCheck::Packaged
        );
    }

    #[test]
    fn no_package_error_means_unpackaged() {
        assert_eq!(
            is_packaged_result(APPMODEL_ERROR_NO_PACKAGE),
            PackageCheck::Unpackaged
        );
    }

    #[test]
    fn any_other_result_is_an_unexpected_error_not_a_third_state() {
        assert_eq!(is_packaged_result(0), PackageCheck::UnexpectedError(0));
        assert_eq!(is_packaged_result(5), PackageCheck::UnexpectedError(5));
    }
}
