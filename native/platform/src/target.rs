//! Owns a foreground-event hook. No clipboard access, key injection or window titles.
use crate::paste_rules::{is_system_focus_class, matches_target_foreground};
use std::cell::RefCell;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, WAIT_FAILED};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    window: usize,
    process: u32,
    thread: u32,
    created: u64,
    application: String,
}
impl Identity {
    fn read(window: HWND, exclude_process: u32) -> Result<Self, &'static str> {
        if window.0.is_null()
            || !unsafe { IsWindow(Some(window)) }.as_bool()
            || !unsafe { IsWindowVisible(window) }.as_bool()
        {
            return Err("target_unavailable");
        }
        let mut process = 0;
        let thread = unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
        if process == 0 || process == exclude_process || thread == 0 {
            return Err("target_own_or_missing");
        }
        let mut class = [0u16; 256];
        let length = unsafe { GetClassNameW(window, &mut class) };
        if length == 0
            || is_system_focus_class(&String::from_utf16_lossy(&class[..length as usize]))
        {
            return Err("target_system_surface");
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }
            .map_err(|_| "target_process_unavailable")?;
        let result = (|| {
            let (mut created, mut exit, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            unsafe { GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) }
                .map_err(|_| "target_process_identity")?;
            let mut path = [0u16; 32768];
            let mut size = path.len() as u32;
            unsafe {
                QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_FORMAT(0),
                    PWSTR(path.as_mut_ptr()),
                    &mut size,
                )
            }
            .map_err(|_| "target_process_name")?;
            let application = Path::new(&String::from_utf16_lossy(&path[..size as usize]))
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if application.is_empty() {
                return Err("target_process_name");
            }
            Ok(Self {
                window: window.0 as usize,
                process,
                thread,
                created: (u64::from(created.dwHighDateTime) << 32)
                    | u64::from(created.dwLowDateTime),
                application,
            })
        })();
        let _ = unsafe { CloseHandle(handle) };
        result
    }
    fn validate(&self) -> Result<(), &'static str> {
        if Self::read(HWND(self.window as _), unsafe { GetCurrentProcessId() })? != *self {
            return Err("target_identity_changed");
        }
        Ok(())
    }
}
#[derive(Default)]
struct State {
    target: Option<Identity>,
    generation: u64,
    last_error: Option<&'static str>,
}
impl State {
    fn advance(&mut self) -> Result<(), &'static str> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("target_generation_exhausted")?;
        Ok(())
    }
    fn invalidate_window(&mut self, window: usize) -> Result<(), &'static str> {
        if self
            .target
            .as_ref()
            .is_some_and(|target| target.window == window)
        {
            // Clear first, including when the generation counter is exhausted.
            self.target = None;
            self.advance()?;
        }
        Ok(())
    }
    fn record(&mut self, target: Identity) -> Result<(), &'static str> {
        if self.target.as_ref() != Some(&target) {
            self.advance()?;
            self.target = Some(target);
        }
        Ok(())
    }
}
struct Context {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    own_process: u32,
}
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
struct ResetContext;
impl Drop for ResetContext {
    fn drop(&mut self) {
        CONTEXT.with(|context| *context.borrow_mut() = None);
    }
}
struct Hook(HWINEVENTHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            let _ = unsafe { UnhookWinEvent(self.0) };
        }
    }
}
unsafe extern "system" fn event_callback(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    let result = std::panic::catch_unwind(|| {
        if event == EVENT_SYSTEM_FOREGROUND {
            observe(window);
        } else if event == EVENT_OBJECT_DESTROY && object == 0 && child == 0 {
            // OBJID_WINDOW / CHILDID_SELF. Child controls must not retire a top-level target.
            CONTEXT.with(|context| {
                if let Some(context) = context.borrow().as_ref() {
                    let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Err(code) = state.invalidate_window(window.0 as usize) {
                        state.last_error = Some(code);
                        context.stop.store(true, Ordering::Release);
                    }
                }
            });
        }
    });
    if result.is_err() {
        CONTEXT.with(|context| {
            if let Some(context) = context.borrow().as_ref() {
                context
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last_error = Some("target_callback_panicked");
                context.stop.store(true, Ordering::Release);
            }
        });
    }
}
fn observe(window: HWND) {
    CONTEXT.with(|context| {
        if let Some(context) = context.borrow().as_ref() {
            let observed = Identity::read(window, context.own_process);
            let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
            let update = match observed {
                Ok(target) => state.record(target),
                // Own windows and shell overlays intentionally preserve the last input target.
                Err("target_own_or_missing" | "target_system_surface") => Ok(()),
                // An external target we cannot inspect must not leave an older app armed.
                Err(_) => {
                    if state.target.take().is_some() {
                        state.advance()
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(code) = update {
                state.last_error = Some(code);
                context.stop.store(true, Ordering::Release);
            }
        }
    });
}

#[derive(Clone, Debug)]
pub struct TargetStatus {
    pub running: bool,
    pub starts: u64,
    pub generation: u64,
    pub has_target: bool,
    pub application: Option<String>,
    pub last_error: Option<&'static str>,
}
#[derive(Clone)]
pub struct TargetTicket {
    identity: Identity,
    generation: u64,
}
pub struct FocusResult {
    pub generation: u64,
    pub restored: bool,
}
#[derive(Default)]
pub struct TargetTracker {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), &'static str>>>,
    starts: u64,
}
impl TargetTracker {
    pub fn status(&self) -> TargetStatus {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let running = self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished());
        let valid = state
            .target
            .as_ref()
            .filter(|target| running && target.validate().is_ok());
        TargetStatus {
            running,
            starts: self.starts,
            generation: state.generation,
            has_target: valid.is_some(),
            application: valid.map(|target| target.application.clone()),
            last_error: state.last_error,
        }
    }
    pub fn start(&mut self) -> Result<TargetStatus, &'static str> {
        if self.status().running {
            return Ok(self.status());
        }
        self.stop()?;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.advance()?;
            state.target = None;
            state.last_error = None;
        }
        self.stop = Arc::new(AtomicBool::new(false));
        let state = self.state.clone();
        let stop = self.stop.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        self.thread = Some(
            thread::Builder::new()
                .name("TieZ-native-target".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(state.clone(), stop, &ready_tx)
                    }))
                    .unwrap_or(Err("target_worker_panicked"));
                    if let Err(code) = result {
                        state.lock().unwrap_or_else(|e| e.into_inner()).last_error = Some(code);
                        let _ = ready_tx.try_send(Err(code));
                    }
                    result
                })
                .map_err(|_| "target_thread_start")?,
        );
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.starts = self.starts.saturating_add(1);
                Ok(self.status())
            }
            result => {
                let code = match result {
                    Ok(Err(code)) => code,
                    _ => "target_start_disconnected",
                };
                let _ = self.stop();
                Err(code)
            }
        }
    }
    pub fn stop(&mut self) -> Result<(), &'static str> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let result = thread.join().unwrap_or(Err("target_worker_panicked"));
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.target = None;
            state.advance()?;
            if let Err(code) = result {
                state.last_error = Some(code);
                return Err(code);
            }
        }
        Ok(())
    }
    pub fn ticket(&self, expected_generation: u64) -> Result<TargetTicket, &'static str> {
        if !self.status().running {
            return Err("target_tracker_stopped");
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation != expected_generation {
            return Err("target_changed");
        }
        let identity = state.target.clone().ok_or("target_missing")?;
        identity.validate()?;
        Ok(TargetTicket {
            identity,
            generation: expected_generation,
        })
    }
    fn validate_ticket(&self, ticket: &TargetTicket) -> Result<(), &'static str> {
        let actual = self.ticket(ticket.generation)?;
        if actual.identity != ticket.identity {
            return Err("target_identity_changed");
        }
        Ok(())
    }
    pub fn validate_foreground(&self, ticket: &TargetTicket) -> Result<(), &'static str> {
        self.validate_ticket(ticket)?;
        if !matches_target_foreground(
            ticket.identity.window,
            unsafe { GetForegroundWindow() }.0 as usize,
        ) {
            return Err("target_focus_changed");
        }
        Ok(())
    }
    pub fn restore(&self, ticket: &TargetTicket) -> Result<FocusResult, &'static str> {
        self.validate_ticket(ticket)?;
        let foreground = unsafe { GetForegroundWindow() };
        if matches_target_foreground(ticket.identity.window, foreground.0 as usize) {
            return Ok(FocusResult {
                generation: ticket.generation,
                restored: false,
            });
        }
        let mut process = 0;
        unsafe {
            GetWindowThreadProcessId(foreground, Some(&mut process));
        }
        if process == 0 || process != unsafe { GetCurrentProcessId() } {
            return Err("target_foreground_not_owned");
        }
        let window = HWND(ticket.identity.window as _);
        if unsafe { IsIconic(window) }.as_bool() {
            unsafe {
                let _ = ShowWindowAsync(window, SW_RESTORE);
            }
        }
        let accepted = unsafe { SetForegroundWindow(window) }.as_bool();
        if !accepted {
            return Err("target_focus_denied");
        }
        let deadline = Instant::now() + Duration::from_millis(300);
        loop {
            self.validate_ticket(ticket)?;
            if self.validate_foreground(ticket).is_ok() {
                return Ok(FocusResult {
                    generation: ticket.generation,
                    restored: true,
                });
            }
            if Instant::now() >= deadline {
                return Err("target_focus_denied");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for TargetTracker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn run(
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    ready: &mpsc::SyncSender<Result<(), &'static str>>,
) -> Result<(), &'static str> {
    CONTEXT.with(|context| {
        *context.borrow_mut() = Some(Context {
            state,
            stop: stop.clone(),
            own_process: unsafe { GetCurrentProcessId() },
        })
    });
    let _reset = ResetContext;
    let hook = Hook(unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(event_callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    });
    if hook.0 .0.is_null() {
        return Err("target_hook_start");
    }
    // A process may destroy/recreate HWNDs on the same thread. Process identity
    // alone cannot retire those tickets; observe top-level destruction as well.
    let _destroy_hook = Hook(unsafe {
        SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_DESTROY,
            None,
            Some(event_callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    });
    if _destroy_hook.0 .0.is_null() {
        return Err("target_destroy_hook_start");
    }
    observe(unsafe { GetForegroundWindow() });
    ready.send(Ok(())).map_err(|_| "target_owner_left")?;
    while !stop.load(Ordering::Acquire) {
        if unsafe { MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT) } == WAIT_FAILED {
            return Err("target_message_wait");
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
    if !unsafe { UnhookWinEvent(hook.0) }.as_bool() {
        return Err("target_hook_stop");
    }
    std::mem::forget(hook);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    #[test]
    fn generations_distinguish_reused_handles_and_duplicate_events() {
        let mut state = State::default();
        let a = Identity {
            window: 42,
            process: 10,
            thread: 11,
            created: 100,
            application: "fixture.exe".into(),
        };
        state.record(a.clone()).unwrap();
        let initial = state.generation;
        state.record(a.clone()).unwrap();
        assert_eq!(state.generation, initial);
        let mut reused = a.clone();
        reused.created += 1;
        state.record(reused).unwrap();
        assert_eq!(state.generation, initial + 1);
        state.record(a).unwrap();
        assert_eq!(state.generation, initial + 2);
    }
    #[test]
    fn destroyed_target_retires_ticket_before_same_process_handle_reuse() {
        let mut state = State::default();
        let target = Identity {
            window: 42,
            process: 10,
            thread: 11,
            created: 100,
            application: "fixture.exe".into(),
        };
        state.record(target.clone()).unwrap();
        let initial = state.generation;
        state.invalidate_window(43).unwrap();
        assert_eq!(state.generation, initial);
        state.invalidate_window(42).unwrap();
        assert!(state.target.is_none());
        assert_eq!(state.generation, initial + 1);
        state.record(target).unwrap();
        assert_eq!(state.generation, initial + 2);
        state.generation = u64::MAX;
        assert_eq!(
            state.invalidate_window(42),
            Err("target_generation_exhausted")
        );
        assert!(state.target.is_none());
    }
    #[test]
    fn destroy_callback_ignores_child_objects_and_unrelated_windows() {
        let state = Arc::new(Mutex::new(State::default()));
        state
            .lock()
            .unwrap()
            .record(Identity {
                window: 42,
                process: 10,
                thread: 11,
                created: 100,
                application: "fixture.exe".into(),
            })
            .unwrap();
        CONTEXT.with(|context| {
            *context.borrow_mut() = Some(Context {
                state: state.clone(),
                stop: Arc::new(AtomicBool::new(false)),
                own_process: 0,
            })
        });
        let _reset = ResetContext;
        unsafe {
            event_callback(
                HWINEVENTHOOK::default(),
                EVENT_OBJECT_DESTROY,
                HWND(42 as _),
                -4,
                0,
                0,
                0,
            );
            event_callback(
                HWINEVENTHOOK::default(),
                EVENT_OBJECT_DESTROY,
                HWND(42 as _),
                0,
                1,
                0,
                0,
            );
            event_callback(
                HWINEVENTHOOK::default(),
                EVENT_OBJECT_DESTROY,
                HWND(43 as _),
                0,
                0,
                0,
                0,
            );
        }
        assert!(state.lock().unwrap().target.is_some());
        unsafe {
            event_callback(
                HWINEVENTHOOK::default(),
                EVENT_OBJECT_DESTROY,
                HWND(42 as _),
                0,
                0,
                0,
                0,
            );
        }
        assert!(state.lock().unwrap().target.is_none());
    }
    #[test]
    fn unavailable_foreground_disarms_previous_target() {
        let state = Arc::new(Mutex::new(State::default()));
        state
            .lock()
            .unwrap()
            .record(Identity {
                window: 42,
                process: 10,
                thread: 11,
                created: 100,
                application: "fixture.exe".into(),
            })
            .unwrap();
        let generation = state.lock().unwrap().generation;
        CONTEXT.with(|context| {
            *context.borrow_mut() = Some(Context {
                state: state.clone(),
                stop: Arc::new(AtomicBool::new(false)),
                own_process: 0,
            })
        });
        let _reset = ResetContext;
        observe(HWND::default());
        let state = state.lock().unwrap();
        assert!(state.target.is_none());
        assert_eq!(state.generation, generation + 1);
    }
    #[test]
    fn generation_exhaustion_never_replaces_old_identity() {
        let mut state = State {
            generation: u64::MAX,
            ..Default::default()
        };
        assert_eq!(
            state.record(Identity {
                window: 42,
                process: 10,
                thread: 11,
                created: 100,
                application: "fixture.exe".into()
            }),
            Err("target_generation_exhausted")
        );
        assert!(state.target.is_none());
    }
    #[test]
    fn hook_start_stop_restart_and_drop_join_the_owner_thread() {
        let mut tracker = TargetTracker::default();
        assert!(!tracker.status().running);
        assert!(tracker.start().unwrap().running);
        assert_eq!(tracker.start().unwrap().starts, 1);
        tracker.stop().unwrap();
        assert!(!tracker.status().running);
        assert!(!tracker.status().has_target);
        tracker.stop().unwrap();
        assert_eq!(tracker.start().unwrap().starts, 2);
        let state = tracker.state.clone();
        drop(tracker);
        assert_eq!(
            Arc::strong_count(&state),
            1,
            "Hook thread or callback context retained state"
        );
        assert!(state.lock().unwrap().target.is_none());
    }
    #[test]
    fn stale_or_missing_tickets_are_rejected_without_focus_calls() {
        let tracker = TargetTracker::default();
        assert!(matches!(tracker.ticket(0), Err("target_tracker_stopped")));
        assert!(Identity::read(HWND::default(), unsafe { GetCurrentProcessId() }).is_err());
    }
    #[test]
    fn own_visible_window_is_not_a_paste_target() {
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("TieZ target rule fixture"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                120,
                80,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        unsafe {
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        }
        let result = Identity::read(window, unsafe { GetCurrentProcessId() });
        unsafe {
            DestroyWindow(window).unwrap();
        }
        assert!(matches!(result, Err("target_own_or_missing")));
    }
}
