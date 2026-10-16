use crate::{CoreError, CoreResult};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tiez_storage::database::{self, has_sensitive_tag};
use tiez_storage::domain::models::ClipboardEntry;
use tiez_storage::infrastructure::repository::clipboard_repo::{
    ClipboardRepository, SqliteClipboardRepository,
};

const SESSION_LIMIT: usize = 500;
const CHANGE_LIMIT: usize = 128;
const SCAN_LIMIT: u32 = 2000;
fn failure(message: impl ToString) -> CoreError {
    CoreError::new("history", message.to_string())
}
fn missing() -> CoreError {
    CoreError::new("not_found", "Clipboard entry was not found")
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryQuery {
    pub limit: u32,
    pub offset: u32,
    pub expected_revision: Option<u64>,
    pub search: String,
    pub content_type: Option<String>,
    pub tag: Option<String>,
    pub tag_only: bool,
    pub pinned_only: bool,
}
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
            expected_revision: None,
            search: String::new(),
            content_type: None,
            tag: None,
            tag_only: false,
            pinned_only: false,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct HistorySummary {
    pub id: i64,
    pub content_type: String,
    pub preview: String,
    pub source_app: String,
    pub timestamp: i64,
    pub is_pinned: bool,
    pub tags: Vec<String>,
    pub use_count: i32,
    pub is_external: bool,
}
impl From<ClipboardEntry> for HistorySummary {
    fn from(e: ClipboardEntry) -> Self {
        Self {
            id: e.id,
            content_type: e.content_type,
            preview: e.preview.chars().take(500).collect(),
            source_app: e.source_app,
            timestamp: e.timestamp,
            is_pinned: e.is_pinned,
            tags: e.tags,
            use_count: e.use_count,
            is_external: e.is_external,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct HistoryPage {
    pub revision: u64,
    pub items: Vec<HistorySummary>,
    pub next_offset: u32,
    pub has_more: bool,
    // All rows, not filtered match counts.
    pub stored_count: i64,
    pub session_count: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistoryChange {
    pub revision: u64,
    pub id: i64,
    pub replaced_id: Option<i64>,
    pub removed_ids: Vec<i64>,
}
#[derive(Debug, Serialize)]
pub struct HistoryChanges {
    pub revision: u64,
    pub resync_required: bool,
    pub changes: Vec<HistoryChange>,
    pub persistent_sync_pending: bool,
}
#[derive(Debug, Serialize)]
pub struct HistoryMutation {
    pub revision: u64,
    pub changed: bool,
    pub id: i64,
    pub replaced_id: Option<i64>,
    pub removed_ids: Vec<i64>,
    pub persistent_sync_pending: bool,
}
pub struct HistoryService {
    connection: Arc<Mutex<Connection>>,
    repository: SqliteClipboardRepository,
    data_dir: PathBuf,
    session: VecDeque<ClipboardEntry>,
    next_session_id: i64,
    revision: u64,
    changes: VecDeque<HistoryChange>,
    persistent_sync_pending: bool,
}
impl HistoryService {
    pub fn open(path: &Path) -> CoreResult<Self> {
        if !path.is_absolute() || !path.parent().is_some_and(Path::is_dir) {
            return Err(CoreError::new(
                "invalid_path",
                "An absolute database path with an existing parent is required",
            ));
        }
        let text = path
            .to_str()
            .ok_or_else(|| failure("Database path is not Unicode"))?;
        let connection = database::init_db(text).map_err(failure)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(failure)?;
        Ok(Self::from_connection(
            connection,
            path.parent().unwrap().into(),
        ))
    }
    fn from_connection(connection: Connection, data_dir: PathBuf) -> Self {
        let connection = Arc::new(Mutex::new(connection));
        Self {
            repository: SqliteClipboardRepository::new(connection.clone()),
            connection,
            data_dir,
            session: VecDeque::new(),
            next_session_id: -1,
            revision: 0,
            changes: VecDeque::new(),
            persistent_sync_pending: false,
        }
    }
    pub fn entry(&self, id: i64) -> CoreResult<ClipboardEntry> {
        if id < 0 {
            return self
                .session
                .iter()
                .find(|e| e.id == id)
                .cloned()
                .ok_or_else(missing);
        }
        self.repository
            .get_entry_by_id(id)
            .map_err(failure)?
            .ok_or_else(missing)
    }
    fn matches(&self, e: &ClipboardEntry, q: &HistoryQuery) -> CoreResult<bool> {
        if q.content_type
            .as_ref()
            .is_some_and(|kind| *kind != e.content_type)
            || q.tag.as_ref().is_some_and(|tag| !e.tags.contains(tag))
            || (q.pinned_only && !e.is_pinned)
        {
            return Ok(false);
        }
        let term = q.search.trim().to_lowercase();
        if term.is_empty() {
            return Ok(true);
        }
        if e.tags.iter().any(|tag| tag.to_lowercase().contains(&term)) {
            return Ok(true);
        }
        if q.tag_only {
            return Ok(false);
        }
        if e.content.to_lowercase().contains(&term) || e.source_app.to_lowercase().contains(&term) {
            return Ok(true);
        }
        if e.id > 0 && !has_sensitive_tag(&e.tags) {
            let c = self.connection.lock().map_err(failure)?;
            let mut stmt=c.prepare("SELECT ocr_text,qr_codes FROM clipboard_image_analysis WHERE entry_id=?1 AND content_hash=(SELECT content_hash FROM clipboard_history WHERE id=?1)").map_err(failure)?;
            let mut rows = stmt.query([e.id]).map_err(failure)?;
            if let Some(row) = rows.next().map_err(failure)? {
                let ocr: String = row.get(0).unwrap_or_default();
                let qr: String = row.get(1).unwrap_or_default();
                return Ok(ocr.to_lowercase().contains(&term) || qr.to_lowercase().contains(&term));
            }
        }
        Ok(false)
    }
    pub fn page(&self, q: HistoryQuery) -> CoreResult<HistoryPage> {
        if !(1..=200).contains(&q.limit) || q.offset > i32::MAX as u32 - SCAN_LIMIT {
            return Err(CoreError::new(
                "invalid_page",
                "Limit must be 1..200 and offset must fit SQLite paging",
            ));
        }
        if (q.offset > 0 && q.expected_revision.is_none())
            || q.expected_revision.is_some_and(|r| r != self.revision)
        {
            return Err(CoreError::new(
                "history_conflict",
                "History changed; restart paging from the first page",
            ));
        }
        let mut items = Vec::new();
        let (mut offset, mut scanned, mut count, mut has_more) = (q.offset, 0, 0, true);
        while count < q.limit && scanned < SCAN_LIMIT {
            let take = (q.limit - count).min(200).min(SCAN_LIMIT - scanned);
            let batch = self
                .repository
                .get_history(take as i32, offset as i32, None)
                .map_err(failure)?;
            let length = batch.len() as u32;
            offset += length;
            scanned += length;
            for e in batch {
                if self.matches(&e, &q)? {
                    items.push(e);
                    count += 1;
                }
            }
            if length < take {
                has_more = false;
                break;
            }
        }
        if q.offset == 0 {
            for e in &self.session {
                if self.matches(e, &q)? {
                    items.push(e.clone());
                }
            }
        }
        items.sort_by(|a, b| {
            b.is_pinned
                .cmp(&a.is_pinned)
                .then_with(|| b.pinned_order.cmp(&a.pinned_order))
                .then_with(|| b.timestamp.cmp(&a.timestamp))
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(HistoryPage {
            revision: self.revision,
            items: items.into_iter().map(Into::into).collect(),
            next_offset: offset,
            has_more,
            stored_count: self.repository.get_count().map_err(failure)?,
            session_count: self.session.len(),
        })
    }
    fn next_revision(&self) -> CoreResult<u64> {
        self.revision
            .checked_add(1)
            .ok_or_else(|| failure("History revision exhausted"))
    }
    fn committed(
        &mut self,
        revision: u64,
        id: i64,
        replaced_id: Option<i64>,
        removed_ids: Vec<i64>,
        persisted: bool,
    ) -> HistoryMutation {
        self.revision = revision;
        self.persistent_sync_pending |= persisted;
        self.changes.push_back(HistoryChange {
            revision,
            id,
            replaced_id,
            removed_ids: removed_ids.clone(),
        });
        while self.changes.len() > CHANGE_LIMIT {
            self.changes.pop_front();
        }
        HistoryMutation {
            revision,
            changed: true,
            id,
            replaced_id,
            removed_ids,
            persistent_sync_pending: self.persistent_sync_pending,
        }
    }
    fn unchanged(&self, id: i64) -> HistoryMutation {
        HistoryMutation {
            revision: self.revision,
            changed: false,
            id,
            replaced_id: None,
            removed_ids: Vec::new(),
            persistent_sync_pending: self.persistent_sync_pending,
        }
    }
    pub fn changes(&self, after: u64) -> CoreResult<HistoryChanges> {
        if after > self.revision {
            return Err(CoreError::new(
                "history_conflict",
                "Revision belongs to another history instance",
            ));
        }
        let oldest = self
            .changes
            .front()
            .map_or(self.revision, |c| c.revision.saturating_sub(1));
        Ok(HistoryChanges {
            revision: self.revision,
            resync_required: after < oldest,
            changes: self
                .changes
                .iter()
                .filter(|c| c.revision > after)
                .cloned()
                .collect(),
            persistent_sync_pending: self.persistent_sync_pending,
        })
    }

    fn verify_sensitive(connection: &Connection, id: i64) -> CoreResult<()> {
        #[cfg(not(feature = "portable"))]
        {
            let (content, preview, html): (String, String, Option<String>) = connection
                .query_row(
                    "SELECT content,preview,html_content FROM clipboard_history WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(failure)?;
            if !content.starts_with(database::ENCRYPT_PREFIX)
                || !preview.starts_with(database::ENCRYPT_PREFIX)
                || html.is_some_and(|value| !value.starts_with(database::ENCRYPT_PREFIX))
            {
                return Err(failure(
                    "Sensitive encryption failed; previous state preserved",
                ));
            }
        }
        #[cfg(feature = "portable")]
        let _ = (connection, id);
        Ok(())
    }
    fn persist(&self, entry: &ClipboardEntry) -> CoreResult<i64> {
        let mut entry = entry.clone();
        if entry.id > 0 {
            let old = self.entry(entry.id)?;
            if entry.tags.is_empty() {
                entry.tags = old.tags;
            }
        }
        let mut seen = BTreeSet::new();
        entry.tags = entry
            .tags
            .into_iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty() && seen.insert(tag.clone()))
            .collect();
        let mut connection = self.connection.lock().map_err(failure)?;
        let transaction = connection.transaction().map_err(failure)?;
        let id = self
            .repository
            .save_with_conn(&transaction, &entry, Some(&self.data_dir))
            .map_err(failure)?;
        if has_sensitive_tag(&entry.tags) {
            self.repository
                .encrypt_entry_with_conn(&transaction, id)
                .map_err(failure)?;
            Self::verify_sensitive(&transaction, id)?;
            transaction
                .execute(
                    "DELETE FROM clipboard_image_analysis WHERE entry_id=?1",
                    [id],
                )
                .map_err(failure)?;
        }
        transaction.commit().map_err(failure)?;
        Ok(id)
    }
    pub fn add_manual(
        &mut self,
        kind: String,
        content: String,
        html_content: Option<String>,
        tags: Vec<String>,
    ) -> CoreResult<HistoryMutation> {
        if !matches!(
            kind.as_str(),
            "text" | "code" | "url" | "rich_text" | "image" | "file" | "video"
        ) {
            return Err(CoreError::new(
                "invalid_value",
                "Unsupported clipboard content type",
            ));
        }
        let revision = self.next_revision()?;
        let e = ClipboardEntry {
            id: 0,
            content_type: kind,
            preview: content.chars().take(200).collect(),
            content,
            html_content,
            source_app: "Manual".into(),
            source_app_path: None,
            timestamp: now_ms(),
            is_pinned: false,
            tags,
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: true,
        };
        let id = self.persist(&e)?;
        Ok(self.committed(revision, id, None, Vec::new(), true))
    }
    /// Accepts persistence-stage input after capture/cleanup/privacy/dedup processing.
    /// This method does not read the OS clipboard or implement that pipeline.
    pub fn record_processed(
        &mut self,
        mut e: ClipboardEntry,
        persistent: bool,
    ) -> CoreResult<HistoryMutation> {
        let revision = self.next_revision()?;
        if persistent {
            let id = self.persist(&e)?;
            return Ok(self.committed(revision, id, None, Vec::new(), true));
        }
        if e.id < 0 {
            if let Some(old) = self.session.iter_mut().find(|old| old.id == e.id) {
                e.is_pinned = old.is_pinned;
                e.pinned_order = old.pinned_order;
                if e.tags.is_empty() {
                    e.tags = old.tags.clone();
                }
                e.use_count = old.use_count + 1;
                *old = e.clone();
                return Ok(self.committed(revision, e.id, None, Vec::new(), false));
            }
        }
        let id = self.next_session_id;
        self.next_session_id = id
            .checked_sub(1)
            .ok_or_else(|| failure("Session ID space exhausted"))?;
        e.id = id;
        self.session.push_back(e);
        let removed = if self.session.len() > SESSION_LIMIT {
            vec![self.session.pop_front().unwrap().id]
        } else {
            Vec::new()
        };
        Ok(self.committed(revision, id, None, removed, false))
    }
    pub fn pin(&mut self, id: i64, pinned: bool) -> CoreResult<HistoryMutation> {
        let mut e = self.entry(id)?;
        if e.is_pinned == pinned {
            return Ok(self.unchanged(id));
        }
        let revision = self.next_revision()?;
        if id < 0 && pinned {
            e.is_pinned = true;
            let next = self.persist(&e)?;
            self.session.retain(|e| e.id != id);
            return Ok(self.committed(revision, next, Some(id), Vec::new(), true));
        }
        if id < 0 {
            self.session
                .iter_mut()
                .find(|entry| entry.id == id)
                .unwrap()
                .is_pinned = pinned;
            return Ok(self.committed(revision, id, None, Vec::new(), false));
        }
        self.repository.toggle_pin(id, pinned).map_err(failure)?;
        Ok(self.committed(revision, id, None, Vec::new(), true))
    }
    pub fn edit(&mut self, id: i64, content: String) -> CoreResult<HistoryMutation> {
        let e = self.entry(id)?;
        if e.content == content && e.content_type != "rich_text" && e.html_content.is_none() {
            return Ok(self.unchanged(id));
        }
        let revision = self.next_revision()?;
        let preview = content.chars().take(500).collect::<String>();
        if id < 0 {
            let e = self.session.iter_mut().find(|e| e.id == id).unwrap();
            e.content = content;
            e.preview = preview;
            e.html_content = None;
            if e.content_type == "rich_text" {
                e.content_type = "text".into();
            }
        } else {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            self.repository
                .update_entry_content_with_conn(&transaction, id, &content, &preview)
                .map_err(failure)?;
            if has_sensitive_tag(&e.tags) {
                Self::verify_sensitive(&transaction, id)?;
                transaction
                    .execute(
                        "DELETE FROM clipboard_image_analysis WHERE entry_id=?1",
                        [id],
                    )
                    .map_err(failure)?;
            }
            transaction.commit().map_err(failure)?;
        }
        Ok(self.committed(revision, id, None, Vec::new(), id > 0))
    }
    pub fn delete(&mut self, id: i64) -> CoreResult<HistoryMutation> {
        self.entry(id)?;
        let revision = self.next_revision()?;
        if id > 0 {
            self.repository
                .delete(id, Some(&self.data_dir))
                .map_err(failure)?;
        } else {
            self.session.retain(|e| e.id != id);
        }
        Ok(self.committed(revision, id, None, vec![id], id > 0))
    }
    pub fn set_tags(&mut self, id: i64, tags: Vec<String>) -> CoreResult<HistoryMutation> {
        let mut e = self.entry(id)?;
        let mut seen = BTreeSet::new();
        let tags = tags
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && seen.insert(t.clone()))
            .collect::<Vec<_>>();
        if e.tags == tags {
            return Ok(self.unchanged(id));
        }
        let revision = self.next_revision()?;
        if id < 0 {
            e.tags = tags;
            let next = self.persist(&e)?;
            self.session.retain(|e| e.id != id);
            return Ok(self.committed(revision, next, Some(id), Vec::new(), true));
        }
        {
            let mut c = self.connection.lock().map_err(failure)?;
            let tx = c.transaction().map_err(failure)?;
            tx.execute("DELETE FROM entry_tags WHERE entry_id=?1", [id])
                .map_err(failure)?;
            for t in &tags {
                tx.execute(
                    "INSERT INTO entry_tags(entry_id,tag) VALUES (?1,?2)",
                    rusqlite::params![id, t],
                )
                .map_err(failure)?;
            }
            tx.execute("UPDATE clipboard_history SET tags=?1,sync_updated_at=?2,
                sync_updated_by=COALESCE((SELECT value FROM settings WHERE key='app.anon_id'),'') WHERE id=?3",
                rusqlite::params![serde_json::to_string(&tags).map_err(failure)?,now_ms(),id]).map_err(failure)?;
            if has_sensitive_tag(&tags) {
                self.repository
                    .encrypt_entry_with_conn(&tx, id)
                    .map_err(failure)?;
                Self::verify_sensitive(&tx, id)?;
                tx.execute(
                    "DELETE FROM clipboard_image_analysis WHERE entry_id=?1",
                    [id],
                )
                .map_err(failure)?;
            } else if has_sensitive_tag(&e.tags) {
                self.repository
                    .decrypt_entry_with_conn(&tx, id)
                    .map_err(failure)?;
            }
            tx.commit().map_err(failure)?;
        }
        Ok(self.committed(revision, id, None, Vec::new(), true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> HistoryService {
        let c = Connection::open_in_memory().unwrap();
        tiez_storage::infrastructure::repository::migrations::run_migrations(&c).unwrap();
        database::seed_defaults(&c).unwrap();
        HistoryService::from_connection(c, std::env::temp_dir())
    }
    fn add(s: &mut HistoryService, text: &str) -> i64 {
        s.add_manual("text".into(), text.into(), None, Vec::new())
            .unwrap()
            .id
    }
    fn processed(text: &str) -> ClipboardEntry {
        ClipboardEntry {
            id: 0,
            content_type: "text".into(),
            content: text.into(),
            html_content: None,
            source_app: "test".into(),
            source_app_path: None,
            timestamp: 1,
            preview: text.into(),
            is_pinned: false,
            tags: Vec::new(),
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: true,
        }
    }
    #[test]
    fn pages_are_stable_bounded_and_keep_full_content_separate() {
        let mut s = service();
        let text = " 中 \r\n".repeat(1000);
        let first = add(&mut s, &text);
        let second = add(&mut s, "new");
        let p = s
            .page(HistoryQuery {
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(p.items[0].id, second);
        assert!(p.has_more);
        let next = s
            .page(HistoryQuery {
                limit: 1,
                offset: p.next_offset,
                expected_revision: Some(p.revision),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.items[0].id, first);
        assert_eq!(next.items[0].preview.chars().count(), 200);
        assert_eq!(s.entry(first).unwrap().content, text);
        add(&mut s, "changed");
        assert_eq!(
            s.page(HistoryQuery {
                offset: 1,
                expected_revision: Some(p.revision),
                ..Default::default()
            })
            .unwrap_err()
            .code,
            "history_conflict"
        );
        assert!(s
            .page(HistoryQuery {
                limit: 201,
                ..Default::default()
            })
            .is_err());
    }
    #[test]
    fn session_cap_and_promotion_preserve_ids_on_failure() {
        let mut s = service();
        let first = s.record_processed(processed("first"), false).unwrap().id;
        for _ in 0..500 {
            s.record_processed(processed("other"), false).unwrap();
        }
        assert_eq!(s.session.len(), 500);
        assert!(s.entry(first).is_err());
        let id = s.session.front().unwrap().id;
        let before = s.revision;
        s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON clipboard_history BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(s.pin(id, true).is_err());
        assert_eq!(s.revision, before);
        assert!(!s.entry(id).unwrap().is_pinned);
        s.connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_insert")
            .unwrap();
        let saved = s.pin(id, true).unwrap();
        assert!(saved.id > 0);
        assert_eq!(saved.replaced_id, Some(id));
        assert!(s.entry(id).is_err());
        assert!(s.entry(saved.id).unwrap().is_pinned);
        assert_eq!(s.session.len(), 499);
        let id = s.session.front().unwrap().id;
        let tagged = s.set_tags(id, vec!["work".into()]).unwrap();
        assert!(tagged.id > 0);
        assert_eq!(tagged.replaced_id, Some(id));
    }
    #[test]
    fn sensitive_tags_encrypt_purge_ocr_and_roll_back_as_one_unit() {
        let mut s = service();
        let id = add(&mut s, " 私密 \n");
        s.connection.lock().unwrap().execute("INSERT INTO clipboard_image_analysis(entry_id,content_hash,ocr_text,analyzed_at) SELECT id,content_hash,'private OCR',1 FROM clipboard_history WHERE id=?1",[id]).unwrap();
        s.set_tags(id, vec!["密码".into(), "密码".into(), " ".into()])
            .unwrap();
        assert_eq!(s.entry(id).unwrap().content, " 私密 \n");
        assert_eq!(s.entry(id).unwrap().tags, vec!["密码"]);
        let raw: String = s
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT content FROM clipboard_history WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        #[cfg(all(windows, not(feature = "portable")))]
        assert!(raw.starts_with("dpapi:"));
        #[cfg(feature = "portable")]
        assert_eq!(raw, " 私密 \n");
        let count: i64 = s
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM clipboard_image_analysis", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let before = s.revision;
        s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_update BEFORE UPDATE ON clipboard_history BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(s.set_tags(id, vec!["work".into()]).is_err());
        assert_eq!(s.revision, before);
        assert_eq!(s.entry(id).unwrap().tags, vec!["密码"]);
        s.connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_update")
            .unwrap();
        s.set_tags(id, vec!["work".into()]).unwrap();
        let raw: String = s
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT content FROM clipboard_history WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, " 私密 \n");
    }
    #[test]
    fn noops_literal_search_and_change_feed_are_consistent() {
        let mut s = service();
        let id = add(&mut s, "100%_中文");
        assert!(!s.edit(id, "100%_中文".into()).unwrap().changed);
        assert_eq!(s.revision, 1);
        let p = s
            .page(HistoryQuery {
                search: "%_中".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(p.items.len(), 1);
        s.delete(id).unwrap();
        let before = s.revision;
        assert!(s.delete(id).is_err());
        assert_eq!(s.revision, before);
        assert_eq!(s.changes(1).unwrap().changes[0].removed_ids, vec![id]);
        for _ in 0..129 {
            add(&mut s, "row");
        }
        assert!(s.changes(0).unwrap().resync_required);
        assert_eq!(s.changes(0).unwrap().changes.len(), 128);
    }
    #[test]
    fn ocr_cleanup_failure_rolls_back_completed_encryption_and_tags() {
        let mut service = service();
        let id = add(&mut service, "secret fixture");
        let before = service.revision;
        service.connection.lock().unwrap().execute(
            "INSERT INTO clipboard_image_analysis(entry_id,content_hash,ocr_text,analyzed_at) SELECT id,content_hash,'fixture OCR',1 FROM clipboard_history WHERE id=?1", [id]).unwrap();
        service.connection.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_cleanup BEFORE DELETE ON clipboard_image_analysis BEGIN SELECT RAISE(ABORT,'injected cleanup'); END;").unwrap();
        assert!(service.set_tags(id, vec!["密码".into()]).is_err());
        assert_eq!(service.revision, before);
        let entry = service.entry(id).unwrap();
        assert_eq!(entry.content, "secret fixture");
        assert!(entry.tags.is_empty());
        let connection = service.connection.lock().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT content FROM clipboard_history WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "secret fixture");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM clipboard_image_analysis", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn plain_edit_removes_rich_format_even_when_text_is_unchanged() {
        let mut service = service();
        let id = service
            .add_manual(
                "rich_text".into(),
                " body ".into(),
                Some("<b> body </b>".into()),
                Vec::new(),
            )
            .unwrap()
            .id;
        assert!(service.edit(id, " body ".into()).unwrap().changed);
        let entry = service.entry(id).unwrap();
        assert_eq!(entry.content_type, "text");
        assert!(entry.html_content.is_none());
        let mut rich = processed(" body ");
        rich.content_type = "rich_text".into();
        rich.html_content = Some("<b> body </b>".into());
        let id = service.record_processed(rich, false).unwrap().id;
        assert!(service.edit(id, " body ".into()).unwrap().changed);
        assert_eq!(service.entry(id).unwrap().content_type, "text");
        assert!(service.entry(id).unwrap().html_content.is_none());
    }
    #[test]
    fn empty_filtered_scan_page_still_advances_to_later_matches() {
        let service = service();
        service.connection.lock().unwrap().execute_batch(
            "WITH RECURSIVE rows(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM rows WHERE n<2501)
             INSERT INTO clipboard_history(id,content_type,content,source_app,timestamp,preview)
             SELECT n,'text',CASE WHEN n=1 THEN 'needle' ELSE 'other' END,'fixture',n,'fixture' FROM rows;").unwrap();
        let page = service
            .page(HistoryQuery {
                limit: 1,
                search: "needle".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(page.items.is_empty());
        assert!(page.has_more);
        let next = service
            .page(HistoryQuery {
                limit: 1,
                offset: page.next_offset,
                expected_revision: Some(page.revision),
                search: "needle".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.items[0].id, 1);
    }
}
