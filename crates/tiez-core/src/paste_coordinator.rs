//! Tauri-independent paste planning shared by native TieZ frontends.
//!
//! Platform adapters own clipboard I/O, focus restoration, and keystrokes.
//! This module owns payload selection, the hide → restore-focus → apply →
//! Ctrl+V sequence, delete-after-paste intent, and a bounded paste-queue policy.

use crate::clipboard_history::HistoryContent;
use std::collections::VecDeque;
use std::fmt;

/// Plain Unicode text versus HTML-preferring rich paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteFormat {
    Plain,
    Rich,
}

impl PasteFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Rich => "rich",
        }
    }
}

/// Clipboard bytes the executor should apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastePayload {
    pub format: PasteFormat,
    pub text: String,
    pub html: Option<String>,
}

/// One paste attempt, including window/focus/keystroke contracts.
///
/// `delete_after` is recorded for the caller. Executors must not delete
/// history rows; persistence adapters apply that flag after a successful paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastePlan {
    pub item_id: i64,
    pub payload: PastePayload,
    pub restore_focus: bool,
    pub hide_window: bool,
    pub send_ctrl_v: bool,
    pub delete_after: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteError {
    Unavailable { item_id: i64, reason: String },
    Empty { item_id: i64 },
}

impl fmt::Display for PasteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { item_id, reason } => {
                write!(
                    formatter,
                    "clipboard entry {item_id} is not available to paste: {reason}"
                )
            }
            Self::Empty { item_id } => {
                write!(formatter, "clipboard entry {item_id} has no pasteable text")
            }
        }
    }
}

impl std::error::Error for PasteError {}

/// Platform operations required to carry out a [`PastePlan`].
pub trait PasteExecutor {
    fn hide_window(&mut self) -> Result<(), String>;
    fn restore_focus(&mut self) -> Result<(), String>;
    fn apply_payload(&mut self, payload: &PastePayload) -> Result<(), String>;
    fn send_paste_keystroke(&mut self) -> Result<(), String>;
}

/// Records executor calls for in-memory tests. Never touches the OS clipboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingPasteExecutor {
    pub events: Vec<String>,
    pub last_payload: Option<PastePayload>,
}

impl PasteExecutor for RecordingPasteExecutor {
    fn hide_window(&mut self) -> Result<(), String> {
        self.events.push("hide_window".to_owned());
        Ok(())
    }

    fn restore_focus(&mut self) -> Result<(), String> {
        self.events.push("restore_focus".to_owned());
        Ok(())
    }

    fn apply_payload(&mut self, payload: &PastePayload) -> Result<(), String> {
        self.events.push(format!(
            "apply_payload:{}:{}",
            payload.format.as_str(),
            payload.text.len()
        ));
        self.last_payload = Some(payload.clone());
        Ok(())
    }

    fn send_paste_keystroke(&mut self) -> Result<(), String> {
        self.events.push("send_ctrl_v".to_owned());
        Ok(())
    }
}

/// Builds a paste plan from resolved history content.
///
/// Sensitive or encrypted entries that the history adapter already marked
/// unavailable are rejected here. Callers that already decrypted a payload
/// (the production Tauri path) should pass `available: true`.
pub fn plan_paste(
    content: &HistoryContent,
    format: PasteFormat,
    delete_after: bool,
) -> Result<PastePlan, PasteError> {
    if !content.available {
        return Err(PasteError::Unavailable {
            item_id: content.id,
            reason: content
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "payload is protected".to_owned()),
        });
    }

    let text = content.content.clone();
    let html = match format {
        PasteFormat::Plain => None,
        PasteFormat::Rich => content
            .html_content
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    };

    if text.is_empty() && html.is_none() {
        return Err(PasteError::Empty {
            item_id: content.id,
        });
    }

    Ok(PastePlan {
        item_id: content.id,
        payload: PastePayload { format, text, html },
        restore_focus: true,
        hide_window: true,
        send_ctrl_v: true,
        delete_after,
    })
}

/// Runs the hide → restore-focus → apply → keystroke sequence.
pub fn execute_paste<E: PasteExecutor>(
    plan: &PastePlan,
    executor: &mut E,
) -> Result<(), String> {
    if plan.hide_window {
        executor.hide_window()?;
    }
    if plan.restore_focus {
        executor.restore_focus()?;
    }
    executor.apply_payload(&plan.payload)?;
    if plan.send_ctrl_v {
        executor.send_paste_keystroke()?;
    }
    Ok(())
}

/// Bounded FIFO of clipboard IDs waiting for sequential paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteItemQueue {
    items: VecDeque<i64>,
    max_size: usize,
}

impl PasteItemQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            items: VecDeque::new(),
            max_size: max_size.max(1),
        }
    }

    pub fn replace(&mut self, ids: impl IntoIterator<Item = i64>) {
        self.items.clear();
        for id in ids {
            if self.items.len() >= self.max_size {
                break;
            }
            self.items.push_back(id);
        }
    }

    pub fn pop_front(&mut self) -> Option<i64> {
        self.items.pop_front()
    }

    pub fn push_front(&mut self, id: i64) {
        self.items.push_front(id);
        while self.items.len() > self.max_size {
            self.items.pop_back();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn ids(&self) -> Vec<i64> {
        self.items.iter().copied().collect()
    }

    pub fn into_vecdeque(self) -> VecDeque<i64> {
        self.items
    }
}

/// Replace a platform queue's ID list using the shared cap/order policy.
pub fn replace_paste_ids(dest: &mut VecDeque<i64>, ids: Vec<i64>, max_size: usize) {
    let mut queue = PasteItemQueue::new(max_size);
    queue.replace(ids);
    *dest = queue.into_vecdeque();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available_text(id: i64, text: &str, html: Option<&str>) -> HistoryContent {
        HistoryContent {
            id,
            content_type: "text".to_owned(),
            content: text.to_owned(),
            html_content: html.map(str::to_owned),
            available: true,
            is_sensitive: false,
            unavailable_reason: None,
        }
    }

    #[test]
    fn plan_paste_plain_drops_html_and_rich_keeps_it() {
        let content = available_text(11, "hello", Some("<b>hello</b>"));

        let plain = plan_paste(&content, PasteFormat::Plain, true).unwrap();
        assert_eq!(plain.item_id, 11);
        assert_eq!(plain.payload.format, PasteFormat::Plain);
        assert_eq!(plain.payload.text, "hello");
        assert_eq!(plain.payload.html, None);
        assert!(plain.restore_focus);
        assert!(plain.hide_window);
        assert!(plain.send_ctrl_v);
        assert!(plain.delete_after);

        let rich = plan_paste(&content, PasteFormat::Rich, false).unwrap();
        assert_eq!(rich.payload.format, PasteFormat::Rich);
        assert_eq!(rich.payload.html.as_deref(), Some("<b>hello</b>"));
        assert!(!rich.delete_after);
    }

    #[test]
    fn plan_paste_rejects_unavailable_and_empty_payloads() {
        let unavailable = HistoryContent {
            id: 7,
            content_type: "text".to_owned(),
            content: String::new(),
            html_content: None,
            available: false,
            is_sensitive: true,
            unavailable_reason: Some("Sensitive entry requires the production privacy adapter".to_owned()),
        };
        match plan_paste(&unavailable, PasteFormat::Plain, false).unwrap_err() {
            PasteError::Unavailable { item_id, reason } => {
                assert_eq!(item_id, 7);
                assert!(reason.contains("privacy"));
            }
            other => panic!("expected unavailable, got {other:?}"),
        }

        let empty = available_text(8, "", None);
        match plan_paste(&empty, PasteFormat::Plain, false).unwrap_err() {
            PasteError::Empty { item_id } => assert_eq!(item_id, 8),
            other => panic!("expected empty, got {other:?}"),
        }
    }

    #[test]
    fn execute_paste_runs_hide_restore_apply_then_keystroke() {
        let plan = plan_paste(
            &available_text(101, "notepad text", None),
            PasteFormat::Plain,
            false,
        )
        .unwrap();
        let mut executor = RecordingPasteExecutor::default();

        execute_paste(&plan, &mut executor).unwrap();

        assert_eq!(
            executor.events,
            vec![
                "hide_window".to_owned(),
                "restore_focus".to_owned(),
                "apply_payload:plain:12".to_owned(),
                "send_ctrl_v".to_owned(),
            ]
        );
        assert_eq!(executor.last_payload.unwrap().text, "notepad text");
    }

    #[test]
    fn paste_queue_caps_order_and_requeues_failed_ids() {
        let mut queue = PasteItemQueue::new(2);
        queue.replace([1, 2, 3, 4]);
        assert_eq!(queue.ids(), vec![1, 2]);
        assert_eq!(queue.pop_front(), Some(1));
        queue.push_front(1);
        assert_eq!(queue.ids(), vec![1, 2]);

        let mut dest = VecDeque::from([9, 8]);
        replace_paste_ids(&mut dest, vec![10, 11, 12], 2);
        assert_eq!(dest, VecDeque::from([10, 11]));
    }
}
