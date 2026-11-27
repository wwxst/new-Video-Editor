//! A minimal native Win32 host window, used by `--embed-host` to prove that Slint can be created
//! as a real child window of an Agent-client shell in the same process.
//!
//! The host paints a 48 pt header and leaves the rest of its client area to the Slint child. Winit's
//! own event loop pumps this window's messages, so no second message loop is needed.

use std::ffi::c_void;
use std::sync::Once;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
    DeleteObject, DrawTextW, EndPaint, FillRect, HBRUSH, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, GetClientRect,
    RegisterClassW, SW_SHOW, ShowWindow, WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

const HEADER_H: i32 = 48;
static REGISTER: Once = Once::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client);
                let bg: HBRUSH = CreateSolidBrush(rgb(0x12, 0x13, 0x16));
                FillRect(hdc, &client, bg);
                DeleteObject(bg as _);
                let header = RECT {
                    left: 0,
                    top: 0,
                    right: client.right,
                    bottom: HEADER_H,
                };
                let hb: HBRUSH = CreateSolidBrush(rgb(0x1d, 0x1f, 0x25));
                FillRect(hdc, &header, hb);
                DeleteObject(hb as _);
                SetBkMode(hdc, 1 /* TRANSPARENT */);
                SetTextColor(hdc, rgb(0xe6, 0xe8, 0xec));
                let face = wide("Segoe UI");
                let font = CreateFontW(
                    -15,
                    0,
                    0,
                    0,
                    600,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    0,
                    0,
                    CLEARTYPE_QUALITY as u32,
                    DEFAULT_PITCH as u32,
                    face.as_ptr(),
                );
                let old = SelectObject(hdc, font as _);
                let mut text_rect = RECT {
                    left: 16,
                    top: 0,
                    right: client.right - 16,
                    bottom: HEADER_H,
                };
                let text = wide("AGENT CLIENT  /  native child-window host");
                DrawTextW(
                    hdc,
                    text.as_ptr(),
                    -1,
                    &mut text_rect,
                    0x24, /* DT_LEFT | DT_VCENTER | DT_SINGLELINE */
                );
                SelectObject(hdc, old);
                DeleteObject(font as _);
                EndPaint(hwnd, &mut ps);
                0
            }
            WM_ERASEBKGND => 1,
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Create and show the host. Returns `(hwnd, client_width, client_height)`.
pub fn create_host(width: u32, height: u32) -> Option<(isize, u32, u32)> {
    unsafe {
        REGISTER.call_once(|| {
            let class_name = wide("ArclineAgentHost");
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: GetModuleHandleW(std::ptr::null()),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wc);
        });
        let class_name = wide("ArclineAgentHost");
        let title = wide("Arcline Agent Client — Slint child host");
        let style = WS_OVERLAPPEDWINDOW | WS_VISIBLE;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        AdjustWindowRect(&mut rect, style, 0);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut::<c_void>(),
        );
        if hwnd.is_null() {
            return None;
        }
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        Some((
            hwnd as isize,
            client.right.max(1) as u32,
            client.bottom.max(1) as u32,
        ))
    }
}

pub fn header_height() -> i32 {
    HEADER_H
}

pub fn destroy_host(hwnd: isize) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd as _);
    }
}
