#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn cap_arenas() {
    // Called before Tauri initializes threads and WebKit.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn cap_arenas() {}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn trim() {
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn trim() {}
