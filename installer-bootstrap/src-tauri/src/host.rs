use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ExistingInstall {
    pub display_version: Option<String>,
    pub install_location: Option<String>,
    /// Used when launching the installed app. Linux builds do not read it.
    #[allow(dead_code)]
    pub main_binary_name: Option<String>,
}

pub fn default_install_dir() -> Option<String> {
    #[cfg(windows)]
    {
        let local = std::env::var("LOCALAPPDATA").ok()?;
        let local = local.trim().trim_end_matches(['\\', '/']);
        if local.is_empty() {
            return None;
        }
        Some(format!("{local}\\TieZ"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn read_existing_install() -> ExistingInstall {
    #[cfg(windows)]
    {
        windows::read_existing_install()
    }
    #[cfg(not(windows))]
    {
        ExistingInstall::default()
    }
}

pub fn free_bytes_for(path: &str) -> Option<u64> {
    #[cfg(windows)]
    {
        windows::free_bytes_for(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

pub fn pick_folder(title: &str) -> Option<String> {
    #[cfg(windows)]
    {
        windows::pick_folder(title)
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

pub fn webview2_present() -> bool {
    #[cfg(windows)]
    {
        windows::webview2_present()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

pub fn show_webview2_required_dialog() {
    #[cfg(windows)]
    windows::show_webview2_required_dialog();
}

pub fn extract_setup(bytes: &[u8], version: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("tiez-bootstrapper").join(version);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("TieZ_{version}_x64-setup.exe"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

pub fn run_setup(setup: &Path, raw_tail: &str) -> Result<i32, String> {
    #[cfg(windows)]
    {
        windows::run_setup(setup, raw_tail)
    }
    #[cfg(not(windows))]
    {
        let _ = (setup, raw_tail);
        Err("windows_only".into())
    }
}

pub fn launch_installed() -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::launch_installed()
    }
    #[cfg(not(windows))]
    {
        Err("windows_only".into())
    }
}

pub fn reveal_path(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::reveal_path(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("windows_only".into())
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;

    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    use tiez_installer_core::{
        resolve_launch_exe, webview2_client_key, webview2_client_wow64_key, UNINSTALL_SUBKEY,
    };

    use super::ExistingInstall;

    pub fn read_existing_install() -> ExistingInstall {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(UNINSTALL_SUBKEY) else {
            return ExistingInstall::default();
        };
        ExistingInstall {
            display_version: read_string(&key, "DisplayVersion"),
            install_location: read_string(&key, "InstallLocation"),
            main_binary_name: read_string(&key, "MainBinaryName"),
        }
    }

    fn read_string(key: &RegKey, name: &str) -> Option<String> {
        key.get_value::<String, _>(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn webview2_present() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        pv_installed(&hkcu, webview2_client_key())
            || pv_installed(&hklm, webview2_client_key())
            || pv_installed(&hklm, webview2_client_wow64_key())
    }

    fn pv_installed(root: &RegKey, subkey: &str) -> bool {
        let Ok(key) = root.open_subkey(subkey) else {
            return false;
        };
        let Ok(pv) = key.get_value::<String, _>("pv") else {
            return false;
        };
        let pv = pv.trim();
        !pv.is_empty() && pv != "0.0.0.0"
    }

    pub fn show_webview2_required_dialog() {
        let text = wide(
            "TieZ 安装向导需要 Microsoft Edge WebView2 运行时才能显示界面。\
请先安装 WebView2，然后重新运行本程序。\n\n\
The TieZ install wizard needs the Microsoft Edge WebView2 Runtime before its window can open. \
Install WebView2, then run this program again.",
        );
        let caption = wide("TieZ");
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                0x0000_0030,
            );
        }
    }

    pub fn free_bytes_for(path: &str) -> Option<u64> {
        let mut current = path.to_string();
        for _ in 0..32 {
            if let Some(bytes) = free_bytes_exact(&current) {
                return Some(bytes);
            }
            let parent = parent_dir(&current)?;
            if parent == current {
                return None;
            }
            current = parent;
        }
        None
    }

    fn free_bytes_exact(path: &str) -> Option<u64> {
        let wide_path = wide(path);
        let mut free = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide_path.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            None
        } else {
            Some(free)
        }
    }

    fn parent_dir(path: &str) -> Option<String> {
        let trimmed = path.trim_end_matches('\\');
        if trimmed.len() <= 3 && trimmed.as_bytes().get(1) == Some(&b':') {
            return None;
        }
        let index = trimmed.rfind('\\')?;
        let parent = &trimmed[..index];
        if parent.len() == 2 && parent.as_bytes().get(1) == Some(&b':') {
            return Some(format!("{parent}\\"));
        }
        if parent.is_empty() {
            None
        } else {
            Some(parent.to_string())
        }
    }

    pub fn pick_folder(title: &str) -> Option<String> {
        let path = rfd::FileDialog::new().set_title(title).pick_folder()?;
        Some(path.display().to_string())
    }

    /// Start the inner NSIS setup without a shell.
    /// `raw_tail` is `/S` or `/S /D=<unquoted path>` and must not be quoted by `arg()`.
    pub fn run_setup(setup: &Path, raw_tail: &str) -> Result<i32, String> {
        let status = Command::new(setup)
            .raw_arg(raw_tail)
            .status()
            .map_err(|error| format!("spawn_failed:{error}"))?;
        Ok(status.code().unwrap_or(1))
    }

    pub fn launch_installed() -> Result<(), String> {
        let existing = read_existing_install();
        let name = existing
            .main_binary_name
            .ok_or_else(|| "missing_main_binary_name".to_string())?;
        let location = existing.install_location.unwrap_or_default();
        let exe = resolve_launch_exe(&name, &location).map_err(|error| format!("{error:?}"))?;
        let path = Path::new(&exe);
        if !path.is_file() {
            return Err(format!("missing_binary:{exe}"));
        }
        Command::new(path)
            .spawn()
            .map_err(|error| format!("spawn_failed:{error}"))?;
        Ok(())
    }

    pub fn reveal_path(path: &Path) -> Result<(), String> {
        let arg = format!("/select,\"{}\"", path.display());
        Command::new("explorer")
            .raw_arg(arg)
            .spawn()
            .map_err(|error| format!("spawn_failed:{error}"))?;
        Ok(())
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            utype: u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_available: *mut u64,
            total_bytes: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }
}
