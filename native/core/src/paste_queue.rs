use crate::{CoreError, CoreResult};
use crate::history::{HistoryChanges, HistoryService};
use serde::Serialize;
use std::collections::VecDeque;

pub(crate) const QUEUE_LIMIT: usize = 1000;

#[derive(Clone, Copy, Debug)]
struct Item { id: i64, token: u64 }
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ticket { pub id: i64, token: u64 }

#[derive(Default)]
pub(crate) struct PasteQueue {
    items: VecDeque<Item>,
    revision: u64,
    next_token: u64,
    observed_history: u64,
    reset_on_capture: bool,
    requires_reset: bool,
    last_error: Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub(crate) struct QueueSnapshot {
    pub revision: u64,
    pub history_revision: u64,
    pub ids: Vec<i64>,
    pub requires_reset: bool,
    pub last_error: Option<&'static str>,
}

fn conflict() -> CoreError {
    CoreError::new("queue_conflict", "Queue changed; reload before changing or pasting it")
}
impl PasteQueue {
    fn advance(&mut self) {
        self.revision = self.revision.checked_add(1).expect("queue revision exhausted");
    }
    fn append(&mut self, id: i64) {
        self.next_token = self.next_token.checked_add(1).expect("queue token exhausted");
        self.items.push_back(Item { id, token: self.next_token });
    }
    fn synchronize(&mut self, changes: HistoryChanges) {
        self.observed_history = changes.revision;
        if self.items.is_empty() { return; }
        if changes.resync_required {
            if !self.requires_reset {
                self.requires_reset = true;
                self.last_error = Some("queue_history_resync");
                self.advance();
            }
            return;
        }
        let mut changed = false;
        for change in changes.changes {
            if let Some(old) = change.replaced_id {
                for item in &mut self.items {
                    if item.id == old { item.id = change.id; changed = true; }
                }
            }
            let before = self.items.len();
            self.items.retain(|item| !change.removed_ids.contains(&item.id));
            changed |= before != self.items.len();
        }
        if changed { self.advance(); }
    }
    fn snapshot(&self, history_revision: u64) -> QueueSnapshot {
        QueueSnapshot { revision: self.revision, history_revision,
            ids: self.items.iter().map(|item| item.id).collect(),
            requires_reset: self.requires_reset, last_error: self.last_error }
    }
    fn replace(&mut self, ids: Vec<i64>, history_revision: u64) {
        self.items.clear();
        for id in ids { self.append(id); }
        self.observed_history = history_revision;
        self.reset_on_capture = false;
        self.requires_reset = false;
        self.last_error = None;
        self.advance();
    }
    fn captured(&mut self, id: i64) {
        if self.reset_on_capture {
            self.items.clear();
            self.reset_on_capture = false;
            self.requires_reset = false;
            self.last_error = None;
        }
        if self.items.len() >= QUEUE_LIMIT {
            if self.last_error != Some("queue_full") {
                self.last_error = Some("queue_full");
                self.advance();
            }
            return;
        }
        self.append(id);
        self.advance();
    }
    fn ticket(&self, expected_revision: u64) -> CoreResult<Option<Ticket>> {
        if self.revision != expected_revision { return Err(conflict()); }
        if self.requires_reset {
            return Err(CoreError::new("queue_history_resync", "History identity changed; rebuild the queue before pasting"));
        }
        Ok(self.items.front().map(|item| Ticket { id: item.id, token: item.token }))
    }
    fn complete(&mut self, ticket: Ticket) {
        // A replacement queue may contain the same ID; only this occurrence may be consumed.
        if self.items.front().is_some_and(|item| item.token == ticket.token) {
            self.items.pop_front();
            self.reset_on_capture = true;
            self.advance();
        }
    }
}

impl HistoryService {
    pub(crate) fn queue_snapshot(&mut self) -> CoreResult<QueueSnapshot> {
        let changes = self.changes(self.paste_queue.observed_history)?;
        self.paste_queue.synchronize(changes);
        Ok(self.paste_queue.snapshot(self.revision()))
    }
    pub(crate) fn queue_replace(&mut self, ids: Vec<i64>, expected_queue: u64, expected_history: u64) -> CoreResult<QueueSnapshot> {
        let snapshot = self.queue_snapshot()?;
        if snapshot.revision != expected_queue { return Err(conflict()); }
        if expected_history != self.revision() {
            return Err(CoreError::new("history_conflict", "History changed; select the queue again"));
        }
        if ids.len() > QUEUE_LIMIT || ids.contains(&0) {
            return Err(CoreError::new("invalid_queue", "Queue requires at most 1000 nonzero history IDs"));
        }
        for id in &ids { self.entry(*id)?; }
        self.paste_queue.replace(ids, self.revision());
        Ok(self.paste_queue.snapshot(self.revision()))
    }
    pub(crate) fn queue_captured(&mut self, id: i64) -> CoreResult<()> {
        self.queue_snapshot()?;
        self.paste_queue.captured(id);
        Ok(())
    }
    pub(crate) fn queue_ticket(&mut self, expected_queue: u64, expected_history: u64) -> CoreResult<Option<Ticket>> {
        self.queue_snapshot()?;
        if expected_history != self.revision() {
            return Err(CoreError::new("history_conflict", "History changed; reload the queue"));
        }
        self.paste_queue.ticket(expected_queue)
    }
    pub(crate) fn queue_complete(&mut self, ticket: Ticket) {
        self.paste_queue.complete(ticket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    fn history() -> HistoryService {
        let connection = Connection::open_in_memory().unwrap();
        tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
        HistoryService::from_connection(connection, std::env::temp_dir())
    }
    fn add(h: &mut HistoryService, content: &str) -> i64 {
        h.add_manual("text".into(), content.into(), None, vec![]).unwrap().id
    }
    fn set(h: &mut HistoryService, ids: Vec<i64>) -> QueueSnapshot {
        let s = h.queue_snapshot().unwrap();
        h.queue_replace(ids, s.revision, s.history_revision).unwrap()
    }
    #[test]
    fn queue_order_duplicates_clear_and_readonly_history() {
        let mut h = history();
        let a = add(&mut h, "  原始\r\n "); let b = add(&mut h, "second");
        let revision = h.revision();
        let s = set(&mut h, vec![b, a, b]);
        assert_eq!(s.ids, vec![b, a, b]);
        assert_eq!(h.revision(), revision);
        assert!(h.changes(revision).unwrap().changes.is_empty());
        assert_eq!(h.entry(a).unwrap().content, "  原始\r\n ");
        let t = h.queue_ticket(s.revision, s.history_revision).unwrap().unwrap();
        assert_eq!(h.queue_snapshot().unwrap().ids, s.ids);
        h.queue_complete(t);
        assert_eq!(h.queue_snapshot().unwrap().ids, vec![a,b]);
        h.queue_complete(t);
        assert_eq!(h.queue_snapshot().unwrap().ids, vec![a,b]);
        assert!(set(&mut h, vec![]).ids.is_empty());
    }
    #[test]
    fn replacement_cannot_be_consumed_even_when_the_head_id_is_identical() {
        let mut q = PasteQueue::default();
        q.replace(vec![1, 2], 0);
        let t = q.ticket(q.revision).unwrap().unwrap();
        q.replace(vec![1, 3], 0);
        q.complete(t);
        assert_eq!(q.snapshot(0).ids, vec![1,3]);
        let t = q.ticket(q.revision).unwrap().unwrap();
        q.captured(4);
        q.complete(t);
        assert_eq!(q.snapshot(0).ids, vec![3,4]);
        q.captured(5);
        assert_eq!(q.snapshot(0).ids, vec![5]);
    }
    #[test]
    fn promotions_and_deletions_reconcile_without_retargeting_reused_ids() {
        let mut h = history();
        let a = add(&mut h, "stored");
        let session = h.record_processed(h.entry(a).unwrap(), false).unwrap().id;
        assert!(session < 0);
        set(&mut h, vec![session, a]);
        let promoted = h.pin(session, true).unwrap().id;
        assert_eq!(h.queue_snapshot().unwrap().ids, vec![promoted, a]);
        set(&mut h, vec![a]);
        h.delete(a).unwrap();
        add(&mut h, "replacement");
        assert!(h.queue_snapshot().unwrap().ids.is_empty());
    }
    #[test]
    fn invalid_stale_and_missing_selection_keeps_queue_and_sensitive_records_private() {
        let mut h = history();
        let id = add(&mut h, "secret body");
        h.set_tags(id, vec!["sensitive".into()]).unwrap();
        let s = set(&mut h, vec![id]);
        for ids in [vec![0], vec![id,9999], vec![id; QUEUE_LIMIT+1]] {
            assert!(h.queue_replace(ids, s.revision, s.history_revision).is_err());
            assert_eq!(h.queue_snapshot().unwrap().ids, vec![id]);
        }
        assert!(h.queue_ticket(s.revision+1, s.history_revision).is_err());
        let wire = serde_json::to_string(&s).unwrap();
        assert!(!wire.contains("secret") && !wire.contains("sensitive"));
    }
    #[test]
    fn lost_history_evidence_blocks_paste_instead_of_guessing_ids() {
        let mut h = history();
        let id = add(&mut h, "queued"); let other = add(&mut h, "other");
        set(&mut h, vec![id]);
        for i in 0..140 { h.edit(other, format!("new {i}")).unwrap(); }
        let s = h.queue_snapshot().unwrap();
        assert!(s.requires_reset);
        assert_eq!(s.ids, vec![id]);
        assert_eq!(h.queue_ticket(s.revision,s.history_revision).unwrap_err().code,"queue_history_resync");
        let s = set(&mut h, vec![id]);
        assert!(!s.requires_reset);
    }
    #[test]
    fn abi_queue_rejects_stale_and_stopped_paste_without_consuming_or_writing_settings() {
        let _serial = crate::ABI_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let root = std::env::temp_dir().join(format!("TieZ-queue-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let invoke = |request: serde_json::Value| crate::dispatch(&serde_json::to_vec(&request).unwrap());
        let opened = invoke(serde_json::json!({"op":"open", "database_path":root.join("test.db")})).unwrap();
        assert_eq!(opened["snapshot"]["values"]["app.sequential_mode"], "false");
        assert_eq!(opened["snapshot"]["values"]["app.sequential_hotkey"], "Alt+V");
        let handle = opened["handle"].as_u64().unwrap();
        let id = invoke(serde_json::json!({"op":"history_add","handle":handle,"content_type":"text","content":"queued","tags":[]})).unwrap()["id"].as_i64().unwrap();
        let state = invoke(serde_json::json!({"op":"queue_snapshot","handle":handle})).unwrap();
        let state = invoke(serde_json::json!({"op":"queue_replace","handle":handle,"ids":[id,id],
            "expected_queue_revision":state["revision"],"expected_history_revision":state["history_revision"]})).unwrap();
        let mut request = serde_json::json!({"op":"queue_paste","handle":handle,
            "expected_queue_revision":state["revision"],"expected_history_revision":state["history_revision"],"expected_generation":0});
        assert!(invoke(request.clone()).is_err());
        request["expected_queue_revision"] = serde_json::json!(999);
        assert_eq!(invoke(request).unwrap_err().code,"queue_conflict");
        assert_eq!(invoke(serde_json::json!({"op":"queue_snapshot","handle":handle})).unwrap(),state);
        assert_eq!(invoke(serde_json::json!({"op":"snapshot","handle":handle})).unwrap()["snapshot"],opened["snapshot"]);
        assert!(invoke(serde_json::json!({"op":"queue_snapshot","handle":handle,"content":"forbidden"})).is_err());
        invoke(serde_json::json!({"op":"close","handle":handle})).unwrap();
        let reopened = invoke(serde_json::json!({"op":"open","database_path":root.join("test.db")})).unwrap();
        let handle = reopened["handle"].as_u64().unwrap();
        assert_eq!(invoke(serde_json::json!({"op":"queue_snapshot","handle":handle})).unwrap()["ids"],serde_json::json!([]));
        invoke(serde_json::json!({"op":"close","handle":handle})).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_capture_is_bounded_and_reports_overflow_without_discarding_old_items() {
        let mut q = PasteQueue::default();
        for id in 1..=QUEUE_LIMIT as i64 { q.captured(id); }
        q.captured(1001);
        let s = q.snapshot(0);
        assert_eq!(s.ids.len(), QUEUE_LIMIT);
        assert_eq!(s.ids[0],1);
        assert_eq!(s.last_error,Some("queue_full"));
    }
}
