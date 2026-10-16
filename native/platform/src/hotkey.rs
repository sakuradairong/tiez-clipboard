//! Keyboard shortcut parsing and two-phase replacement. Runs on the shell owner thread.
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: u32,
    pub key: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Disabled,
    Keyboard(Chord),
    MiddleMouse,
}
impl Shortcut {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        if value.trim().is_empty() {
            return Ok(Self::Disabled);
        }
        if value.eq_ignore_ascii_case("MouseMiddle") || value.eq_ignore_ascii_case("MButton") {
            return Ok(Self::MiddleMouse);
        }
        if value.len() > 128 {
            return Err("hotkey_invalid");
        }
        let mut modifiers = 0;
        let mut key = None;
        for part in value
            .split('+')
            .map(|part| part.trim().to_ascii_lowercase())
        {
            let modifier = match part.as_str() {
                "alt" | "option" => 1,
                "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => 2,
                "shift" => 4,
                "win" | "windows" | "super" | "command" | "cmd" | "meta" => 8,
                _ => 0,
            };
            if modifier != 0 {
                if modifiers & modifier != 0 {
                    return Err("hotkey_invalid");
                }
                modifiers |= modifier;
            } else {
                if key.is_some() {
                    return Err("hotkey_invalid");
                }
                key = Some(parse_key(&part).ok_or("hotkey_invalid")?);
            }
        }
        Ok(Self::Keyboard(Chord {
            modifiers,
            key: key.ok_or("hotkey_invalid")?,
        }))
    }
}
fn parse_key(value: &str) -> Option<u32> {
    if value.len() == 1 {
        let byte = value.as_bytes()[0];
        if byte.is_ascii_alphanumeric() {
            return Some(u32::from(byte.to_ascii_uppercase()));
        }
    }
    if let Some(number) = value
        .strip_prefix('f')
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Some(0x70 + number - 1);
    }
    if let Some(number) = value
        .strip_prefix("numpad")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n <= 9)
    {
        return Some(0x60 + number);
    }
    if let Some(suffix) = value
        .strip_prefix("key")
        .filter(|v| v.len() == 1 && v.as_bytes()[0].is_ascii_alphabetic())
    {
        return parse_key(suffix);
    }
    if let Some(suffix) = value
        .strip_prefix("digit")
        .filter(|v| v.len() == 1 && v.as_bytes()[0].is_ascii_digit())
    {
        return parse_key(suffix);
    }
    Some(match value {
        "space" => 0x20,
        "tab" => 9,
        "enter" | "return" => 0x0D,
        "escape" | "esc" => 0x1B,
        "backspace" => 8,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "left" | "arrowleft" => 0x25,
        "right" | "arrowright" => 0x27,
        "up" | "arrowup" => 0x26,
        "down" | "arrowdown" => 0x28,
        "semicolon" => 0xBA,
        "equal" => 0xBB,
        "comma" => 0xBC,
        "minus" => 0xBD,
        "period" => 0xBE,
        "slash" => 0xBF,
        "backquote" => 0xC0,
        "bracketleft" => 0xDB,
        "backslash" => 0xDC,
        "bracketright" => 0xDD,
        "quote" => 0xDE,
        "multiply" | "numpadmultiply" => 0x6A,
        "add" | "numpadadd" => 0x6B,
        "subtract" | "numpadsubtract" => 0x6D,
        "decimal" | "numpaddecimal" => 0x6E,
        "divide" | "numpaddivide" => 0x6F,
        _ => return None,
    })
}

pub(crate) trait Backend {
    fn register(&mut self, id: i32, chord: Chord) -> Result<(), &'static str>;
    fn unregister(&mut self, id: i32) -> Result<(), &'static str>;
}
pub(crate) struct WindowsBackend(pub HWND);
impl Backend for WindowsBackend {
    fn register(&mut self, id: i32, chord: Chord) -> Result<(), &'static str> {
        unsafe {
            RegisterHotKey(
                Some(self.0),
                id,
                HOT_KEY_MODIFIERS(chord.modifiers) | MOD_NOREPEAT,
                chord.key,
            )
        }
        .map_err(|_| "hotkey_conflict")
    }
    fn unregister(&mut self, id: i32) -> Result<(), &'static str> {
        unsafe { UnregisterHotKey(Some(self.0), id) }.map_err(|_| "hotkey_unregister")
    }
}
#[derive(Clone, Copy)]
struct Binding {
    id: i32,
    chord: Chord,
}
struct Pending {
    token: u64,
    same: bool,
    next: Option<Binding>,
}
pub(crate) struct Bindings<B: Backend> {
    backend: B,
    current: Option<Binding>,
    pending: Option<Pending>,
    retired: Vec<i32>,
    next_id: i32,
    token: u64,
    pub error: Option<&'static str>,
}
impl<B: Backend> Bindings<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            current: None,
            pending: None,
            retired: Vec::new(),
            next_id: 0x4100,
            token: 0,
            error: None,
        }
    }
    pub fn active(&self) -> bool {
        self.current.is_some()
    }
    pub fn prepare(&mut self, shortcut: Shortcut) -> Result<u64, &'static str> {
        let result = self.prepare_inner(shortcut);
        self.error = result
            .as_ref()
            .err()
            .copied()
            .or(if self.retired.is_empty() {
                None
            } else {
                Some("hotkey_unregister")
            });
        result
    }
    fn prepare_inner(&mut self, shortcut: Shortcut) -> Result<u64, &'static str> {
        if self.pending.is_some() {
            return Err("hotkey_update_pending");
        }
        if !self.retired.is_empty() {
            return Err("hotkey_restart_required");
        }
        let next_token = self
            .token
            .checked_add(1)
            .ok_or("hotkey_generation_exhausted")?;
        let desired = match shortcut {
            Shortcut::Keyboard(chord) => Some(chord),
            Shortcut::Disabled => None,
            Shortcut::MiddleMouse => return Err("hotkey_mouse_pending"),
        };
        let same = self.current.map(|binding| binding.chord) == desired;
        let next = if same {
            None
        } else if let Some(chord) = desired {
            if self.next_id > 0xBFFF {
                return Err("hotkey_restart_required");
            }
            self.backend.register(self.next_id, chord)?;
            let binding = Binding {
                id: self.next_id,
                chord,
            };
            self.next_id += 1;
            Some(binding)
        } else {
            None
        };
        self.token = next_token;
        self.pending = Some(Pending {
            token: next_token,
            same,
            next,
        });
        Ok(next_token)
    }
    pub fn resolve(&mut self, token: u64, commit: bool) -> Result<(), &'static str> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.token != token)
        {
            return Err("hotkey_stale_transaction");
        }
        let pending = self.pending.take().unwrap();
        if pending.same {
            return Ok(());
        }
        let obsolete = if commit {
            std::mem::replace(&mut self.current, pending.next)
        } else {
            pending.next
        };
        if let Some(binding) = obsolete {
            if let Err(code) = self.backend.unregister(binding.id) {
                self.retired.push(binding.id);
                self.error = Some(code);
                return Err(code);
            }
        }
        self.error = None;
        Ok(())
    }
    pub fn accepts(&self, id: usize, packed: isize) -> bool {
        self.current.is_some_and(|binding| {
            binding.id as usize == id
                && binding.chord.modifiers == packed as u32 & 0xffff
                && binding.chord.key == (packed as u32 >> 16) & 0xffff
        })
    }
}
impl<B: Backend> Drop for Bindings<B> {
    fn drop(&mut self) {
        if let Some(binding) = self.current.take() {
            let _ = self.backend.unregister(binding.id);
        }
        if let Some(binding) = self.pending.take().and_then(|pending| pending.next) {
            let _ = self.backend.unregister(binding.id);
        }
        for id in self.retired.drain(..) {
            let _ = self.backend.unregister(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Fake {
        registered: BTreeMap<i32, Chord>,
        conflict: bool,
        failed_release: bool,
    }
    impl Backend for Fake {
        fn register(&mut self, id: i32, chord: Chord) -> Result<(), &'static str> {
            if self.conflict {
                return Err("hotkey_conflict");
            }
            self.registered.insert(id, chord);
            Ok(())
        }
        fn unregister(&mut self, id: i32) -> Result<(), &'static str> {
            if self.failed_release {
                return Err("hotkey_unregister");
            }
            self.registered.remove(&id);
            Ok(())
        }
    }
    fn shortcut(key: u32) -> Shortcut {
        Shortcut::Keyboard(Chord { modifiers: 7, key })
    }
    #[test]
    fn parser_preserves_windows_aliases_and_navigation_keys() {
        let expected = Shortcut::Keyboard(Chord {
            modifiers: 15,
            key: 0x87,
        });
        assert_eq!(Shortcut::parse("Win+Ctrl+Alt+Shift+F24").unwrap(), expected);
        assert_eq!(
            Shortcut::parse("command+control+option+shift+f24").unwrap(),
            expected
        );
        assert_eq!(
            Shortcut::parse("Ctrl+KeyV").unwrap(),
            Shortcut::Keyboard(Chord {
                modifiers: 2,
                key: 0x56
            })
        );
        assert_eq!(Shortcut::parse("  ").unwrap(), Shortcut::Disabled);
        assert_eq!(
            Shortcut::parse("MouseMiddle").unwrap(),
            Shortcut::MiddleMouse
        );
        assert_eq!(
            Shortcut::parse("Alt+ArrowLeft").unwrap(),
            Shortcut::Keyboard(Chord {
                modifiers: 1,
                key: 0x25
            })
        );
        for invalid in [
            "Ctrl",
            "Ctrl+Control+V",
            "Ctrl++V",
            "A+B",
            "F25",
            "Ctrl+💡",
            "Ctrl+MouseMiddle",
        ] {
            assert!(Shortcut::parse(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn conflict_keeps_the_old_binding_and_pending_keys_are_not_dispatched() {
        let mut bindings = Bindings::new(Fake::default());
        let first = bindings.prepare(shortcut(0x87)).unwrap();
        bindings.resolve(first, true).unwrap();
        bindings.backend.conflict = true;
        assert_eq!(
            bindings.prepare(shortcut(0x86)).unwrap_err(),
            "hotkey_conflict"
        );
        assert!(bindings.accepts(0x4100, (0x87 << 16) | 7));
        assert_eq!(bindings.backend.registered.len(), 1);
        bindings.backend.conflict = false;
        let prepared = bindings.prepare(shortcut(0x86)).unwrap();
        assert!(!bindings.accepts(0x4101, (0x86 << 16) | 7));
        bindings.resolve(prepared, false).unwrap();
        assert_eq!(bindings.backend.registered.len(), 1);
        assert!(bindings.accepts(0x4100, (0x87 << 16) | 7));
    }
    #[test]
    fn commit_switches_once_and_rejects_stale_or_mismatched_messages() {
        let mut bindings = Bindings::new(Fake::default());
        let first = bindings.prepare(shortcut(0x87)).unwrap();
        bindings.resolve(first, true).unwrap();
        let second = bindings.prepare(shortcut(0x86)).unwrap();
        assert_eq!(
            bindings.resolve(first, true),
            Err("hotkey_stale_transaction")
        );
        bindings.resolve(second, true).unwrap();
        assert!(!bindings.accepts(0x4100, (0x87 << 16) | 7));
        assert!(!bindings.accepts(0x4101, (0x86 << 16) | 3));
        assert!(bindings.accepts(0x4101, (0x86 << 16) | 7));
        let same = bindings.prepare(shortcut(0x86)).unwrap();
        bindings.resolve(same, true).unwrap();
        assert_eq!(bindings.backend.registered.len(), 1);
        let disabled = bindings.prepare(Shortcut::Disabled).unwrap();
        bindings.resolve(disabled, true).unwrap();
        assert!(!bindings.active());
        assert!(bindings.backend.registered.is_empty());
    }
    #[test]
    fn cleanup_failure_is_explicit_and_prevents_unbounded_retired_bindings() {
        let mut bindings = Bindings::new(Fake::default());
        let first = bindings.prepare(shortcut(0x87)).unwrap();
        bindings.resolve(first, true).unwrap();
        let next = bindings.prepare(shortcut(0x86)).unwrap();
        bindings.backend.failed_release = true;
        assert_eq!(bindings.resolve(next, true), Err("hotkey_unregister"));
        assert!(bindings.accepts(0x4101, (0x86 << 16) | 7));
        assert!(!bindings.accepts(0x4100, (0x87 << 16) | 7));
        assert_eq!(
            bindings.prepare(shortcut(0x85)),
            Err("hotkey_restart_required")
        );
        assert_eq!(bindings.retired.len(), 1);
    }
}
