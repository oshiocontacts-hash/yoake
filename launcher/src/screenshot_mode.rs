//! `HORIZON_LAUNCHER_SCREENSHOT=<out.png>`: draws the launcher once for UI review, writes the
//! picture and exits. Inert unless that variable is set. Windows only.
//!
//! GPUI's Windows renderer can only render to an image in its test builds, so this takes the
//! window's picture from the system instead. The window never takes part in the desktop:
//!
//! - it is created hidden (`show: false`, no focus), so it has no taskbar button or place yet;
//! - it is moved far off every screen (while still hidden) and only shown if it really is there
//!   (`SW_SHOWNOACTIVATE`: never activated, never focused);
//! - `PrintWindow` (with `PW_RENDERFULLCONTENT`) copies what the window draws, even off screen.
//!
//! `HORIZON_LAUNCHER_SCREENSHOT_SIZE=1280x800` is the logical size (default 1280x800) and
//! `HORIZON_LAUNCHER_SCREENSHOT_WAIT_MS` how long to let the page settle (default 6000: the
//! opening and the page's background reads). Use fresh `HORIZON_LAUNCHER_HOME`-style folders, as
//! AGENTS.md says for any run of the launcher.
use std::path::PathBuf;

pub(crate) struct Shot {
    pub out: PathBuf,
    pub width: f32,
    pub height: f32,
    pub wait_ms: u64,
}

/// The screenshot asked for by the environment, if any.
pub(crate) fn wanted() -> Option<Shot> {
    let out = PathBuf::from(std::env::var_os("HORIZON_LAUNCHER_SCREENSHOT").filter(|v| !v.is_empty())?);
    let (width, height) = std::env::var("HORIZON_LAUNCHER_SCREENSHOT_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?))
        })
        .filter(|(w, h)| *w >= 320. && *h >= 240. && *w <= 4096. && *h <= 4096.)
        .unwrap_or((1280., 800.));
    let wait_ms = std::env::var("HORIZON_LAUNCHER_SCREENSHOT_WAIT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6000);
    Some(Shot {
        out,
        width,
        height,
        wait_ms,
    })
}

#[cfg(windows)]
pub(crate) use imp::drive;

#[cfg(not(windows))]
pub(crate) fn drive(_: gpui::AnyWindowHandle, _: Shot, cx: &mut gpui::App) {
    eprintln!("HORIZON_LAUNCHER_SCREENSHOT works on Windows only.");
    cx.quit();
}

#[cfg(windows)]
mod imp {
    use super::Shot;
    use gpui::{AnyWindowHandle, App, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
        SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };

    /// Where the window waits: far outside any arrangement of screens.
    const FAR: i32 = -30000;

    fn hwnd(window: &Window) -> Option<HWND> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut c_void),
            _ => None,
        }
    }

    fn rect(hwnd: HWND) -> RECT {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut r) };
        r
    }

    /// Moves the hidden window far away at the size asked for (device pixels), then shows it
    /// without activating it, but only if it really is out of every screen's way.
    fn place_and_show(window: &mut Window, shot: &Shot) -> Result<(), String> {
        let hwnd = hwnd(window).ok_or("no window handle")?;
        let scale = window.scale_factor();
        let (w, h) = (
            (shot.width * scale).round() as i32,
            (shot.height * scale).round() as i32,
        );
        unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), FAR, FAR, w, h, SWP_NOACTIVATE | SWP_NOZORDER) };
        let r = rect(hwnd);
        if r.left > FAR + 1000 || r.top > FAR + 1000 {
            return Err(format!(
                "the window did not stay off screen ({}, {}): not showing it",
                r.left, r.top
            ));
        }
        window.resize(gpui::size(gpui::px(shot.width), gpui::px(shot.height)));
        // A tool window that never takes focus: no taskbar button, no Alt+Tab entry. Set before
        // the window is shown, so it never appears in either.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let style = (style | (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) as isize)
                & !(WS_EX_APPWINDOW as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        }
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Ok(())
    }

    fn grab(hwnd: HWND) -> Result<image::RgbaImage, String> {
        let r = rect(hwnd);
        // The window must still be where we left it: if anything moved it onto a screen, take
        // nothing rather than copy over what the user is looking at.
        if r.left > FAR + 1000 || r.top > FAR + 1000 {
            return Err(format!(
                "the window is on screen ({}, {}): not taking the picture",
                r.left, r.top
            ));
        }
        let (w, h) = ((r.right - r.left).max(0), (r.bottom - r.top).max(0));
        if w == 0 || h == 0 {
            return Err("the window has no size".into());
        }
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let previous = SelectObject(memory, bitmap);
            // PW_RENDERFULLCONTENT: also what a DirectX window composes.
            let printed = PrintWindow(hwnd, memory, 2);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(
                memory,
                bitmap,
                0,
                h as u32,
                pixels.as_mut_ptr() as *mut c_void,
                &mut info,
                DIB_RGB_COLORS,
            );
            SelectObject(memory, previous);
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
            if printed == 0 || lines == 0 {
                return Err("the system would not copy the window".into());
            }
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            image::RgbaImage::from_raw(w as u32, h as u32, pixels).ok_or_else(|| "bad size".into())
        }
    }

    /// Shows the window off screen, waits, saves the picture, quits.
    pub(crate) fn drive(handle: AnyWindowHandle, shot: Shot, cx: &mut App) {
        cx.spawn(async move |cx| {
            let placed = handle
                .update(cx, |_, window, _| place_and_show(window, &shot))
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            if let Err(e) = placed {
                eprintln!("screenshot: {e}");
                cx.update(|cx| cx.quit());
                return;
            }
            // The size set while hidden may need a second look once shown: say it again, and ask
            // for fresh frames before and after the wait.
            for round in 0..2 {
                cx.background_executor()
                    .timer(Duration::from_millis(if round == 0 { shot.wait_ms } else { 800 }))
                    .await;
                let _ = handle.update(cx, |_, window, _| {
                    if round == 0 {
                        window.resize(gpui::size(gpui::px(shot.width), gpui::px(shot.height)));
                    }
                    window.refresh();
                });
            }
            cx.background_executor().timer(Duration::from_millis(800)).await;
            let taken = handle
                .update(cx, |_, window, _| {
                    let hwnd = hwnd(window).ok_or_else(|| "no window handle".to_string())?;
                    grab(hwnd)
                })
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            match taken {
                Ok(image) => match image.save(&shot.out) {
                    Ok(()) => println!(
                        "screenshot: {} ({}x{})",
                        shot.out.display(),
                        image.width(),
                        image.height()
                    ),
                    Err(e) => eprintln!("screenshot: {e}"),
                },
                Err(e) => eprintln!("screenshot: {e}"),
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    }
}
