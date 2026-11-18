//! Frames while Windows runs a modal loop of its own.
//!
//! Pressing a window's title bar or border, or opening its system menu, hands the thread to a
//! message loop inside `DefWindowProc` that returns only when the mouse is released or the menu
//! closes. winit waits for deadlines in its own loop, so none of them fire meanwhile: animations
//! stop, and a resize shows stale frames stretched over the new size until the drag ends.
//!
//! The modal loop still dispatches `WM_TIMER` and `WM_PAINT`. So each window is subclassed, and
//! around the messages that enter such a loop a timer runs that invalidates the window. winit turns
//! the paint into a redraw, and the runtime runs, holds or declines a frame exactly as it would
//! for any other redraw.

use std::ptr;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_NCRBUTTONDOWN, WM_SYSCOMMAND, WM_TIMER,
};

/// This subclass among any others on the window.
const SUBCLASS: usize = 0x7a67_7569;

/// The timer that invalidates the window during a modal loop.
const TIMER: usize = 0x7a67_7569;

/// How often the timer fires. Windows raises anything shorter to its 10 ms minimum and fires at
/// the system tick, so this asks for "as often as there is".
const INTERVAL_MS: u32 = 1;

/// Keeps `window` drawing through the modal loops Windows runs for it.
pub(crate) fn keep_drawing(window: &winit::window::Window) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    // SAFETY: `hwnd` is a live window this thread created, and the subclass removes itself when
    // the window is destroyed.
    unsafe {
        SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0);
    }
}

/// Runs a timer around the messages whose default handling is a modal loop.
unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        WM_SYSCOMMAND | WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN => {
            // SAFETY: called on the window's own thread with the window's own message.
            unsafe {
                SetTimer(hwnd, TIMER, INTERVAL_MS, None);
                let result = DefSubclassProc(hwnd, message, wparam, lparam);
                KillTimer(hwnd, TIMER);
                result
            }
        }
        WM_TIMER if wparam == TIMER => {
            // SAFETY: as above; a null rectangle invalidates the whole client area.
            unsafe {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_NCDESTROY => {
            // SAFETY: as above.
            unsafe {
                KillTimer(hwnd, TIMER);
                RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
        }
        // SAFETY: as above.
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}
