//! Accepted-input state shared by legacy and native paste senders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PasteKey {
    pub(crate) virtual_key: u16,
    pub(crate) scan_code: u16,
    pub(crate) flags: u32,
}

pub(crate) fn track_accepted_paste_keys(
    pending_keys: &mut Vec<PasteKey>,
    keys: &[(PasteKey, bool)],
    sent: u32,
) {
    for (key, is_key_up) in keys.iter().take(sent as usize) {
        if *is_key_up {
            pending_keys.retain(|pending| pending != key);
        } else if !pending_keys.contains(key) {
            pending_keys.push(*key);
        }
    }
}
