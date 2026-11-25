//! Extract an EXE or DLL icon with Win32 and encode a small PNG data URL.
use std::ptr;
use std::slice;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO, BI_RGB,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::Shell::ExtractIconExW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

use super::dirs::Dirs;
use super::registry::expand_env;

const SIZE: usize = 64;
const MAX_ICON_BYTES: usize = 512 * 1024;

fn icon_location(value: &str) -> Option<(String, i32)> {
    let value = value.trim();
    let (path, index) = match value.rsplit_once(',') {
        Some((path, index)) if index.trim().parse::<i32>().is_ok() => {
            (path, index.trim().parse().ok()?)
        }
        _ => (value, 0),
    };
    let path = expand_env(path.trim().trim_matches('"'));
    let lower = path.to_ascii_lowercase();
    if !lower.ends_with(".exe") && !lower.ends_with(".dll") {
        return None;
    }
    if !std::path::Path::new(&path).is_file() {
        return None;
    }
    Some((path, index))
}

pub fn resolve_icon(_dirs: &Dirs, icon: &str) -> Option<String> {
    let (path, index) = icon_location(icon)?;
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut handle = ptr::null_mut();
    let count = unsafe { ExtractIconExW(wide.as_ptr(), index, &mut handle, ptr::null_mut(), 1) };
    if count == 0 || handle.is_null() {
        return None;
    }
    let png = extract_png(handle);
    unsafe {
        DestroyIcon(handle);
    }
    let png = png?;
    if png.len() > MAX_ICON_BYTES {
        return None;
    }
    Some(format!("data:image/png;base64,{}", BASE64.encode(png)))
}

fn extract_png(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    // A top-down 32-bit DIB lets DrawIconEx composite the icon into RGBA pixels.
    let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = std::mem::size_of_val(&info.bmiHeader) as u32;
    info.bmiHeader.biWidth = SIZE as i32;
    info.bmiHeader.biHeight = -(SIZE as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits = ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe {
            DeleteDC(dc);
        }
        return None;
    }
    let old = unsafe { SelectObject(dc, bitmap) };
    let drawn = unsafe {
        DrawIconEx(
            dc,
            0,
            0,
            icon,
            SIZE as i32,
            SIZE as i32,
            0,
            ptr::null_mut(),
            DI_NORMAL,
        )
    } != 0;
    let result = if drawn {
        let bgra = unsafe { slice::from_raw_parts(bits as *const u8, SIZE * SIZE * 4) };
        let mut rgba = bgra.to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let mut encoded = Vec::new();
        let mut encoder = png::Encoder::new(&mut encoded, SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        match encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&rgba))
        {
            Ok(()) => Some(encoded),
            Err(_) => None,
        }
    } else {
        None
    };
    unsafe {
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_icon_location() {
        assert_eq!(
            icon_location(r#""C:\Windows\System32\shell32.dll",-1"#).map(|(_, i)| i),
            Some(-1)
        );
        assert!(icon_location("not-an-icon.txt").is_none());
    }
}
