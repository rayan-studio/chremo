/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Native resize borders for the custom Windows title bar.
//!
//! Hit testing must run before mouse events reach egui or a web page. Returning
//! native non-client codes lets Windows own the cursor and the entire resize drag.

use std::io;
use std::ptr::null_mut;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetWindowLongW, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT,
    HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsIconic, IsZoomed, SM_CXPADDEDBORDER, SM_CXSIZEFRAME,
    SM_CYSIZEFRAME, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongW,
    SetWindowPos, WM_NCDESTROY, WM_NCHITTEST, WS_MAXIMIZEBOX, WS_THICKFRAME,
};
use winit::window::Window;

const RESIZE_SUBCLASS_ID: usize = 0x53565253;

pub(super) fn install(window: &Window) -> io::Result<()> {
    let handle = window.window_handle().map_err(io::Error::other)?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err(io::Error::other("Expected a Win32 window handle"));
    };
    // SAFETY: Called on the owning event-loop thread while the Window is alive.
    // No Rust references are stored in the callback. It removes itself on destruction.
    let hwnd = handle.hwnd.get() as HWND;
    if unsafe {
        // Undecorated winit windows normally lose the native sizing styles.
        // Restore them so Windows accepts HT* resize hit-test results.
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_THICKFRAME == 0 {
            SetWindowLongW(
                hwnd,
                GWL_STYLE,
                (style | WS_THICKFRAME | WS_MAXIMIZEBOX) as i32,
            );
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
        SetWindowSubclass(hwnd, Some(resize_proc), RESIZE_SUBCLASS_ID, 0)
    } == 0
    {
        return Err(io::Error::other("SetWindowSubclass failed"));
    }
    Ok(())
}

unsafe extern "system" fn resize_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _reference_data: usize,
) -> LRESULT {
    // SAFETY: Windows calls this procedure with a live HWND on its owning thread.
    // All output pointers refer to stack storage, and unhandled messages always
    // continue down the subclass chain (including winit and accessibility hooks).
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(resize_proc), subclass_id);
        } else if message == WM_NCHITTEST && IsZoomed(hwnd) == 0 && IsIconic(hwnd) == 0 {
            // winit removes WS_THICKFRAME in fullscreen and for non-resizable windows.
            if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_THICKFRAME != 0 {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    let padding = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    let border_x = (GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + padding).max(1);
                    let border_y = (GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + padding).max(1);
                    // GET_X/Y_LPARAM semantics: screen coordinates are signed, including
                    // monitors to the left of or above the primary monitor.
                    let x = lparam as i16 as i32;
                    let y = (lparam >> 16) as i16 as i32;
                    if let Some(hit) = resize_hit(&rect, x, y, border_x, border_y) {
                        return hit as LRESULT;
                    }
                }
            }
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

fn resize_hit(rect: &RECT, x: i32, y: i32, border_x: i32, border_y: i32) -> Option<u32> {
    if x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom {
        return None;
    }
    let left = x < rect.left + border_x;
    let right = x >= rect.right - border_x;
    let top = y < rect.top + border_y;
    let bottom = y >= rect.bottom - border_y;
    // Give the corners a larger grab target, confined to the resize border.
    let near_left = x < rect.left + 2 * border_x;
    let near_right = x >= rect.right - 2 * border_x;
    let near_top = y < rect.top + 2 * border_y;
    let near_bottom = y >= rect.bottom - 2 * border_y;
    match () {
        _ if (top && near_left) || (left && near_top) => Some(HTTOPLEFT),
        _ if (top && near_right) || (right && near_top) => Some(HTTOPRIGHT),
        _ if (bottom && near_left) || (left && near_bottom) => Some(HTBOTTOMLEFT),
        _ if (bottom && near_right) || (right && near_bottom) => Some(HTBOTTOMRIGHT),
        _ if left => Some(HTLEFT),
        _ if right => Some(HTRIGHT),
        _ if top => Some(HTTOP),
        _ if bottom => Some(HTBOTTOM),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_edges_and_corners_at_multiple_scales_and_monitor_positions() {
        for scale in [1, 2, 3] {
            for (left, top) in [(100, 100), (-1400, -800)] {
                let rect = RECT {
                    left,
                    top,
                    right: left + 800 * scale,
                    bottom: top + 600 * scale,
                };
                for (x, y, expected) in [
                    (1, 300, HTLEFT),
                    (799, 300, HTRIGHT),
                    (400, 1, HTTOP),
                    (400, 599, HTBOTTOM),
                    (1, 1, HTTOPLEFT),
                    (799, 1, HTTOPRIGHT),
                    (1, 599, HTBOTTOMLEFT),
                    (799, 599, HTBOTTOMRIGHT),
                    (12, 1, HTTOPLEFT),
                    (1, 12, HTTOPLEFT),
                ] {
                    assert_eq!(
                        resize_hit(
                            &rect,
                            left + x * scale,
                            top + y * scale,
                            8 * scale,
                            8 * scale
                        ),
                        Some(expected)
                    );
                }
                for (x, y) in [
                    (400, 22),
                    (400, 300),
                    (12, 12),
                    (-1, 300),
                    (800, 300),
                    (400, 600),
                ] {
                    assert_eq!(
                        resize_hit(
                            &rect,
                            left + x * scale,
                            top + y * scale,
                            8 * scale,
                            8 * scale
                        ),
                        None
                    );
                }
            }
        }
    }
}
