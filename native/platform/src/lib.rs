//! Windows host services for the native runtime, without Tauri or a browser host.
//! Notification handling never opens or reads the clipboard; the core owns processing.
#![cfg(windows)]

#[path = "../../../src-tauri/src/infrastructure/windows_api/win_clipboard.rs"]
pub mod clipboard_formats;
mod listener;
pub mod source_app;
pub use listener::{ClipboardEvents, ClipboardListener, ClipboardSignal};

#[path = "../../../src-tauri/src/services/paste_rules.rs"]
mod paste_rules;
pub mod target;

#[path = "../../../src-tauri/src/services/paste_input_rules.rs"]
mod paste_input_rules;
pub mod keyboard;

pub mod shell;

pub mod hotkey;

pub mod open;

pub mod apps;

pub mod app_icons;
