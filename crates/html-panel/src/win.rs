//! 窓ありの起動（`Options::window`）の下回り：ブラウザを「前に出さずに」起動し、窓を画面の外へ隠す。
//!
//! 頭なし（headless）の Chrome は音を本当には鳴らさないので、音が要る時は窓のある Chrome を使う。
//! - 最小化・非アクティブ（SW_SHOWMINNOACTIVE）で、止めた状態で起動 → ジョブに入れてから動かす（proc）。
//! - 窓は見つけ次第、タスクバーと Alt+Tab から外し（WS_EX_TOOLWINDOW）、画面の外で元の大きさに戻す
//!   （最小化のままだと絵を描かなくなる）。フォーカスは取らせない。
//! - ジョブは KILL_ON_JOB_CLOSE：ゲームが落ちてもブラウザは残らない。

use std::path::Path;

pub const HIDDEN_X: i32 = -24000;
pub const HIDDEN_Y: i32 = -24000;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, LSFW_LOCK, LSFW_UNLOCK, LockSetForegroundWindow, GetClassNameW, GetForegroundWindow, SW_MINIMIZE, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, SW_HIDE,
        SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, SetWindowLongPtrW, SetWindowPlacement, ShowWindow, WINDOWPLACEMENT, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    use windows::core::BOOL;

    /// 最小化・非アクティブで起動し、ジョブに入れる（html-panel と共通）。
    pub fn spawn(exe: &Path, args: &[String]) -> Result<crate::proc::Proc, String> {
        // 遊んでいる人（ゲームが前面）から、起動したばかりの Chrome が前面を取らないよう、15 秒だけ
        // ほかのプロセスの SetForegroundWindow を止める。ゲームが前面でない時は効かない（その時は
        // hide_windows の見張りが、前に出た窓を最小化して明け渡す）
        unsafe {
            if LockSetForegroundWindow(LSFW_LOCK).is_ok() {
                let _ = std::thread::Builder::new().name("html-panel-unlock".to_owned()).spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    let _ = LockSetForegroundWindow(LSFW_UNLOCK);
                });
            }
        }
        crate::proc::spawn(exe, args, Some(SW_SHOWMINNOACTIVE.0 as u16))
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        list.push(hwnd);
        BOOL(1)
    }

    /// `pid` の Chrome の窓を、全部画面の外へ（タスクバーにも出さずに）。隠した窓の数を返す。
    pub fn hide_windows(pid: u32, size: [u32; 2]) -> usize {
        let mut all: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut Vec<HWND> as isize));
        }
        let mut moved = 0;
        for hwnd in all {
            unsafe {
                let mut owner = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut owner));
                if owner != pid || !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                let mut class = [0u16; 64];
                let n = GetClassNameW(hwnd, &mut class) as usize;
                if String::from_utf16_lossy(&class[..n]) != "Chrome_WidgetWin_1" {
                    continue;
                }
                // それでも前に出てしまった窓は、いったん最小化して前を明け渡す（Windows が前の窓に戻す）。
                // 最小化のままだと描かないので、すぐ下で画面の外に戻す
                if GetForegroundWindow() == hwnd {
                    let _ = ShowWindow(hwnd, SW_MINIMIZE);
                }
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let tool = WS_EX_TOOLWINDOW.0 as isize;
                let app = WS_EX_APPWINDOW.0 as isize;
                // 試験用（HTML_PANEL_TASKBAR=1）：タスクバーに残す（OBS の「アプリの音」で選べるように）
                let keep = std::env::var_os("HTML_PANEL_TASKBAR").is_some();
                let restyle = !keep && (ex & tool == 0 || ex & app != 0);
                if restyle {
                    // タスクバーの出入りは、一度隠して出し直すと効く
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | tool) & !app);
                }
                let mut rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rect);
                let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                let _ = GetWindowPlacement(hwnd, &mut placement);
                let minimized = placement.showCmd == SW_SHOWMINNOACTIVE.0 as u32 || placement.showCmd == 2 || placement.showCmd == 6;
                let away = rect.left <= HIDDEN_X / 2 && rect.top <= HIDDEN_Y / 2;
                if restyle || minimized || !away {
                    // 外枠と題の帯の分だけ大きく
                    let (w, h) = (size[0] as i32 + 16, size[1] as i32 + 120);
                    placement.showCmd = SW_SHOWNOACTIVATE.0 as u32;
                    placement.rcNormalPosition = RECT { left: HIDDEN_X, top: HIDDEN_Y, right: HIDDEN_X + w, bottom: HIDDEN_Y + h };
                    let _ = SetWindowPlacement(hwnd, &placement);
                    moved += 1;
                }
            }
        }
        moved
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn spawn(exe: &Path, args: &[String]) -> Result<crate::proc::Proc, String> {
        crate::proc::spawn(exe, args, None)
    }
    pub fn hide_windows(_pid: u32, _size: [u32; 2]) -> usize {
        0
    }
}

pub use imp::{hide_windows, spawn};
