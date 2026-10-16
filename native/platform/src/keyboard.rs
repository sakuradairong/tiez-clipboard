//! A checked Ctrl+V batch requiring idle modifiers/V; cleans up accepted pending keys.
use crate::clipboard_formats::get_clipboard_sequence_number;
use crate::paste_input_rules::{track_accepted_paste_keys, PasteKey};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

type KeyEvent = (PasteKey, bool);
const CONTROL: PasteKey = PasteKey {
    virtual_key: 0xA2,
    scan_code: 0,
    flags: 0,
};
const V: PasteKey = PasteKey {
    virtual_key: 0x56,
    scan_code: 0,
    flags: 0,
};
const PASTE: [KeyEvent; 4] = [(CONTROL, false), (V, false), (V, true), (CONTROL, true)];

#[derive(Debug)]
pub struct InputError {
    pub code: &'static str,
    pub sent: u32,
    pub cleanup_sent: u32,
    pub cleanup_complete: bool,
}
impl InputError {
    fn before_input(code: &'static str) -> Self {
        Self {
            code,
            sent: 0,
            cleanup_sent: 0,
            cleanup_complete: true,
        }
    }
}
trait Input {
    fn idle(&mut self) -> bool;
    fn validate(&mut self) -> Result<(), &'static str>;
    fn send(&mut self, keys: &[KeyEvent]) -> u32;
}
fn send_checked(input: &mut impl Input) -> Result<(), InputError> {
    if !input.idle() {
        return Err(InputError::before_input("paste_keys_held"));
    }
    input.validate().map_err(InputError::before_input)?;
    // A single SendInput batch prevents our own down/up sequence interleaving.
    let sent = input.send(&PASTE);
    let mut pending = Vec::new();
    track_accepted_paste_keys(&mut pending, &PASTE, sent);
    if sent == PASTE.len() as u32 {
        return Ok(());
    }
    let releases: Vec<_> = pending.iter().rev().map(|key| (*key, true)).collect();
    let cleanup_sent = if releases.is_empty() {
        0
    } else {
        input.send(&releases)
    };
    track_accepted_paste_keys(&mut pending, &releases, cleanup_sent);
    let cleanup_complete = pending.is_empty();
    Err(InputError {
        code: if !cleanup_complete {
            "paste_cleanup_incomplete"
        } else if sent == 0 {
            "paste_input_blocked"
        } else {
            "paste_input_partial"
        },
        sent,
        cleanup_sent,
        cleanup_complete,
    })
}

pub fn ensure_keys_idle() -> Result<(), &'static str> {
    // GetAsyncKeyState's high bit reports current down state, not its reset-on-read bit.
    let held = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN, VK_V]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } & i16::MIN != 0);
    if held {
        Err("paste_keys_held")
    } else {
        Ok(())
    }
}
struct WindowsInput<F> {
    sequence: u32,
    target: F,
}
impl<F: FnMut() -> Result<(), &'static str>> Input for WindowsInput<F> {
    fn idle(&mut self) -> bool {
        ensure_keys_idle().is_ok()
    }
    fn validate(&mut self) -> Result<(), &'static str> {
        if get_clipboard_sequence_number() != self.sequence {
            return Err("paste_clipboard_changed");
        }
        (self.target)()
    }
    fn send(&mut self, keys: &[KeyEvent]) -> u32 {
        let inputs: Vec<_> = keys
            .iter()
            .map(|(key, up)| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(key.virtual_key),
                        wScan: key.scan_code,
                        dwFlags: KEYBD_EVENT_FLAGS(
                            key.flags | if *up { KEYEVENTF_KEYUP.0 } else { 0 },
                        ),
                        dwExtraInfo: 0x545A4E50,
                        ..Default::default()
                    },
                },
            })
            .collect();
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) }
    }
}
/// The guard must verify the exact target immediately before input. Windows does
/// not atomically bind global input to HWND; a later focus change remains possible.
pub fn send_paste(
    sequence: u32,
    target: impl FnMut() -> Result<(), &'static str>,
) -> Result<(), InputError> {
    send_checked(&mut WindowsInput { sequence, target })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        held: bool,
        guard: Result<(), &'static str>,
        counts: VecDeque<u32>,
        batches: Vec<Vec<KeyEvent>>,
        validated: bool,
    }
    impl Fake {
        fn new(counts: &[u32]) -> Self {
            Self {
                held: false,
                guard: Ok(()),
                counts: counts.iter().copied().collect(),
                batches: Vec::new(),
                validated: false,
            }
        }
    }
    impl Input for Fake {
        fn idle(&mut self) -> bool {
            !self.held
        }
        fn validate(&mut self) -> Result<(), &'static str> {
            self.validated = true;
            self.guard
        }
        fn send(&mut self, keys: &[KeyEvent]) -> u32 {
            assert!(self.validated);
            self.batches.push(keys.to_vec());
            self.counts.pop_front().expect("Unexpected input retry")
        }
    }
    #[test]
    fn modifiers_and_v_held_never_get_released_or_injected() {
        let mut input = Fake::new(&[]);
        input.held = true;
        let error = send_checked(&mut input).unwrap_err();
        assert_eq!(error.code, "paste_keys_held");
        assert_eq!(error.sent, 0);
        assert!(!input.validated);
        assert!(input.batches.is_empty());
    }
    #[test]
    fn changed_clipboard_or_focus_never_sends_input() {
        for code in [
            "paste_clipboard_changed",
            "target_focus_changed",
            "target_changed",
        ] {
            let mut input = Fake::new(&[]);
            input.guard = Err(code);
            let error = send_checked(&mut input).unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(error.sent, 0);
            assert!(input.batches.is_empty());
        }
    }
    #[test]
    fn complete_input_is_one_balanced_batch() {
        let mut input = Fake::new(&[4]);
        send_checked(&mut input).unwrap();
        assert_eq!(input.batches, vec![PASTE.to_vec()]);
    }
    #[test]
    fn blocked_input_does_not_release_user_keys_or_retry() {
        let mut input = Fake::new(&[0]);
        let error = send_checked(&mut input).unwrap_err();
        assert_eq!(error.code, "paste_input_blocked");
        assert_eq!(error.cleanup_sent, 0);
        assert!(error.cleanup_complete);
        assert_eq!(input.batches.len(), 1);
    }
    #[test]
    fn every_partial_prefix_releases_only_accepted_pending_keys_in_reverse() {
        for (sent, releases) in [
            (1, vec![(CONTROL, true)]),
            (2, vec![(V, true), (CONTROL, true)]),
            (3, vec![(CONTROL, true)]),
        ] {
            let mut input = Fake::new(&[sent, releases.len() as u32]);
            let error = send_checked(&mut input).unwrap_err();
            assert_eq!(error.code, "paste_input_partial");
            assert_eq!(error.sent, sent);
            assert!(error.cleanup_complete);
            assert_eq!(input.batches[1], releases);
            assert_eq!(input.batches.len(), 2);
        }
    }
    #[test]
    fn incomplete_cleanup_is_reported_without_retrying_paste() {
        for cleanup in [0, 1] {
            let mut input = Fake::new(&[2, cleanup]);
            let error = send_checked(&mut input).unwrap_err();
            assert_eq!(error.code, "paste_cleanup_incomplete");
            assert!(!error.cleanup_complete);
            assert_eq!(error.cleanup_sent, cleanup);
            assert_eq!(input.batches.len(), 2);
        }
    }
}
