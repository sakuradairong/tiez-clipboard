//! Read-only clipboard-owner/foreground source metadata. Never requests VM writes.
use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::DataExchange::GetClipboardOwner;
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
};

#[derive(Debug, Clone)]
pub struct ActiveAppInfo {
    pub app_name: String,
    pub process_path: Option<String>,
}
impl Default for ActiveAppInfo {
    fn default() -> Self {
        Self {
            app_name: "Unknown".into(),
            process_path: None,
        }
    }
}
fn resolve(window: HWND) -> Option<ActiveAppInfo> {
    if window.0.is_null() {
        return None;
    }
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 || pid == GetCurrentProcessId() {
            return None;
        }
        let mut class = [0u16; 256];
        let length = GetClassNameW(window, &mut class);
        let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
        if matches!(class.as_str(), "Shell_TrayWnd" | "Progman" | "WorkerW") {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut path = [0u16; 32768];
        let length = GetModuleFileNameExW(Some(process), None, &mut path);
        let _ = CloseHandle(process);
        if length == 0 || length as usize >= path.len() {
            return None;
        }
        let path = String::from_utf16_lossy(&path[..length as usize]);
        Some(ActiveAppInfo {
            app_name: Path::new(&path).file_name()?.to_string_lossy().into_owned(),
            process_path: Some(path),
        })
    }
}
pub fn get_clipboard_source_app_info() -> ActiveAppInfo {
    unsafe {
        GetClipboardOwner()
            .ok()
            .and_then(resolve)
            .or_else(|| resolve(GetForegroundWindow()))
            .unwrap_or_default()
    }
}
