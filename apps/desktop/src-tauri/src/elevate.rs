//! Running the service installer with admin rights (shows a UAC prompt).

use std::path::Path;

#[cfg(windows)]
pub fn run_elevated(exe: &Path, args: &str) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if let Err(error) = ShellExecuteExW(&mut info) {
            let error: windows::core::Error = error;
            if error.code() == ERROR_CANCELLED.to_hresult() {
                bail!("The administrator prompt was cancelled");
            }
            return Err(error).context("couldn't start the service installer");
        }
        if info.hProcess.is_invalid() {
            bail!("couldn't start the service installer");
        }
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 || code != 0 {
            bail!("The service installer failed (exit code {code}). See the service log for details.");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn run_elevated(_exe: &Path, _args: &str) -> anyhow::Result<()> {
    anyhow::bail!("installing the service is only supported on Windows")
}
