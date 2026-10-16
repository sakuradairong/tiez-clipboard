use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WAIT_FAILED, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, GetClipboardSequenceNumber, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::*;

static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);

// Kernel event handles may be used concurrently by waiting/signalling threads. Arc
// keeps the handle alive until both the listener and its dispatch thread are gone.
struct WakeEvent(HANDLE);
unsafe impl Send for WakeEvent {}
unsafe impl Sync for WakeEvent {}
impl Drop for WakeEvent {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
struct StopState {
    requested: AtomicBool,
    event: WakeEvent,
}
#[derive(Default)]
struct SignalState {
    sequence: AtomicU32,
    notifications: AtomicU64,
}
struct WindowState {
    sender: SyncSender<()>,
    signal: Arc<SignalState>,
}

/// A coalesced notification hint, not a captured clipboard snapshot. Consumers must
/// recheck the OS sequence around their own format reads to reject torn snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardSignal {
    pub sequence: u32,
}

/// One bounded wake-up slot; bursts replace metadata rather than growing a queue.
/// Dropping this receiver does not block the native message procedure.
pub struct ClipboardEvents {
    receiver: Receiver<()>,
    signal: Arc<SignalState>,
}
impl ClipboardEvents {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ClipboardSignal, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)?;
        Ok(ClipboardSignal {
            sequence: self.signal.sequence.load(Ordering::Acquire),
        })
    }
    pub fn try_recv(&self) -> Result<ClipboardSignal, TryRecvError> {
        self.receiver.try_recv()?;
        Ok(ClipboardSignal {
            sequence: self.signal.sequence.load(Ordering::Acquire),
        })
    }
}

/// Owns one message-only HWND and its dispatch thread. Stop/Drop remove the listener,
/// destroy the window on its thread, unregister its class and join before returning.
/// No user clipboard contents or database are accessed here.
pub struct ClipboardListener {
    stop: Arc<StopState>,
    thread: Option<JoinHandle<Result<(), String>>>,
    #[cfg(test)]
    window: isize,
}
impl ClipboardListener {
    pub fn start() -> Result<(Self, ClipboardEvents), String> {
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|error| format!("Create listener stop event: {error}"))?;
        let stop = Arc::new(StopState {
            requested: AtomicBool::new(false),
            event: WakeEvent(event),
        });
        let signal = Arc::new(SignalState::default());
        let (sender, receiver) = mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_stop = Arc::clone(&stop);
        let state = WindowState {
            sender,
            signal: Arc::clone(&signal),
        };
        let thread = thread::Builder::new()
            .name("TieZ-native-clipboard".into())
            .spawn(move || {
                let result = run_window(state, &thread_stop, &ready_tx);
                if let Err(error) = &result {
                    let _ = ready_tx.try_send(Err(error.clone()));
                }
                result
            })
            .map_err(|error| format!("Start listener thread: {error}"))?;
        match ready_rx.recv() {
            Ok(Ok(window)) => {
                #[cfg(not(test))]
                let _ = window;
                Ok((
                    Self {
                        stop,
                        thread: Some(thread),
                        #[cfg(test)]
                        window,
                    },
                    ClipboardEvents { receiver, signal },
                ))
            }
            result => {
                stop.requested.store(true, Ordering::Release);
                let _ = unsafe { SetEvent(stop.event.0) };
                let joined = thread.join();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => format!("Listener startup ended without acknowledgement: {joined:?}"),
                })
            }
        }
    }

    pub fn stop(&mut self) -> Result<(), String> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        self.stop.requested.store(true, Ordering::Release);
        // A timeout in the message wait also checks requested, so even a failed
        // signal cannot strand the join. Never join while holding a core mutex.
        let wake = unsafe { SetEvent(self.stop.event.0) };
        let result = thread
            .join()
            .map_err(|_| "Listener thread panicked".to_string())?;
        result?;
        wake.map_err(|error| format!("Signal listener stop: {error}"))
    }
}
impl Drop for ClipboardListener {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct WindowClass {
    name: Vec<u16>,
    instance: HINSTANCE,
    registered: bool,
}
impl WindowClass {
    fn register() -> Result<Self, String> {
        let name: Vec<u16> = format!(
            "TieZNativeClipboard-{}-{}",
            std::process::id(),
            NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let instance = HINSTANCE(
            unsafe { GetModuleHandleW(None) }
                .map_err(|e| format!("Get module: {e}"))?
                .0,
        );
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(format!(
                "Register listener class: {}",
                windows::core::Error::from_win32()
            ));
        }
        Ok(Self {
            name,
            instance,
            registered: true,
        })
    }
    fn unregister(&mut self) -> Result<(), String> {
        if !self.registered {
            return Ok(());
        }
        unsafe { UnregisterClassW(PCWSTR(self.name.as_ptr()), Some(self.instance)) }
            .map_err(|e| format!("Unregister listener class: {e}"))?;
        self.registered = false;
        Ok(())
    }
}
impl Drop for WindowClass {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

struct ListenerWindow {
    window: HWND,
    registered: bool,
}
impl ListenerWindow {
    fn close(&mut self) -> Result<(), String> {
        let mut error = None;
        if self.registered {
            error = unsafe { RemoveClipboardFormatListener(self.window) }.err();
            self.registered = false;
        }
        if unsafe { IsWindow(Some(self.window)) }.as_bool() {
            // Detach the borrowed state even if native destruction reports failure.
            unsafe {
                SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
            }
            if let Err(failure) = unsafe { DestroyWindow(self.window) } {
                error = Some(failure);
            }
        }
        match error {
            Some(error) => Err(format!("Close listener window: {error}")),
            None => Ok(()),
        }
    }
}
impl Drop for ListenerWindow {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn run_window(
    state: WindowState,
    stop: &StopState,
    ready: &SyncSender<Result<isize, String>>,
) -> Result<(), String> {
    let mut class = WindowClass::register()?;
    // This Box remains owned by this thread until after window destruction, even
    // when CreateWindowExW fails midway through creation. The window only borrows it.
    let state = Box::new(state);
    let window = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class.name.as_ptr()),
            PCWSTR::null(),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(class.instance),
            Some((&*state as *const WindowState).cast()),
        )
    }
    .map_err(|e| format!("Create listener window: {e}"))?;
    let mut window = ListenerWindow {
        window,
        registered: false,
    };
    unsafe { AddClipboardFormatListener(window.window) }
        .map_err(|e| format!("Register clipboard listener: {e}"))?;
    window.registered = true;
    ready
        .send(Ok(window.window.0 as isize))
        .map_err(|_| "Listener owner left during startup".to_string())?;
    let result = pump(stop);
    let close = window.close();
    let unregister = class.unregister();
    result.and(close).and(unregister)
}

fn pump(stop: &StopState) -> Result<(), String> {
    while !stop.requested.load(Ordering::Acquire) {
        let result =
            unsafe { MsgWaitForMultipleObjects(Some(&[stop.event.0]), false, 1000, QS_ALLINPUT) };
        if result == WAIT_FAILED {
            return Err(format!(
                "Wait for listener messages: {}",
                windows::core::Error::from_win32()
            ));
        }
        if result == WAIT_TIMEOUT {
            continue;
        }
        let mut message = MSG::default();
        while !stop.requested.load(Ordering::Acquire)
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

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
    } else if message == WM_CLIPBOARDUPDATE {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *const WindowState;
        if let Some(state) = state.as_ref() {
            state
                .signal
                .sequence
                .store(GetClipboardSequenceNumber(), Ordering::Release);
            let _ = state.sender.try_send(());
            state.signal.notifications.fetch_add(1, Ordering::Release);
        }
        return LRESULT(0);
    } else if message == WM_DESTROY {
        PostQuitMessage(0);
    } else if message == WM_NCDESTROY {
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
    }
    DefWindowProcW(window, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn notify(listener: &ClipboardListener, count: usize) {
        for _ in 0..count {
            unsafe {
                PostMessageW(
                    Some(HWND(listener.window as _)),
                    WM_CLIPBOARDUPDATE,
                    WPARAM(0),
                    LPARAM(0),
                )
            }
            .unwrap();
        }
    }

    #[test]
    fn repeated_start_stop_destroys_window_and_disconnects() {
        for _ in 0..4 {
            let (mut listener, events) = ClipboardListener::start().unwrap();
            let window = HWND(listener.window as _);
            assert!(unsafe { IsWindow(Some(window)) }.as_bool());
            notify(&listener, 1);
            let _hint = events.recv_timeout(Duration::from_secs(2)).unwrap();
            listener.stop().unwrap();
            listener.stop().unwrap();
            assert!(!unsafe { IsWindow(Some(window)) }.as_bool());
            while events.try_recv().is_ok() {}
            assert_eq!(events.try_recv(), Err(TryRecvError::Disconnected));
        }
    }

    #[test]
    fn bursts_use_one_bounded_wakeup_and_latest_metadata() {
        let (mut listener, events) = ClipboardListener::start().unwrap();
        let initial = events.signal.notifications.load(Ordering::Acquire);
        notify(&listener, 256);
        let deadline = Instant::now() + Duration::from_secs(2);
        while events.signal.notifications.load(Ordering::Acquire) < initial + 256 {
            assert!(
                Instant::now() < deadline,
                "Dispatch thread did not drain synthetic updates"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let hint = events.try_recv().unwrap();
        assert_eq!(
            hint.sequence,
            events.signal.sequence.load(Ordering::Acquire)
        );
        assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
        listener.stop().unwrap();
    }

    #[test]
    fn receiver_drop_and_pending_burst_do_not_block_shutdown() {
        let (mut listener, events) = ClipboardListener::start().unwrap();
        drop(events);
        notify(&listener, 256);
        let window = HWND(listener.window as _);
        listener.stop().unwrap();
        assert!(!unsafe { IsWindow(Some(window)) }.as_bool());
    }

    #[test]
    fn drop_joins_the_thread_and_closes_sender() {
        let (listener, events) = ClipboardListener::start().unwrap();
        let window = HWND(listener.window as _);
        drop(listener);
        assert!(!unsafe { IsWindow(Some(window)) }.as_bool());
        while events.try_recv().is_ok() {}
        assert_eq!(
            events.recv_timeout(Duration::from_millis(100)),
            Err(RecvTimeoutError::Disconnected)
        );
    }
}
