//! Native external launch only. Callers own consent, revision checks and export lifetime.
//! Payloads are never logged, formatted as commands for a shell, or returned in errors.
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, RPC_E_CHANGED_MODE};
use windows::Win32::Storage::FileSystem::GetBinaryTypeW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, GetProcessId, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::System::WindowsProgramming::{SCS_32BIT_BINARY, SCS_64BIT_BINARY};
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, IApplicationActivationManager, IShellItem, IShellItemArray,
    SHCreateItemFromParsingName, SHCreateShellItemArrayFromShellItem, ShellExecuteExW,
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_UNICODE,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

// Deliberately no Debug: targets and selected applications can contain private data.
#[derive(Clone, Copy)]
pub enum Target<'a> {
    Path(&'a Path),
    Url(&'a str),
}
#[derive(Clone, Copy)]
pub enum Handler<'a> {
    System,
    Desktop(&'a Path),
    Packaged(&'a str),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    System,
    Desktop,
    Packaged,
}
/// Acceptance by Windows/process creation is not proof of display, file loading or editing.
#[derive(Debug)]
pub struct Receipt {
    pub route: Route,
    pub process_id: Option<u32>,
}
const MAX_WIDE: usize = 32_767;
fn wide(value: &OsStr, code: &'static str) -> Result<Vec<u16>, &'static str> {
    let units: Vec<_> = value.encode_wide().collect();
    if units.is_empty() || units.len() >= MAX_WIDE || units.contains(&0) {
        return Err(code);
    }
    Ok(units.into_iter().chain([0]).collect())
}
fn valid_identity(value: &str) -> bool {
    value.len() <= 255
        && value.split_once('!').is_some_and(|(family, app)| {
            !family.is_empty()
                && !app.is_empty()
                && !app.contains('!')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.!-".contains(&byte))
        })
}
impl Target<'_> {
    fn value(self) -> Result<OsString, &'static str> {
        match self {
            Self::Path(path) => {
                if !path.is_absolute() {
                    return Err("open_target");
                }
                if !path.exists() {
                    return Err("open_target_missing");
                }
                Ok(path.as_os_str().into())
            }
            Self::Url(url) => {
                // Full URL parsing/normalization belongs to content::open. Defend the OS boundary too.
                let Some((scheme, rest)) = url.split_once(':') else {
                    return Err("open_url");
                };
                let allowed = match scheme {
                    "http" | "https" | "ftp" | "ftps" => rest.starts_with("//") && rest.len() > 2,
                    "mailto" => !rest.is_empty(),
                    _ => false,
                };
                if !allowed || url.chars().any(char::is_control) {
                    return Err("open_url");
                }
                Ok(OsString::from(url))
            }
        }
    }
}
/// Scope COM on this calling thread. An already initialized MTA is kept intact.
struct Apartment(bool);
impl Apartment {
    fn enter() -> Result<Self, &'static str> {
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err("open_com")
        }
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}
/// Quote a single UTF-16 argument using the Windows CRT rules, including terminal slashes.
fn quote(units: &[u16], output: &mut Vec<u16>) {
    output.push(b'"' as u16);
    let mut slashes = 0;
    for &unit in units {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            output.extend(std::iter::repeat_n(
                b'\\' as u16,
                slashes * if unit == b'"' as u16 { 2 } else { 1 },
            ));
            if unit == b'"' as u16 {
                output.push(b'\\' as u16);
            }
            output.push(unit);
            slashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
}
fn command_line(application: &[u16], target: &[u16]) -> Result<Vec<u16>, &'static str> {
    let mut result = Vec::new();
    quote(&application[..application.len() - 1], &mut result);
    result.push(b' ' as u16);
    quote(&target[..target.len() - 1], &mut result);
    result.push(0);
    if result.len() > MAX_WIDE {
        return Err("open_argument_size");
    }
    Ok(result)
}
fn desktop(path: &Path, target: &[u16]) -> Result<Receipt, &'static str> {
    if !path.is_absolute() {
        return Err("open_handler");
    }
    if !path.is_file() {
        return Err("open_handler_missing");
    }
    let application = wide(path.as_os_str(), "open_handler")?;
    let batch = path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
    let mut binary_type = 0;
    if batch
        || unsafe { GetBinaryTypeW(PCWSTR(application.as_ptr()), &mut binary_type) }.is_err()
        || !matches!(binary_type, SCS_32BIT_BINARY | SCS_64BIT_BINARY)
    {
        return Err("open_handler_executable");
    }
    let mut arguments = command_line(&application, target)?;
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // Explicit application name prevents Windows from guessing a program from the command line.
    // Validate a native PE first: this Windows build accepts .cmd in CreateProcessW too.
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(arguments.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &startup,
            &mut process,
        )
    }
    .map_err(|_| "open_desktop_launch")?;
    unsafe {
        let _ = CloseHandle(process.hThread);
        let _ = CloseHandle(process.hProcess);
    }
    Ok(Receipt {
        route: Route::Desktop,
        process_id: Some(process.dwProcessId),
    })
}
fn system(target: &[u16]) -> Result<Receipt, &'static str> {
    let _apartment = Apartment::enter()?;
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_UNICODE,
        lpVerb: w!("open"),
        lpFile: PCWSTR(target.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let accepted = unsafe { ShellExecuteExW(&mut info) };
    let process_id = if !info.hProcess.is_invalid() {
        let id = unsafe { GetProcessId(info.hProcess) };
        unsafe {
            let _ = CloseHandle(info.hProcess);
        }
        (id != 0).then_some(id)
    } else {
        None
    };
    accepted.map_err(|_| "open_system_launch")?;
    Ok(Receipt {
        route: Route::System,
        process_id,
    })
}
fn packaged(identity: &str, target: &[u16], is_url: bool) -> Result<Receipt, &'static str> {
    if !valid_identity(identity) {
        return Err("open_handler");
    }
    let app = wide(OsStr::new(identity), "open_handler")?;
    let _apartment = Apartment::enter()?;
    unsafe {
        let manager: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                .map_err(|_| "open_activation_manager")?;
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(target.as_ptr()), None)
            .map_err(|_| "open_shell_item")?;
        let items: IShellItemArray =
            SHCreateShellItemArrayFromShellItem(&item).map_err(|_| "open_shell_item")?;
        let id = if is_url {
            manager.ActivateForProtocol(PCWSTR(app.as_ptr()), &items)
        } else {
            manager.ActivateForFile(PCWSTR(app.as_ptr()), &items, w!("open"))
        }
        .map_err(|_| "open_packaged_launch")?;
        Ok(Receipt {
            route: Route::Packaged,
            process_id: (id != 0).then_some(id),
        })
    }
}
/// No automatic fallback: return the actual selected-handler error for the core/UI to handle.
/// Call on the core operation worker, never a WinUI dispatcher or while holding history locks.
pub fn launch(handler: Handler<'_>, target: Target<'_>) -> Result<Receipt, &'static str> {
    let value = target.value()?;
    let units = wide(&value, "open_target")?;
    match handler {
        Handler::System => system(&units),
        Handler::Desktop(path) => desktop(path, &units),
        Handler::Packaged(identity) => packaged(identity, &units, matches!(target, Target::Url(_))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::System::Com::{
        CoGetApartmentType, APTTYPE, APTTYPEQUALIFIER, APTTYPE_MTA, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Shell::CommandLineToArgvW;
    #[test]
    fn command_line_roundtrips_with_windows_argument_parser() {
        let application = wide(OsStr::new(r"C:\Program Files\编辑器\app.exe"), "fixture").unwrap();
        for arg in [
            r"C:\路径 %20 ' & one.txt",
            r"C:\ends with slash\",
            "https://example.test/?q=\"x\"&v=%20",
            "  中文\r\n\t  ",
            r#"quotes\"still"#,
            "",
        ] {
            // Empty arguments are supported by quoting although a launch target itself cannot be empty.
            let units: Vec<_> = OsStr::new(arg).encode_wide().chain([0]).collect();
            let command = command_line(&application, &units).unwrap();
            let mut count = 0;
            let args = unsafe { CommandLineToArgvW(PCWSTR(command.as_ptr()), &mut count) };
            assert!(!args.is_null());
            assert_eq!(count, 2);
            let observed = unsafe { (*args.add(1)).to_string().unwrap() };
            unsafe {
                let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(args.cast())));
            }
            assert_eq!(observed, arg);
        }
        let surrogate = OsString::from_wide(&[0xD800, b'\\' as u16]);
        let units = wide(&surrogate, "fixture").unwrap();
        let command = command_line(&application, &units).unwrap();
        let mut count = 0;
        let args = unsafe { CommandLineToArgvW(PCWSTR(command.as_ptr()), &mut count) };
        assert_eq!(count, 2);
        let observed = unsafe { (*args.add(1)).as_wide() };
        assert_eq!(observed, &units[..units.len() - 1]);
        unsafe {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(args.cast())));
        }
    }
    #[test]
    fn invalid_targets_handlers_and_oversize_never_reach_launch() {
        for url in [
            "",
            "file:///C:/Windows/cmd.exe",
            "javascript:alert(1)",
            "https://",
            "mailto:",
            "https://example.test/\0x",
            "https://example.test/\n&ver",
        ] {
            assert!(launch(Handler::System, Target::Url(url)).is_err());
        }
        assert_eq!(
            launch(Handler::System, Target::Path(Path::new("relative.txt"))).unwrap_err(),
            "open_target"
        );
        assert_eq!(
            launch(
                Handler::System,
                Target::Path(Path::new(r"C:\TieZ-not-present\missing.txt"))
            )
            .unwrap_err(),
            "open_target_missing"
        );
        let target = Target::Url("https://example.test/?x=1&ver");
        assert_eq!(
            launch(Handler::Desktop(Path::new("app.exe")), target).unwrap_err(),
            "open_handler"
        );
        for id in [
            "Family!App!Other",
            "!App",
            "Family!",
            "Family!App';calc",
            "Family!App\n",
        ] {
            assert_eq!(
                launch(Handler::Packaged(id), target).unwrap_err(),
                "open_handler"
            );
        }
        assert!(wide(OsStr::new("bad\0target"), "invalid").is_err());
        assert!(wide(OsStr::new(&"x".repeat(MAX_WIDE)), "invalid").is_err());
        let application = wide(OsStr::new(&"x".repeat(17000)), "fixture").unwrap();
        assert_eq!(
            command_line(&application, &application).unwrap_err(),
            "open_argument_size"
        );
    }
    #[test]
    fn com_scope_balances_new_sta_and_preserves_existing_mta() {
        std::thread::spawn(|| unsafe {
            let mut kind = APTTYPE::default();
            let mut qualifier = APTTYPEQUALIFIER::default();
            assert!(CoGetApartmentType(&mut kind, &mut qualifier).is_err());
            {
                let _first = Apartment::enter().unwrap();
                let _second = Apartment::enter().unwrap();
                CoGetApartmentType(&mut kind, &mut qualifier).unwrap();
            }
            assert!(CoGetApartmentType(&mut kind, &mut qualifier).is_err());
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
            {
                let _scope = Apartment::enter().unwrap();
            }
            CoGetApartmentType(&mut kind, &mut qualifier).unwrap();
            assert_eq!(kind, APTTYPE_MTA);
            CoUninitialize();
            assert!(CoGetApartmentType(&mut kind, &mut qualifier).is_err());
        })
        .join()
        .unwrap();
    }
}
