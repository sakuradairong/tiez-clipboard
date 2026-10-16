//! One owned native tray/message thread. Actions are bounded/coalesced host requests.
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WAIT_FAILED, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const NOTIFY: u32 = WM_APP + 41;
// Windows header macro; not emitted by windows-rs metadata.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const SHOW: u8 = 1;
const SETTINGS: u8 = 2;
const QUIT: u8 = 4;
static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellAction {
    ShowMain,
    OpenSettings,
    Quit,
}
#[derive(Default)]
struct State {
    pending: u8,
    tray_visible: bool,
    last_error: Option<&'static str>,
}
impl State {
    fn queue(&mut self, action: u8) {
        self.pending |= action;
    }
    fn drain(&mut self) -> Vec<ShellAction> {
        let flags = std::mem::take(&mut self.pending);
        if flags & QUIT != 0 {
            return vec![ShellAction::Quit];
        }
        let mut result = Vec::new();
        if flags & SHOW != 0 {
            result.push(ShellAction::ShowMain);
        }
        if flags & SETTINGS != 0 {
            result.push(ShellAction::OpenSettings);
        }
        result
    }
}
struct Context {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    taskbar_created: u32,
    desired_tray: bool,
    icon: HICON,
}
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
struct ResetContext;
impl Drop for ResetContext {
    fn drop(&mut self) {
        CONTEXT.with(|slot| *slot.borrow_mut() = None);
    }
}
#[derive(Clone, Debug)]
pub struct ShellStatus {
    pub running: bool,
    pub starts: u64,
    pub tray_visible: bool,
    pub last_error: Option<&'static str>,
}
#[derive(Default)]
pub struct ShellService {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    thread: Option<JoinHandle<Result<(), &'static str>>>,
    starts: u64,
    desired_tray: bool,
}
impl ShellService {
    pub fn status(&self) -> ShellStatus {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let running = self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished());
        ShellStatus {
            running,
            starts: self.starts,
            tray_visible: running && state.tray_visible,
            last_error: state.last_error,
        }
    }
    pub fn start(&mut self, show_tray: bool) -> Result<ShellStatus, &'static str> {
        if self.status().running {
            if self.desired_tray != show_tray {
                return Err("shell_restart_required");
            }
            return Ok(self.status());
        }
        self.stop()?;
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = State::default();
        self.stop = Arc::new(AtomicBool::new(false));
        self.desired_tray = show_tray;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (state, stop, window) = (self.state.clone(), self.stop.clone(), self.window.clone());
        self.thread = Some(
            thread::Builder::new()
                .name("TieZ-native-shell".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(state.clone(), stop, window.clone(), show_tray, &ready_tx)
                    }))
                    .unwrap_or(Err("shell_worker_panicked"));
                    window.store(0, Ordering::Release);
                    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                    state.tray_visible = false;
                    if let Err(code) = result {
                        state.last_error = Some(code);
                        let _ = ready_tx.try_send(Err(code));
                    }
                    result
                })
                .map_err(|_| "shell_thread_start")?,
        );
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.starts = self.starts.saturating_add(1);
                Ok(self.status())
            }
            result => {
                let code = match result {
                    Ok(Err(code)) => code,
                    _ => "shell_start_disconnected",
                };
                let _ = self.stop();
                Err(code)
            }
        }
    }
    pub fn drain(&self) -> Vec<ShellAction> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).drain()
    }
    pub fn stop(&mut self) -> Result<(), &'static str> {
        self.stop.store(true, Ordering::Release);
        let window = self.window.load(Ordering::Acquire);
        if window != 0 {
            let _ =
                unsafe { PostMessageW(Some(HWND(window as _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        if let Some(thread) = self.thread.take() {
            let result = thread.join().unwrap_or(Err("shell_worker_panicked"));
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.pending = 0;
            state.tray_visible = false;
            result?;
        }
        Ok(())
    }
}
impl Drop for ShellService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
struct Resources {
    name: Vec<u16>,
    instance: HINSTANCE,
    window: HWND,
    icon: HICON,
    owned_icon: bool,
}
impl Drop for Resources {
    fn drop(&mut self) {
        unsafe {
            if !self.window.0.is_null() {
                let data = icon_data(self.window, self.icon);
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
                let _ = DestroyWindow(self.window);
            }
            let _ = UnregisterClassW(PCWSTR(self.name.as_ptr()), Some(self.instance));
            if self.owned_icon {
                let _ = DestroyIcon(self.icon);
            }
        }
    }
}
fn icon_data(window: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: NOTIFY,
        hIcon: icon,
        ..Default::default()
    };
    for (to, from) in data.szTip.iter_mut().zip("TieZ 原生迁移版".encode_utf16()) {
        *to = from;
    }
    data
}
fn add_tray(window: HWND, icon: HICON) -> Result<(), &'static str> {
    let mut data = icon_data(window, icon);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            return Err("shell_tray_add");
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        if !Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool() {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            return Err("shell_tray_version");
        }
    }
    Ok(())
}
fn load_icon() -> Result<(HICON, bool), &'static str> {
    let mut path = vec![0u16; 32768];
    let count = unsafe { GetModuleFileNameW(None, &mut path) } as usize;
    if count != 0 && count < path.len() {
        let mut icon = HICON::default();
        if unsafe { ExtractIconExW(PCWSTR(path.as_ptr()), 0, None, Some(&mut icon), 1) } == 1
            && !icon.0.is_null()
        {
            return Ok((icon, true));
        }
    }
    unsafe { LoadIconW(None, IDI_APPLICATION) }
        .map(|icon| (icon, false))
        .map_err(|_| "shell_icon")
}
fn run(
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    shared_window: Arc<AtomicIsize>,
    desired_tray: bool,
    ready: &mpsc::SyncSender<Result<(), &'static str>>,
) -> Result<(), &'static str> {
    let name: Vec<u16> = format!(
        "TieZNativeShell-{}-{}",
        std::process::id(),
        NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let instance = HINSTANCE(
        unsafe { GetModuleHandleW(None) }
            .map_err(|_| "shell_module")?
            .0,
    );
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: PCWSTR(name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err("shell_register_class");
    }
    let mut resources = Resources {
        name,
        instance,
        window: HWND::default(),
        icon: HICON::default(),
        owned_icon: false,
    };
    let (icon, owned) = load_icon()?;
    resources.icon = icon;
    resources.owned_icon = owned;
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    if taskbar_created == 0 {
        return Err("shell_taskbar_message");
    }
    CONTEXT.with(|slot| {
        *slot.borrow_mut() = Some(Context {
            state: state.clone(),
            stop: stop.clone(),
            taskbar_created,
            desired_tray,
            icon,
        })
    });
    let _reset = ResetContext;
    // A hidden top-level tool window receives Explorer's TaskbarCreated broadcast.
    resources.window = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(resources.name.as_ptr()),
            w!("TieZ native shell"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|_| "shell_create_window")?;
    shared_window.store(resources.window.0 as isize, Ordering::Release);
    if desired_tray {
        add_tray(resources.window, icon)?;
        state.lock().unwrap_or_else(|e| e.into_inner()).tray_visible = true;
    }
    ready.send(Ok(())).map_err(|_| "shell_owner_left")?;
    while !stop.load(Ordering::Acquire) {
        if unsafe { MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT) } == WAIT_FAILED {
            return Err("shell_message_wait");
        }
        let mut message = MSG::default();
        while !stop.load(Ordering::Acquire)
            && unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool()
        {
            if message.message == WM_QUIT {
                return Ok(());
            }
            unsafe {
                DispatchMessageW(&message);
            }
        }
    }
    Ok(())
}
struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}
fn show_menu(window: HWND) -> Result<u8, &'static str> {
    let menu = Menu(unsafe { CreatePopupMenu() }.map_err(|_| "shell_menu_create")?);
    unsafe {
        AppendMenuW(menu.0, MF_STRING, 1, w!("显示主界面")).map_err(|_| "shell_menu_item")?;
        AppendMenuW(menu.0, MF_STRING, 2, w!("设置")).map_err(|_| "shell_menu_item")?;
        AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null()).map_err(|_| "shell_menu_item")?;
        AppendMenuW(menu.0, MF_STRING, 3, w!("退出 TieZ")).map_err(|_| "shell_menu_item")?;
        let mut point = POINT::default();
        GetCursorPos(&mut point).map_err(|_| "shell_menu_position")?;
        let previous = GetForegroundWindow();
        let mut previous_pid = 0;
        GetWindowThreadProcessId(previous, Some(&mut previous_pid));
        let _ = SetForegroundWindow(window);
        let command = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            window,
            None,
        )
        .0;
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        if command == 0 && GetForegroundWindow() == window {
            let mut actual_pid = 0;
            GetWindowThreadProcessId(previous, Some(&mut actual_pid));
            if previous_pid != 0 && previous_pid == actual_pid && IsWindow(Some(previous)).as_bool()
            {
                let _ = SetForegroundWindow(previous);
            }
        }
        Ok(match command {
            1 => SHOW,
            2 => SETTINGS,
            3 => QUIT,
            _ => 0,
        })
    }
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let handled = std::panic::catch_unwind(|| {
        CONTEXT.with(|slot| {
            let context = slot.borrow();
            let Some(context) = context.as_ref() else {
                return false;
            };
            if message == WM_CLOSE {
                context.stop.store(true, Ordering::Release);
                let _ = EndMenu();
                return true;
            }
            if message == context.taskbar_created && context.desired_tray {
                let result = add_tray(window, context.icon);
                let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
                state.tray_visible = result.is_ok();
                state.last_error = result.err();
                return true;
            }
            if message == NOTIFY {
                let event = lparam.0 as u32 & 0xffff;
                let action = match event {
                    NIN_SELECT | NIN_KEYSELECT => Ok(SHOW),
                    WM_CONTEXTMENU => show_menu(window),
                    _ => Ok(0),
                };
                let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
                match action {
                    Ok(action) => state.queue(action),
                    Err(code) => state.last_error = Some(code),
                }
                return true;
            }
            false
        })
    });
    match handled {
        Ok(true) => LRESULT(0),
        Ok(false) => DefWindowProcW(window, message, wparam, lparam),
        Err(_) => {
            CONTEXT.with(|slot| {
                if let Some(context) = slot.borrow().as_ref() {
                    context
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .last_error = Some("shell_callback_panicked");
                    context.stop.store(true, Ordering::Release);
                }
            });
            LRESULT(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_are_bounded_coalesced_and_quit_has_priority() {
        let mut state = State::default();
        for _ in 0..1000 {
            state.queue(SHOW);
            state.queue(SETTINGS);
        }
        assert_eq!(
            state.drain(),
            vec![ShellAction::ShowMain, ShellAction::OpenSettings]
        );
        assert!(state.drain().is_empty());
        state.queue(SHOW | SETTINGS | QUIT);
        assert_eq!(state.drain(), vec![ShellAction::Quit]);
    }
    #[test]
    fn real_window_routes_release_keyboard_select_and_ignores_mouse_down() {
        let mut shell = ShellService::default();
        assert!(shell.start(false).unwrap().running);
        let window = HWND(shell.window.load(Ordering::Acquire) as _);
        for event in [
            WM_LBUTTONDOWN,
            WM_RBUTTONDOWN,
            NIN_SELECT,
            NIN_SELECT,
            NIN_KEYSELECT,
        ] {
            unsafe {
                SendMessageW(
                    window,
                    NOTIFY,
                    Some(WPARAM(0)),
                    Some(LPARAM(event as isize)),
                );
            }
        }
        assert_eq!(shell.drain(), vec![ShellAction::ShowMain]);
        assert!(shell.drain().is_empty());
        shell.stop().unwrap();
        assert!(!unsafe { IsWindow(Some(window)) }.as_bool());
    }
    #[test]
    fn stop_restart_and_drop_join_and_release_window_state() {
        let mut shell = ShellService::default();
        assert_eq!(shell.start(false).unwrap().starts, 1);
        assert_eq!(shell.start(false).unwrap().starts, 1);
        assert_eq!(shell.start(true).unwrap_err(), "shell_restart_required");
        shell.stop().unwrap();
        shell.stop().unwrap();
        assert!(!shell.status().running);
        assert_eq!(shell.start(false).unwrap().starts, 2);
        let window = HWND(shell.window.load(Ordering::Acquire) as _);
        let state = shell.state.clone();
        drop(shell);
        assert!(!unsafe { IsWindow(Some(window)) }.as_bool());
        assert_eq!(Arc::strong_count(&state), 1);
    }
}
