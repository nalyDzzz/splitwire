//! Running processes, for the app picker.

use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub instances: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedApp {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
}

impl PickedApp {
    pub fn from_path(path: String) -> Self {
        let name = file_name(&path).to_string();
        let description = file_description(&path);
        Self { name, path, description }
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// One entry per executable, most-used first. Processes we can't inspect
/// (protected system processes) are skipped.
pub fn list() -> Vec<ProcessInfo> {
    let mut by_path: HashMap<String, ProcessInfo> = HashMap::new();
    for path in platform::running_paths() {
        by_path
            .entry(path.to_lowercase())
            .and_modify(|p| p.instances += 1)
            .or_insert_with(|| ProcessInfo {
                name: file_name(&path).to_string(),
                description: None,
                path,
                instances: 1,
            });
    }
    let mut list: Vec<ProcessInfo> = by_path.into_values().collect();
    for process in &mut list {
        process.description = file_description(&process.path);
    }
    let windows_dir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase();
    list.sort_by_cached_key(|p| (p.path.to_lowercase().starts_with(&windows_dir), p.name.to_lowercase()));
    list
}

pub fn file_description(path: &str) -> Option<String> {
    platform::file_description(path)
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub fn running_paths() -> Vec<String> {
        let mut paths = Vec::new();
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return paths;
            };
            let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
            while ok {
                if let Some(path) = process_path(entry.th32ProcessID) {
                    paths.push(path);
                }
                ok = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
        }
        paths
    }

    fn process_path(pid: u32) -> Option<String> {
        if pid == 0 || pid == 4 {
            return None; // Idle and System
        }
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(handle);
            result.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    /// The `FileDescription` from an exe's version resource, e.g. "Visual Studio Code".
    pub fn file_description(path: &str) -> Option<String> {
        unsafe {
            let file = HSTRING::from(path);
            let size = GetFileVersionInfoSizeW(&file, None);
            if size == 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            GetFileVersionInfoW(&file, None, size, data.as_mut_ptr() as *mut c_void).ok()?;

            let mut ptr: *mut c_void = std::ptr::null_mut();
            let mut len = 0u32;
            let translations: Vec<(u16, u16)> =
                if VerQueryValueW(data.as_ptr() as *const c_void, &HSTRING::from(r"\VarFileInfo\Translation"), &mut ptr, &mut len)
                    .as_bool()
                    && len >= 4
                {
                    std::slice::from_raw_parts(ptr as *const u16, (len / 2) as usize)
                        .chunks_exact(2)
                        .map(|c| (c[0], c[1]))
                        .collect()
                } else {
                    Vec::new()
                };

            // Fall back to US English / Unicode, the most common table.
            for (lang, codepage) in translations.into_iter().chain([(0x0409, 0x04b0)]) {
                let key = HSTRING::from(format!(r"\StringFileInfo\{lang:04x}{codepage:04x}\FileDescription"));
                if VerQueryValueW(data.as_ptr() as *const c_void, &key, &mut ptr, &mut len).as_bool() && len > 1 {
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len as usize - 1));
                    let text = text.trim_matches(char::from(0)).trim();
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            None
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn running_paths() -> Vec<String> {
        Vec::new()
    }

    pub fn file_description(_path: &str) -> Option<String> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn lists_this_process_and_reads_descriptions() {
        let me = std::env::current_exe().unwrap().to_string_lossy().to_lowercase();
        let list = super::list();
        assert!(list.iter().any(|p| p.path.to_lowercase() == me), "own process missing");

        let notepad = std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join(r"System32\notepad.exe");
        if notepad.exists() {
            assert!(super::file_description(&notepad.to_string_lossy()).is_some());
        }
    }
}
