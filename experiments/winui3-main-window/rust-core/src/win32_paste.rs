//! Windows clipboard + Ctrl+V executor for the WinUI probe.
//!
//! Focus restore and window hide stay in the C++ `UiLifecycle` shell so this
//! module never takes a Tauri `AppHandle` or HWND through the C ABI.

use std::ptr;
use tiez_core::paste_coordinator::{execute_paste, PasteExecutor, PastePayload, PastePlan};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

const CF_UNICODETEXT: u32 = 13;
const VK_CONTROL: VIRTUAL_KEY = 0x11;
const VK_V: VIRTUAL_KEY = 0x56;

pub(crate) struct Win32PasteExecutor;

impl PasteExecutor for Win32PasteExecutor {
    fn hide_window(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn restore_focus(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn apply_payload(&mut self, payload: &PastePayload) -> Result<(), String> {
        set_unicode_text(&payload.text)
    }

    fn send_paste_keystroke(&mut self) -> Result<(), String> {
        send_ctrl_v()
    }
}

pub(crate) fn execute(plan: &PastePlan) -> Result<(), String> {
    execute_paste(plan, &mut Win32PasteExecutor)
}

fn set_unicode_text(text: &str) -> Result<(), String> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len().saturating_mul(std::mem::size_of::<u16>());

    unsafe {
        if OpenClipboard(ptr::null_mut()) == 0 {
            return Err("OpenClipboard failed".to_owned());
        }
        if EmptyClipboard() == 0 {
            CloseClipboard();
            return Err("EmptyClipboard failed".to_owned());
        }
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if handle.is_null() {
            CloseClipboard();
            return Err("GlobalAlloc failed".to_owned());
        }
        let locked = GlobalLock(handle);
        if locked.is_null() {
            CloseClipboard();
            return Err("GlobalLock failed".to_owned());
        }
        ptr::copy_nonoverlapping(wide.as_ptr(), locked.cast::<u16>(), wide.len());
        GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT, handle).is_null() {
            CloseClipboard();
            return Err("SetClipboardData failed".to_owned());
        }
        CloseClipboard();
    }
    Ok(())
}

fn send_ctrl_v() -> Result<(), String> {
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_V, false),
        keyboard_input(VK_V, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err("SendInput failed for Ctrl+V".to_owned());
    }
    Ok(())
}

fn keyboard_input(virtual_key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
