//! Small safe wrappers over the Win32 calls the service needs.

use std::ffi::c_void;

use anyhow::Context;
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION,
    TOKEN_QUERY,
};
use windows::Win32::System::Console::{
    AllocConsole, GenerateConsoleCtrlEvent, GetConsoleWindow, SetConsoleCtrlHandler, CTRL_BREAK_EVENT,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

/// Owned `SECURITY_ATTRIBUTES` built from an SDDL string.
pub struct SecurityAttributes {
    attrs: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

// The descriptor is an immutable, owned heap allocation.
unsafe impl Send for SecurityAttributes {}
unsafe impl Sync for SecurityAttributes {}

impl SecurityAttributes {
    pub fn from_sddl(sddl: &str) -> anyhow::Result<Self> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .context("invalid security descriptor")?;
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: BOOL(0),
        };
        Ok(Self { attrs, descriptor })
    }

    /// Pointer for APIs that take `LPSECURITY_ATTRIBUTES`. Valid while `self` lives.
    pub fn as_ptr(&self) -> *mut c_void {
        &self.attrs as *const SECURITY_ATTRIBUTES as *mut c_void
    }
}

impl Drop for SecurityAttributes {
    fn drop(&mut self) {
        unsafe {
            LocalFree(Some(HLOCAL(self.descriptor.0)));
        }
    }
}

/// A job object that kills its processes when the last handle closes, so
/// sing-box never outlives the service, even if the service crashes.
pub struct KillOnCloseJob(HANDLE);

unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    pub fn new() -> anyhow::Result<Self> {
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.context("CreateJobObject failed")?;
        let job = Self(job);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .context("SetInformationJobObject failed")?;
        Ok(job)
    }

    pub fn assign(&self, process: std::os::windows::io::RawHandle) -> anyhow::Result<()> {
        unsafe { AssignProcessToJobObject(self.0, HANDLE(process)) }.context("AssignProcessToJobObject failed")
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Services have no console, but sing-box only shuts down cleanly on a
/// console control event. Give the service a hidden console that child
/// processes inherit, and ignore control events aimed at the service itself.
pub fn ensure_console() {
    unsafe {
        if GetConsoleWindow().0.is_null() && AllocConsole().is_ok() {
            let _ = ShowWindow(GetConsoleWindow(), SW_HIDE);
        }
        let _ = SetConsoleCtrlHandler(Some(ignore_ctrl_event), true);
    }
}

unsafe extern "system" fn ignore_ctrl_event(_ctrl_type: u32) -> BOOL {
    BOOL(1)
}

/// Asks a process started with `CREATE_NEW_PROCESS_GROUP` (and sharing our
/// console) to exit, as if Ctrl+Break was pressed.
pub fn send_ctrl_break(pid: u32) -> anyhow::Result<()> {
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) }.context("GenerateConsoleCtrlEvent failed")
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}
