//! Win32 AppBar registration so the sidebar reserves screen space on the right edge
//! (maximized windows stop at its left border, like the taskbar).

#[cfg(windows)]
pub mod imp {
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, ABE_RIGHT, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
    };

    fn data(hwnd: HWND) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: 0,
            uEdge: ABE_RIGHT,
            rc: RECT::default(),
            lParam: LPARAM(0),
        }
    }

    /// Physical rect of the monitor the window is on: (full, work).
    pub unsafe fn monitor_rects(hwnd: HWND) -> (RECT, RECT) {
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut mi);
            (mi.rcMonitor, mi.rcWork)
        }
    }

    /// Dock the window to the right edge. When `reserve` is true the space is reserved
    /// via the shell AppBar API. Safe to call repeatedly: any previous registration for
    /// this window is dropped first so the new position is computed against a clean
    /// work area (otherwise the shell would push us left of our own old reservation).
    /// Returns the final physical rect (x, y, w, h).
    pub unsafe fn dock_right(hwnd: HWND, width_px: i32, reserve: bool) -> (i32, i32, i32, i32) {
        unsafe {
            let mut abd = data(hwnd);
            SHAppBarMessage(ABM_REMOVE, &mut abd);

            let (full, work) = monitor_rects(hwnd);
            if reserve {
                SHAppBarMessage(ABM_NEW, &mut abd);
                abd.rc = RECT {
                    left: full.right - width_px,
                    top: full.top,
                    right: full.right,
                    bottom: full.bottom,
                };
                SHAppBarMessage(ABM_QUERYPOS, &mut abd);
                abd.rc.left = abd.rc.right - width_px;
                SHAppBarMessage(ABM_SETPOS, &mut abd);
            } else {
                abd.rc = RECT {
                    left: work.right - width_px,
                    top: work.top,
                    right: work.right,
                    bottom: work.bottom,
                };
            }
            let r = abd.rc;
            let _ = SetWindowPos(
                hwnd,
                None,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER,
            );
            (r.left, r.top, r.right - r.left, r.bottom - r.top)
        }
    }

    pub unsafe fn undock(hwnd: HWND) {
        unsafe {
            let mut abd = data(hwnd);
            SHAppBarMessage(ABM_REMOVE, &mut abd);
        }
    }

    /// Hide natively. We deliberately bypass tao's `set_visible(false)` here: its
    /// matching show path re-applies window styles and calls ShowWindow, which makes
    /// Windows nudge a window that sits entirely outside the work area (i.e. an
    /// appbar) back inside it. `dock_right` shows the window again via SetWindowPos.
    pub unsafe fn hide(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}
