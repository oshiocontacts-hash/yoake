//! ブラウザのプロセスを立ち上げて、確実に片付ける。
//!
//! Windows では、止めた状態で起動してジョブ（KILL_ON_JOB_CLOSE）に入れてから動かす。
//! ゲームが落ちても、ジョブが閉じた時点でブラウザ（子プロセスごと）も終わる。

use std::path::Path;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, ResumeThread, STARTF_USESHOWWINDOW, STARTUPINFOW,
        TerminateProcess, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    pub struct Proc {
        process: HANDLE,
        job: Option<HANDLE>,
        pub pid: u32,
    }

    // ハンドルを閉じるまで持っているだけ。
    unsafe impl Send for Proc {}
    unsafe impl Sync for Proc {}

    pub(crate) fn quote(arg: &str) -> String {
        if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
            return arg.to_owned();
        }
        format!("\"{}\"", arg.replace('"', "\\\""))
    }

    fn job() -> Option<HANDLE> {
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set.is_err() {
                let _ = CloseHandle(job);
                return None;
            }
            Some(job)
        }
    }

    /// `show` は最初の窓の出し方（SW_* の値）。None なら指定しない（頭なしのとき）。
    pub fn spawn(exe: &Path, args: &[String], show: Option<u16>) -> Result<Proc, String> {
        let mut line = quote(&exe.display().to_string());
        for a in args {
            line.push(' ');
            line.push_str(&quote(a));
        }
        let mut wide: Vec<u16> = line.encode_utf16().chain(std::iter::once(0)).collect();
        let mut si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
        let mut flags: PROCESS_CREATION_FLAGS = CREATE_SUSPENDED;
        match show {
            Some(cmd) => {
                si.dwFlags = STARTF_USESHOWWINDOW;
                si.wShowWindow = cmd;
            }
            // 頭なしは窓を作らないが、念のためコンソールも出さない
            None => flags |= CREATE_NO_WINDOW,
        }
        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(PCWSTR::null(), Some(PWSTR(wide.as_mut_ptr())), None, None, false, flags, None, PCWSTR::null(), &si, &mut pi)
                .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
            let job = job();
            if let Some(j) = job {
                if AssignProcessToJobObject(j, pi.hProcess).is_err() {
                    eprintln!("html-panel: could not put the browser in a job; it may outlive a crash");
                }
            }
            ResumeThread(pi.hThread);
            let _ = CloseHandle(pi.hThread);
            Ok(Proc { process: pi.hProcess, job, pid: pi.dwProcessId })
        }
    }

    impl Proc {
        pub fn alive(&self) -> bool {
            unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT }
        }

        pub fn kill(&self) {
            unsafe {
                if self.alive() {
                    let _ = TerminateProcess(self.process, 0);
                    let _ = WaitForSingleObject(self.process, 3000);
                }
            }
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            self.kill();
            unsafe {
                let _ = CloseHandle(self.process);
                // ジョブを閉じると、残っている子プロセス（描く係・GPU 係など）も終わる
                if let Some(j) = self.job.take() {
                    let _ = CloseHandle(j);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::Mutex;

    pub struct Proc {
        child: Mutex<Child>,
        pub pid: u32,
    }

    pub fn spawn(exe: &Path, args: &[String], _show: Option<u16>) -> Result<Proc, String> {
        let child = Command::new(exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
        Ok(Proc { pid: child.id(), child: Mutex::new(child) })
    }

    impl Proc {
        pub fn alive(&self) -> bool {
            self.child.lock().map(|mut c| matches!(c.try_wait(), Ok(None))).unwrap_or(false)
        }

        pub fn kill(&self) {
            if let Ok(mut c) = self.child.lock() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

pub use imp::{Proc, spawn};
