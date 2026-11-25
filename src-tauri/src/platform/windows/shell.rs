use std::io;
use std::ptr;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn open_uri(uri: &str) -> Result<(), String> {
    let wide: Vec<u16> = uri.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // ShellExecuteW invokes Windows' registered URI handler; no command shell is involved.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "ShellExecuteW failed for {uri}: {}",
            io::Error::last_os_error()
        ))
    }
}
