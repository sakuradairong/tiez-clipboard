//! One owned native tray/message thread. Actions are bounded/coalesced host requests.
use crate::hotkey::{Bindings, Shortcut, WindowsBackend};
use std::cell::RefCell;
use std::collections::VecDeque;
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
const HOTKEY_COMMAND: u32 = WM_APP + 42;
// Windows header macro; not emitted by windows-rs metadata.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const SHOW: u8 = 1;
const SETTINGS: u8 = 2;
const QUIT: u8 = 4;
const TOGGLE: u8 = 8;
static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellAction {
    ShowMain,
    OpenSettings,
    Quit,
    ToggleMain,
}
#[derive(Default)]
struct State {
    pending: VecDeque<ShellAction>,
    tray_visible: bool,
    last_error: Option<&'static str>,
    hotkey_active: bool,
    hotkey_error: Option<&'static str>,
}
impl State {
    fn queue(&mut self, flags: u8) {
        if flags & QUIT != 0 {
            self.pending.clear();
            self.pending.push_back(ShellAction::Quit);
            return;
        }
        if self.pending.front() == Some(&ShellAction::Quit) {
            return;
        }
        for (flag, action) in [
            (SHOW, ShellAction::ShowMain),
            (SETTINGS, ShellAction::OpenSettings),
            (TOGGLE, ShellAction::ToggleMain),
        ] {
            if flag & flags == 0 {
                continue;
            }
            if action != ShellAction::ToggleMain && self.pending.back() == Some(&action) {
                continue;
            }
            if self.pending.len() == 32 {
                self.pending.pop_front();
                self.last_error = Some("shell_action_overflow");
            }
            self.pending.push_back(action);
        }
    }
    fn drain(&mut self) -> Vec<ShellAction> {
        self.pending.drain(..).collect()
    }
}

struct Context {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    taskbar_created: u32,
    desired_tray: bool,
    icon: HICON,
    commands: RefCell<mpsc::Receiver<Command>>,
    hotkeys: RefCell<Option<Bindings<WindowsBackend>>>,
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
    pub main_hotkey_active: bool,
    pub hotkey_error: Option<&'static str>,
}
#[derive(Default)]
pub struct ShellService {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    thread: Option<JoinHandle<Result<(), &'static str>>>,
    starts: u64,
    desired_tray: bool,
    endpoint: Option<Arc<Endpoint>>,
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
            main_hotkey_active: running && state.hotkey_active,
            hotkey_error: state.hotkey_error,
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
        let (command_tx, command_rx) = mpsc::sync_channel(8);
        self.endpoint = Some(Arc::new(Endpoint {
            sender: command_tx,
            window: self.window.clone(),
        }));
        let (state, stop, window) = (self.state.clone(), self.stop.clone(), self.window.clone());
        self.thread = Some(
            thread::Builder::new()
                .name("TieZ-native-shell".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(
                            state.clone(),
                            stop,
                            window.clone(),
                            show_tray,
                            command_rx,
                            &ready_tx,
                        )
                    }))
                    .unwrap_or(Err("shell_worker_panicked"));
                    window.store(0, Ordering::Release);
                    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                    state.tray_visible = false;
                    state.hotkey_active = false;
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
    pub fn prepare_hotkey(&self, value: &str) -> Result<PreparedHotkey, &'static str> {
        let shortcut = Shortcut::parse(value)?;
        if !self.status().running {
            return Err("shell_stopped");
        }
        let endpoint = self.endpoint.as_ref().ok_or("shell_stopped")?.clone();
        let token = endpoint.call(CommandKind::Prepare(shortcut))?;
        Ok(PreparedHotkey {
            endpoint,
            token,
            finished: false,
        })
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
            state.pending.clear();
            state.tray_visible = false;
            state.hotkey_active = false;
            self.endpoint = None;
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
    commands: mpsc::Receiver<Command>,
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
            commands: RefCell::new(commands),
            hotkeys: RefCell::new(None),
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
    CONTEXT.with(|slot| {
        if let Some(context) = slot.borrow().as_ref() {
            *context.hotkeys.borrow_mut() = Some(Bindings::new(WindowsBackend(resources.window)));
        }
    });
    shared_window.store(resources.window.0 as isize, Ordering::Release);
    if desired_tray {
        add_tray(resources.window, icon)?;
        state.lock().unwrap_or_else(|e| e.into_inner()).tray_visible = true;
    }
    ready.send(Ok(())).map_err(|_| "shell_owner_left")?;
    while !stop.load(Ordering::Acquire) {
        // Also drain on the periodic wake, so a failed wake-up message cannot strand a request.
        CONTEXT.with(|slot| {
            if let Some(context) = slot.borrow().as_ref() {
                handle_commands(context);
            }
        });
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
            if message == HOTKEY_COMMAND {
                handle_commands(context);
                return true;
            }
            if message == WM_HOTKEY {
                let accepted = context.hotkeys.try_borrow().ok().is_some_and(|hotkeys| {
                    hotkeys
                        .as_ref()
                        .is_some_and(|bindings| bindings.accepts(wparam.0, lparam.0))
                });
                if accepted {
                    context
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .queue(TOGGLE);
                }
                return true;
            }
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
        }
        state.queue(SETTINGS);
        assert_eq!(
            state.drain(),
            vec![ShellAction::ShowMain, ShellAction::OpenSettings]
        );
        assert!(state.drain().is_empty());
        state.queue(SHOW | SETTINGS | QUIT);
        assert_eq!(state.drain(), vec![ShellAction::Quit]);
    }
    #[test]
    fn stateful_toggle_actions_keep_order_and_count_within_the_bound() {
        let mut state = State::default();
        state.queue(SHOW);
        state.queue(TOGGLE);
        state.queue(TOGGLE);
        state.queue(SHOW);
        assert_eq!(
            state.drain(),
            vec![
                ShellAction::ShowMain,
                ShellAction::ToggleMain,
                ShellAction::ToggleMain,
                ShellAction::ShowMain
            ]
        );
        for _ in 0..40 {
            state.queue(TOGGLE);
        }
        assert_eq!(state.drain().len(), 32);
        assert_eq!(state.last_error, Some("shell_action_overflow"));
        state.queue(QUIT);
        state.queue(SHOW);
        state.queue(TOGGLE);
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
    fn provisional_hotkey_drop_and_shutdown_release_real_registration() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
        };
        let probe = || {
            thread::spawn(|| unsafe {
                RegisterHotKey(None, 1, HOT_KEY_MODIFIERS(7) | MOD_NOREPEAT, 0x87).unwrap();
                UnregisterHotKey(None, 1).unwrap();
            })
            .join()
            .unwrap()
        };
        let mut shell = ShellService::default();
        shell.start(false).unwrap();
        let staged = shell.prepare_hotkey("Ctrl+Alt+Shift+F24").unwrap();
        assert!(!shell.status().main_hotkey_active);
        drop(staged);
        probe();
        shell
            .prepare_hotkey("Ctrl+Alt+Shift+F24")
            .unwrap()
            .commit()
            .unwrap();
        assert!(shell.status().main_hotkey_active);
        let window = HWND(shell.window.load(Ordering::Acquire) as _);
        unsafe {
            SendMessageW(
                window,
                WM_HOTKEY,
                Some(WPARAM(0x4100)),
                Some(LPARAM((0x87 << 16) | 7)),
            );
        }
        assert!(
            shell.drain().is_empty(),
            "Rolled-back registration dispatched a stale message"
        );
        unsafe {
            SendMessageW(
                window,
                WM_HOTKEY,
                Some(WPARAM(0x4101)),
                Some(LPARAM((0x87 << 16) | 7)),
            );
        }
        assert_eq!(shell.drain(), vec![ShellAction::ToggleMain]);
        shell.stop().unwrap();
        probe();
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

enum CommandKind {
    Prepare(Shortcut),
    Resolve { token: u64, commit: bool },
}
struct Command {
    kind: CommandKind,
    reply: mpsc::SyncSender<Result<u64, &'static str>>,
}
struct Endpoint {
    sender: mpsc::SyncSender<Command>,
    window: Arc<AtomicIsize>,
}
impl Endpoint {
    fn call(&self, kind: CommandKind) -> Result<u64, &'static str> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.sender
            .send(Command { kind, reply })
            .map_err(|_| "shell_stopped")?;
        let window = HWND(self.window.load(Ordering::Acquire) as _);
        let _ = unsafe { PostMessageW(Some(window), HOTKEY_COMMAND, WPARAM(0), LPARAM(0)) };
        // The owner performs only in-process registration calls. No UI or core callback.
        receive.recv().map_err(|_| "shell_stopped")?
    }
}
pub struct PreparedHotkey {
    endpoint: Arc<Endpoint>,
    token: u64,
    finished: bool,
}
impl PreparedHotkey {
    pub fn commit(mut self) -> Result<(), &'static str> {
        // The DB has committed; never silently restore old configuration on an ack failure.
        self.finished = true;
        self.endpoint
            .call(CommandKind::Resolve {
                token: self.token,
                commit: true,
            })
            .map(|_| ())
    }
}
impl Drop for PreparedHotkey {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.endpoint.call(CommandKind::Resolve {
                token: self.token,
                commit: false,
            });
        }
    }
}
fn handle_commands(context: &Context) {
    let commands: Vec<_> = context.commands.borrow_mut().try_iter().take(8).collect();
    for command in commands {
        let Ok(mut hotkeys) = context.hotkeys.try_borrow_mut() else {
            let _ = command.reply.send(Err("hotkey_update_pending"));
            continue;
        };
        let Some(bindings) = hotkeys.as_mut() else {
            let _ = command.reply.send(Err("shell_stopped"));
            continue;
        };
        let preparing = matches!(&command.kind, CommandKind::Prepare(_));
        let result = match command.kind {
            CommandKind::Prepare(shortcut) => bindings.prepare(shortcut),
            CommandKind::Resolve { token, commit } => {
                let result = bindings.resolve(token, commit).map(|_| 0);
                if commit {
                    context
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pending
                        .retain(|action| *action != ShellAction::ToggleMain);
                }
                result
            }
        };
        // If the preparer's response receiver disappeared, roll back its provisional registration.
        let prepared_token = if preparing {
            result.as_ref().ok().copied()
        } else {
            None
        };
        {
            let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
            state.hotkey_active = bindings.active();
            state.hotkey_error = bindings.error;
        }
        if command.reply.send(result).is_err() {
            if let Some(token) = prepared_token {
                let _ = bindings.resolve(token, false);
                let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
                state.hotkey_active = bindings.active();
                state.hotkey_error = bindings.error;
            }
        }
    }
}
