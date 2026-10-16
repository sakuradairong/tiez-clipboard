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
const CLEAR_FILTER: &str = "is_pinned = 0 AND NOT EXISTS
    (SELECT 1 FROM entry_tags WHERE entry_id = clipboard_history.id)";
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
#[derive(Debug, Serialize)]
pub struct HistoryClearPreview {
    pub revision: u64,
    pub stored_count: i64,
    pub session_count: usize,
}
#[derive(Debug, Serialize)]
pub struct HistoryClearResult {
    pub revision: u64,
    pub changed: bool,
    pub removed_count: usize,
    pub persistent_sync_pending: bool,
    pub compaction_error: Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct HistoryImagePreview {
    pub id: i64,
    pub revision: u64,
    #[serde(flatten)]
    pub raster: tiez_content::preview::RasterPreview,
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
        self.persist_with_limit(entry, false).map(|result| result.0)
    }
    fn persist_with_limit(
        &self,
        entry: &ClipboardEntry,
        enforce_limit: bool,
    ) -> CoreResult<(i64, Vec<i64>)> {
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
        let cleanup = if enforce_limit {
            Some(
                tiez_storage::retention::prune(&self.repository, &transaction, &self.data_dir)
                    .map_err(failure)?,
            )
        } else {
            None
        };
        transaction.commit().map_err(failure)?;
        let removed = if let Some(cleanup) = cleanup {
            cleanup
                .finish(&self.repository, &connection, &self.data_dir)
                .map_err(failure)?
        } else {
            Vec::new()
        };
        Ok((id, removed))
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
    /// Applies capture dedup, persistence/retention and session removals as one
    /// accepted history revision. Failed persistence never removes session rows.
    pub fn record_capture(
        &mut self,
        mut entry: ClipboardEntry,
        persistent: bool,
        deduplicate: bool,
        recent_self_copy: bool,
    ) -> CoreResult<HistoryMutation> {
        let rich_equivalent = |old: &ClipboardEntry| {
            entry.content_type != "rich_text"
                || old.content_type != "rich_text"
                || match (entry.html_content.as_deref(), old.html_content.as_deref()) {
                    (None, None) => true,
                    (Some(a), Some(b)) => {
                        a.trim().replace("\r\n", "\n") == b.trim().replace("\r\n", "\n")
                    }
                    _ => false,
                }
                || recent_self_copy
                    && database::calc_text_hash(&old.content)
                        == database::calc_text_hash(&entry.content)
        };
        let matches = if deduplicate {
            let hash = database::calc_text_hash(&entry.content);
            let image_hash = if entry.content_type == "image" {
                database::calc_image_hash(&entry.content)
            } else {
                None
            };
            self.session
                .iter()
                .filter(|old| {
                    let image = entry.content_type == "image"
                        && old.content_type == "image"
                        && image_hash.is_some()
                        && database::calc_image_hash(&old.content) == image_hash;
                    image
                        || (old.content == entry.content
                            || database::calc_text_hash(&old.content) == hash)
                            && rich_equivalent(old)
                })
                .map(|old| old.id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if persistent && deduplicate {
            let connection = self.connection.lock().map_err(failure)?;
            let kinds: &[&str] = if entry.content_type == "rich_text" {
                &["rich_text", "text", "code", "url"]
            } else {
                &[&entry.content_type]
            };
            let mut found = None;
            for kind in kinds {
                if let Some(id) = self
                    .repository
                    .find_by_content_with_conn(&connection, &entry.content, Some(kind))
                    .map_err(failure)?
                {
                    let old = self
                        .repository
                        .get_entry_by_id_with_conn(&connection, id)
                        .map_err(failure)?
                        .ok_or_else(missing)?;
                    if rich_equivalent(&old) {
                        found = Some(id);
                        break;
                    }
                }
            }
            entry.id = found.unwrap_or(0);
        } else {
            entry.id = 0;
        }
        let revision = self.next_revision()?;
        if persistent {
            let (id, mut removed) = self.persist_with_limit(&entry, true)?;
            self.session.retain(|old| !matches.contains(&old.id));
            let replaced = matches.last().copied();
            removed.extend(matches.into_iter().filter(|old| Some(*old) != replaced));
            return Ok(self.committed(revision, id, replaced, removed, true));
        }
        let reused = matches.last().copied();
        let id = if let Some(id) = reused {
            let old = self
                .session
                .iter()
                .find(|old| old.id == id)
                .ok_or_else(missing)?;
            entry.is_pinned = old.is_pinned;
            entry.pinned_order = old.pinned_order;
            if entry.tags.is_empty() {
                entry.tags = old.tags.clone();
            }
            entry.use_count = old.use_count.saturating_add(1);
            id
        } else {
            let id = self.next_session_id;
            self.next_session_id = id
                .checked_sub(1)
                .ok_or_else(|| failure("Session ID space exhausted"))?;
            id
        };
        entry.id = id;
        // Reinsert at the newest position, retaining the stable negative identity.
        self.session.retain(|old| !matches.contains(&old.id));
        self.session.push_back(entry);
        let mut removed = matches
            .into_iter()
            .filter(|old| *old != id)
            .collect::<Vec<_>>();
        if self.session.len() > SESSION_LIMIT {
            removed.push(self.session.pop_front().unwrap().id);
        }
        Ok(self.committed(revision, id, None, removed, false))
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
    /// Deletes an explicit selection as one revision. Session rows and attachments remain
    /// untouched until the entire persistent transaction has committed.
    pub fn delete_many(
        &mut self,
        ids: Vec<i64>,
        expected_revision: u64,
    ) -> CoreResult<HistoryMutation> {
        if expected_revision != self.revision {
            return Err(CoreError::new(
                "history_conflict",
                "History changed; refresh before deleting",
            ));
        }
        if ids.len() > 1000 || ids.contains(&0) {
            return Err(CoreError::new(
                "invalid_batch",
                "Select at most 1000 nonzero entry IDs",
            ));
        }
        let ids: BTreeSet<_> = ids.into_iter().collect();
        let Some(first) = ids.first().copied() else {
            return Ok(self.unchanged(0));
        };
        for id in &ids {
            self.entry(*id)?;
        }
        let revision = self.next_revision()?;
        let stored: Vec<_> = ids.iter().copied().filter(|id| *id > 0).collect();
        let persisted = !stored.is_empty();
        {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            let mut cleanup = tiez_storage::retention::delete_selected(
                &self.repository,
                &transaction,
                &self.data_dir,
                stored,
            )
            .map_err(failure)?;
            cleanup.include_session(
                &self.repository,
                &self.data_dir,
                self.session.iter().filter(|entry| ids.contains(&entry.id)),
                self.session.iter().filter(|entry| !ids.contains(&entry.id)),
            );
            transaction.commit().map_err(failure)?;
            cleanup
                .finish(&self.repository, &connection, &self.data_dir)
                .map_err(failure)?;
        }
        self.session.retain(|entry| !ids.contains(&entry.id));
        Ok(self.committed(revision, first, None, ids.into_iter().collect(), persisted))
    }
    pub fn image_preview(
        &self,
        id: i64,
        expected_revision: u64,
        allow_sensitive: bool,
        max_edge: u32,
    ) -> CoreResult<HistoryImagePreview> {
        if expected_revision != self.revision {
            return Err(CoreError::new(
                "history_conflict",
                "History changed; refresh before previewing",
            ));
        }
        let entry = self.entry(id)?;
        if has_sensitive_tag(&entry.tags) && !allow_sensitive {
            return Err(CoreError::new(
                "sensitive_confirmation",
                "Reveal sensitive content before previewing",
            ));
        }
        if entry.content_type != "image" {
            return Err(CoreError::new(
                "unsupported_preview",
                "Entry is not an image",
            ));
        }
        let mut roots = tiez_content::clipboard_image_roots(&self.data_dir);
        if entry.is_external {
            let path = Path::new(&entry.content);
            if path.is_absolute() {
                if let Some(parent) = path.parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }
        let raster =
            tiez_content::preview::raster(&entry.content, &roots, max_edge).map_err(|code| {
                CoreError::new(
                    code,
                    "Image preview could not be prepared; original content unchanged",
                )
            })?;
        Ok(HistoryImagePreview {
            id,
            revision: self.revision,
            raster,
        })
    }
    pub fn clear_preview(&self) -> CoreResult<HistoryClearPreview> {
        let connection = self.connection.lock().map_err(failure)?;
        let stored_count = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM clipboard_history WHERE {CLEAR_FILTER}"),
                [],
                |r| r.get(0),
            )
            .map_err(failure)?;
        Ok(HistoryClearPreview {
            revision: self.revision,
            stored_count,
            session_count: self
                .session
                .iter()
                .filter(|entry| !entry.is_pinned && entry.tags.is_empty())
                .count(),
        })
    }
    /// The preview revision authorizes precisely this history state, independently of UI filters.
    /// A clear invalidates the change feed rather than returning an unbounded list of deleted IDs.
    pub fn clear(&mut self, expected_revision: u64) -> CoreResult<HistoryClearResult> {
        if expected_revision != self.revision {
            return Err(CoreError::new(
                "history_conflict",
                "History changed; review the clear preview again",
            ));
        }
        let removable = |entry: &ClipboardEntry| !entry.is_pinned && entry.tags.is_empty();
        let session_count = self.session.iter().filter(|entry| removable(entry)).count();
        let (revision, removed_count, persisted, compaction_error) = {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            let stored = {
                let mut statement = transaction
                    .prepare(&format!(
                        "SELECT id FROM clipboard_history WHERE {CLEAR_FILTER} ORDER BY id"
                    ))
                    .map_err(failure)?;
                let rows = statement
                    .query_map([], |row| row.get::<_, i64>(0))
                    .map_err(failure)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(failure)?
            };
            let removed_count = stored.len() + session_count;
            if removed_count == 0 {
                transaction.rollback().map_err(failure)?;
                return Ok(HistoryClearResult {
                    revision: self.revision,
                    changed: false,
                    removed_count: 0,
                    persistent_sync_pending: self.persistent_sync_pending,
                    compaction_error: None,
                });
            }
            let revision = self.next_revision()?;
            let persisted = !stored.is_empty();
            let mut cleanup = tiez_storage::retention::delete_selected(
                &self.repository,
                &transaction,
                &self.data_dir,
                stored,
            )
            .map_err(failure)?;
            cleanup.include_session(
                &self.repository,
                &self.data_dir,
                self.session.iter().filter(|entry| removable(entry)),
                self.session.iter().filter(|entry| !removable(entry)),
            );
            transaction.commit().map_err(failure)?;
            cleanup
                .finish(&self.repository, &connection, &self.data_dir)
                .map_err(failure)?;
            // Like the legacy clear, attempt to reclaim database space after commit.
            // This optional maintenance must never turn an accepted deletion into an error.
            let compaction_error = if persisted {
                connection
                    .execute_batch("VACUUM;")
                    .err()
                    .map(|_| "history_compaction")
            } else {
                None
            };
            (revision, removed_count, persisted, compaction_error)
        };
        self.session.retain(|entry| !removable(entry));
        self.revision = revision;
        self.persistent_sync_pending |= persisted;
        // changes() uses the current revision as its floor when this deque is empty.
        // Clients older than the clear must resync, including after subsequent edits.
        self.changes.clear();
        Ok(HistoryClearResult {
            revision,
            changed: true,
            removed_count,
            persistent_sync_pending: self.persistent_sync_pending,
            compaction_error,
        })
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
    #[test]
    fn capture_dedup_keeps_pinned_tags_and_disabled_mode_keeps_copies() {
        let mut s = service();
        let id = add(&mut s, "  captured  ");
        s.pin(id, true).unwrap();
        s.set_tags(id, vec!["work".into()]).unwrap();
        assert_eq!(
            s.record_capture(processed("  captured  "), true, true, false)
                .unwrap()
                .id,
            id
        );
        let entry = s.entry(id).unwrap();
        assert!(entry.is_pinned);
        assert_eq!(entry.tags, vec!["work"]);
        assert_eq!(entry.use_count, 1);
        assert_ne!(
            s.record_capture(processed("  captured  "), true, false, false)
                .unwrap()
                .id,
            id
        );
    }
    #[test]
    fn rich_capture_identity_includes_original_html() {
        let mut s = service();
        let mut a = processed("rich");
        a.content_type = "rich_text".into();
        a.html_content = Some("<b>rich</b>".into());
        let mut b = a.clone();
        b.html_content = Some("<i>rich</i>".into());
        let first = s.record_capture(a.clone(), true, true, false).unwrap().id;
        let second = s.record_capture(b, true, true, false).unwrap().id;
        assert_ne!(first, second);
        assert_eq!(s.record_capture(a, true, true, false).unwrap().id, first);
        assert_eq!(
            s.entry(second).unwrap().html_content.as_deref(),
            Some("<i>rich</i>")
        );
    }
    #[test]
    fn session_capture_reuses_identity_and_promotes_after_persistence_changes() {
        let mut s = service();
        let first = s
            .record_capture(processed("session"), false, true, false)
            .unwrap()
            .id;
        assert!(first < 0);
        assert_eq!(
            s.record_capture(processed("session"), false, true, false)
                .unwrap()
                .id,
            first
        );
        assert_eq!(s.entry(first).unwrap().use_count, 1);
        let second = s
            .record_capture(processed("session"), false, false, false)
            .unwrap()
            .id;
        let saved = s
            .record_capture(processed("session"), true, true, false)
            .unwrap();
        assert!(saved.id > 0);
        assert_eq!(saved.replaced_id, Some(second));
        assert!(saved.removed_ids.contains(&first));
        let page = s.page(Default::default()).unwrap();
        assert_eq!(page.session_count, 0);
        assert_eq!(page.stored_count, 1);
    }
    #[test]
    fn capture_retention_failure_preserves_session_and_revision() {
        let mut s = service();
        let id = s
            .record_capture(processed("session"), false, true, false)
            .unwrap()
            .id;
        add(&mut s, "old stored");
        let revision = s.revision;
        {
            let c = s.connection.lock().unwrap();
            c.execute(
                "UPDATE settings SET value='1' WHERE key='app.persistent_limit'",
                [],
            )
            .unwrap();
            c.execute_batch("CREATE TRIGGER reject_capture_prune BEFORE DELETE ON clipboard_history BEGIN SELECT RAISE(ABORT,'synthetic retention failure'); END").unwrap();
        }
        assert!(s
            .record_capture(processed("session"), true, true, false)
            .is_err());
        assert_eq!(s.revision, revision);
        assert!(s.entry(id).is_ok());
        let page = s.page(Default::default()).unwrap();
        assert_eq!(page.stored_count, 1);
        assert_eq!(page.session_count, 1);
    }
    #[test]
    fn capture_limit_preserves_protected_rows_and_publishes_removals() {
        let mut s = service();
        let pinned = add(&mut s, "pinned");
        s.pin(pinned, true).unwrap();
        let tagged = add(&mut s, "tagged");
        s.set_tags(tagged, vec!["work".into()]).unwrap();
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE settings SET value='1' WHERE key='app.persistent_limit'",
                [],
            )
            .unwrap();
        let old = s
            .record_capture(processed("first"), true, true, false)
            .unwrap()
            .id;
        let next = s
            .record_capture(processed("second"), true, true, false)
            .unwrap();
        assert!(next.removed_ids.contains(&old));
        assert!(s.entry(old).is_err());
        assert!(s.entry(pinned).is_ok());
        assert!(s.entry(tagged).is_ok());
        assert_eq!(s.page(Default::default()).unwrap().stored_count, 3);
        assert!(next.persistent_sync_pending);
    }
    #[test]
    fn sensitive_capture_persists_encrypted_fields() {
        let mut s = service();
        let mut e = processed("user@example.com");
        e.tags = vec!["sensitive".into()];
        let saved = s.record_capture(e, true, true, false).unwrap();
        assert_eq!(s.entry(saved.id).unwrap().content, "user@example.com");
        #[cfg(not(feature = "portable"))]
        assert!(s
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT content FROM clipboard_history WHERE id=?1",
                [saved.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap()
            .starts_with(database::ENCRYPT_PREFIX));
    }

    #[test]
    fn batch_delete_mixed_ids_deduplicates_and_publishes_one_revision() {
        let mut s = service();
        let stored = add(&mut s, "batch stored");
        s.set_tags(stored, vec!["work".into()]).unwrap();
        let session = s
            .record_capture(processed("batch session"), false, true, false)
            .unwrap()
            .id;
        let before = s.revision;
        let result = s
            .delete_many(vec![stored, session, stored], before)
            .unwrap();
        assert_eq!(result.revision, before + 1);
        assert_eq!(result.removed_ids, vec![session, stored]);
        assert!(result.persistent_sync_pending);
        assert!(s.entry(stored).is_err() && s.entry(session).is_err());
        let changes = s.changes(before).unwrap();
        assert_eq!(changes.changes.len(), 1);
        assert_eq!(changes.changes[0].removed_ids, result.removed_ids);
        let c = s.connection.lock().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM entry_tags WHERE entry_id=?1",
                [stored],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn batch_delete_rejects_stale_missing_zero_and_oversize_without_mutation() {
        let mut s = service();
        let id = add(&mut s, "keep");
        let revision = s.revision;
        assert_eq!(
            s.delete_many(vec![id], revision - 1).unwrap_err().code,
            "history_conflict"
        );
        assert_eq!(
            s.delete_many(vec![id, i64::MAX], revision)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            s.delete_many(vec![id, 0], revision).unwrap_err().code,
            "invalid_batch"
        );
        assert_eq!(
            s.delete_many(vec![id; 1001], revision).unwrap_err().code,
            "invalid_batch"
        );
        assert!(!s.delete_many(Vec::new(), revision).unwrap().changed);
        assert!(s.entry(id).is_ok());
        assert_eq!(s.revision, revision);
        assert!(s.changes(revision).unwrap().changes.is_empty());
    }
    #[test]
    fn batch_session_only_does_not_request_persistent_sync() {
        let mut s = service();
        let id = s
            .record_capture(processed("session only"), false, true, false)
            .unwrap()
            .id;
        let result = s.delete_many(vec![id], s.revision).unwrap();
        assert!(!result.persistent_sync_pending);
        assert!(s.entry(id).is_err());
    }
    #[test]
    fn batch_later_delete_failure_keeps_rows_session_and_tombstones() {
        let mut s = service();
        let first = add(&mut s, "first");
        let second = add(&mut s, "second");
        let session = s
            .record_capture(processed("session"), false, true, false)
            .unwrap()
            .id;
        let revision = s.revision;
        s.connection.lock().unwrap().execute_batch(&format!("CREATE TRIGGER reject_batch BEFORE DELETE ON clipboard_history WHEN OLD.id={second} BEGIN SELECT RAISE(ABORT,'batch fixture'); END;")).unwrap();
        assert!(s
            .delete_many(vec![first, second, session], revision)
            .is_err());
        for id in [first, second, session] {
            assert!(s.entry(id).is_ok());
        }
        assert_eq!(s.revision, revision);
        assert!(s.changes(revision).unwrap().changes.is_empty());
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn batch_failed_commit_keeps_session_and_database() {
        let mut s = service();
        let stored = add(&mut s, "stored");
        let session = s
            .record_capture(processed("session"), false, true, false)
            .unwrap()
            .id;
        let revision = s.revision;
        s.connection.lock().unwrap().execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE batch_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE batch_child(id INTEGER REFERENCES batch_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER reject_batch_commit AFTER DELETE ON clipboard_history BEGIN INSERT INTO batch_child VALUES(999); END;").unwrap();
        assert!(s.delete_many(vec![stored, session], revision).is_err());
        assert!(s.entry(stored).is_ok() && s.entry(session).is_ok());
        assert_eq!(s.revision, revision);
    }

    #[test]
    fn batch_attachments_keep_surviving_session_refs_then_clean_last_reference() {
        let mut s = service();
        let root =
            std::env::temp_dir().join(format!("TieZ-batch-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(root.join("attachments")).unwrap();
        let path = root.join("attachments/shared.png");
        std::fs::write(&path, b"fixture").unwrap();
        s.data_dir = root.clone();
        let mut entry = processed(path.to_str().unwrap());
        entry.content_type = "image".into();
        entry.is_external = true;
        let session = s.record_capture(entry, false, false, false).unwrap().id;
        s.connection.lock().unwrap().execute("INSERT INTO clipboard_history
            (id,content_type,content,source_app,timestamp,preview,content_hash,content_hash_version,tags,is_external)
            VALUES (1,'image',?1,'fixture',1,'image',1,2,'[]',1)", [path.to_str().unwrap()]).unwrap();
        s.delete_many(vec![1], s.revision).unwrap();
        assert!(
            path.exists(),
            "Surviving session still references attachment"
        );
        assert!(s.entry(session).is_ok());
        s.delete_many(vec![session], s.revision).unwrap();
        assert!(
            !path.exists(),
            "Last session attachment reference was not cleaned"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn batch_failed_commit_never_cleans_selected_session_attachment() {
        let mut s = service();
        let root = std::env::temp_dir().join(format!(
            "TieZ-batch-rollback-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(root.join("attachments")).unwrap();
        let path = root.join("attachments/session.png");
        std::fs::write(&path, b"fixture").unwrap();
        s.data_dir = root.clone();
        let mut entry = processed(path.to_str().unwrap());
        entry.content_type = "image".into();
        entry.is_external = true;
        let session = s.record_capture(entry, false, false, false).unwrap().id;
        let stored = add(&mut s, "must roll back");
        let revision = s.revision;
        s.connection.lock().unwrap().execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE file_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE file_child(id INTEGER REFERENCES file_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_file_commit AFTER DELETE ON clipboard_history BEGIN INSERT INTO file_child VALUES(999); END;").unwrap();
        assert!(s.delete_many(vec![stored, session], revision).is_err());
        assert!(path.exists());
        assert!(s.entry(stored).is_ok() && s.entry(session).is_ok());
        assert_eq!(s.revision, revision);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clear_preview_is_read_only_and_preserves_pinned_and_tagged_history() {
        let mut s = service();
        let ordinary = add(&mut s, "ordinary");
        let pinned = add(&mut s, "pinned");
        s.pin(pinned, true).unwrap();
        let tagged = add(&mut s, "tagged");
        s.set_tags(tagged, vec!["work".into()]).unwrap();
        let session = s
            .record_capture(processed("session ordinary"), false, true, false)
            .unwrap()
            .id;
        let mut protected = processed("session pinned");
        protected.is_pinned = true;
        let session_pinned = s.record_capture(protected, false, true, false).unwrap().id;
        let mut protected = processed("session tagged");
        protected.tags = vec!["work".into()];
        let session_tagged = s.record_capture(protected, false, true, false).unwrap().id;
        let revision = s.revision;
        let preview = s.clear_preview().unwrap();
        assert_eq!(
            (
                preview.revision,
                preview.stored_count,
                preview.session_count
            ),
            (revision, 1, 1)
        );
        assert_eq!(s.revision, revision);
        assert!(s.changes(revision).unwrap().changes.is_empty());
        let cleared = s.clear(preview.revision).unwrap();
        assert!(cleared.changed && cleared.persistent_sync_pending);
        assert_eq!((cleared.removed_count, cleared.revision), (2, revision + 1));
        assert!(s.entry(ordinary).is_err() && s.entry(session).is_err());
        for id in [pinned, tagged, session_pinned, session_tagged] {
            assert!(s.entry(id).is_ok());
        }
        assert!(s.changes(revision).unwrap().resync_required);
        let empty = s.clear_preview().unwrap();
        assert_eq!(empty.stored_count + empty.session_count as i64, 0);
        assert!(!s.clear(empty.revision).unwrap().changed);
        assert_eq!(s.revision, cleared.revision);
    }
    #[test]
    fn clear_delete_failure_rolls_back_every_row_session_tombstone_and_revision() {
        let mut s = service();
        let first = add(&mut s, "clear first");
        let second = add(&mut s, "clear second");
        let session = s
            .record_capture(processed("clear session"), false, true, false)
            .unwrap()
            .id;
        let revision = s.revision;
        s.connection.lock().unwrap().execute_batch(&format!(
            "CREATE TRIGGER reject_clear BEFORE DELETE ON clipboard_history WHEN OLD.id={second} BEGIN SELECT RAISE(ABORT,'clear fixture'); END;" )).unwrap();
        assert!(s.clear(revision).is_err());
        for id in [first, second, session] {
            assert!(s.entry(id).is_ok());
        }
        assert_eq!(s.revision, revision);
        assert!(!s.changes(revision).unwrap().resync_required);
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn clear_failed_commit_preserves_session_attachment_and_database() {
        let mut s = service();
        let root = std::env::temp_dir().join(format!(
            "TieZ-clear-rollback-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(root.join("attachments")).unwrap();
        let path = root.join("attachments/session.png");
        std::fs::write(&path, b"fixture").unwrap();
        s.data_dir = root.clone();
        let mut entry = processed(path.to_str().unwrap());
        entry.content_type = "image".into();
        entry.is_external = true;
        let session = s.record_capture(entry, false, false, false).unwrap().id;
        let stored = add(&mut s, "clear roll back");
        let revision = s.revision;
        s.connection.lock().unwrap().execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE clear_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE clear_child(id INTEGER REFERENCES clear_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER reject_clear_commit AFTER DELETE ON clipboard_history BEGIN INSERT INTO clear_child VALUES(999); END;").unwrap();
        assert!(s.clear(revision).is_err());
        assert!(path.exists() && s.entry(session).is_ok() && s.entry(stored).is_ok());
        assert_eq!(s.revision, revision);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn clear_has_no_selection_limit_and_keeps_change_payload_bounded() {
        let mut s = service();
        s.connection.lock().unwrap().execute_batch("WITH RECURSIVE rows(n) AS
            (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<1205)
            INSERT INTO clipboard_history(id,content_type,content,source_app,timestamp,preview,content_hash,tags)
            SELECT n,'text','clear '||n,'fixture',n,'clear',n,'[]' FROM rows;
            UPDATE clipboard_history SET is_pinned=1 WHERE id=1;
            INSERT INTO entry_tags(entry_id,tag) VALUES(2,'work');").unwrap();
        let preview = s.clear_preview().unwrap();
        assert_eq!(preview.stored_count, 1203);
        let cleared = s.clear(preview.revision).unwrap();
        assert_eq!(cleared.removed_count, 1203);
        assert!(serde_json::to_string(&cleared).unwrap().len() < 200);
        let changes = s.changes(preview.revision).unwrap();
        assert!(changes.resync_required && changes.changes.is_empty());
        assert!(s.entry(1).is_ok() && s.entry(2).is_ok());
        add(&mut s, "after clear");
        assert!(s.changes(preview.revision).unwrap().resync_required);
        let following = s.changes(cleared.revision).unwrap();
        assert!(!following.resync_required && following.changes.len() == 1);
    }
    #[test]
    fn clear_rejects_changed_preview_and_session_only_does_not_request_sync() {
        let mut s = service();
        let first = s
            .record_capture(processed("first"), false, true, false)
            .unwrap()
            .id;
        let preview = s.clear_preview().unwrap();
        let second = s
            .record_capture(processed("second"), false, true, false)
            .unwrap()
            .id;
        assert_eq!(
            s.clear(preview.revision).unwrap_err().code,
            "history_conflict"
        );
        assert!(s.entry(first).is_ok() && s.entry(second).is_ok());
        let cleared = s.clear(s.revision).unwrap();
        assert_eq!(cleared.removed_count, 2);
        assert!(!cleared.persistent_sync_pending);
    }

    #[test]
    fn clear_compaction_failure_still_reports_committed_deletion() {
        let mut s = service();
        let id = add(&mut s, "committed despite compaction failure");
        let revision = s.revision;
        struct ActiveStatement(*mut rusqlite::ffi::sqlite3_stmt);
        impl Drop for ActiveStatement {
            fn drop(&mut self) {
                unsafe {
                    rusqlite::ffi::sqlite3_finalize(self.0);
                }
            }
        }
        let statement = {
            let connection = s.connection.lock().unwrap();
            let mut statement = ActiveStatement(std::ptr::null_mut());
            let sql = std::ffi::CString::new("SELECT key FROM settings").unwrap();
            assert_eq!(
                unsafe {
                    rusqlite::ffi::sqlite3_prepare_v2(
                        connection.handle(),
                        sql.as_ptr(),
                        -1,
                        &mut statement.0,
                        std::ptr::null_mut(),
                    )
                },
                rusqlite::ffi::SQLITE_OK
            );
            assert_eq!(
                unsafe { rusqlite::ffi::sqlite3_step(statement.0) },
                rusqlite::ffi::SQLITE_ROW
            );
            statement
        };
        let result = s.clear(revision).unwrap();
        assert!(result.changed && result.persistent_sync_pending);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.compaction_error, Some("history_compaction"));
        assert!(s.entry(id).is_err());
        assert_eq!(s.revision, revision + 1);
        assert!(s.changes(revision).unwrap().resync_required);
        drop(statement);
        assert!(!s.clear(s.revision).unwrap().changed);
    }

    const PREVIEW_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAEUlEQVR4nGP4z8DQAMIMGAwAfcEI+4thakwAAAAASUVORK5CYII=";
    #[test]
    fn image_preview_is_read_only_and_requires_current_sensitive_confirmation() {
        let mut s = service();
        let directory = std::env::temp_dir().join(format!(
            "TieZ-preview-core-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        s.data_dir = directory.clone();
        let id = s
            .add_manual("image".into(), PREVIEW_PNG.into(), None, vec![])
            .unwrap()
            .id;
        let revision = s.revision;
        let before = s.entry(id).unwrap();
        let before_bytes = std::fs::read(&before.content).unwrap();
        let preview = s.image_preview(id, revision, false, 512).unwrap();
        assert_eq!(
            (
                preview.raster.original_width,
                preview.raster.original_height
            ),
            (2, 3)
        );
        assert_eq!((preview.id, preview.revision), (id, revision));
        assert_eq!(s.entry(id).unwrap().content, before.content);
        assert_eq!(std::fs::read(&before.content).unwrap(), before_bytes);
        assert_eq!(s.revision, revision);
        assert!(s.changes(revision).unwrap().changes.is_empty());
        s.set_tags(id, vec!["sensitive".into()]).unwrap();
        assert_eq!(
            s.image_preview(id, revision, true, 512).unwrap_err().code,
            "history_conflict"
        );
        assert_eq!(
            s.image_preview(id, s.revision, false, 512)
                .unwrap_err()
                .code,
            "sensitive_confirmation"
        );
        assert!(s.image_preview(id, s.revision, true, 512).is_ok());
        assert_eq!(
            s.image_preview(id, s.revision, true, 1025)
                .unwrap_err()
                .code,
            "preview_dimensions"
        );
        drop(s);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn image_preview_supports_session_ids_and_rejects_non_images_and_missing_rows() {
        let mut s = service();
        let mut image = processed(PREVIEW_PNG);
        image.content_type = "image".into();
        let session = s.record_capture(image, false, false, false).unwrap().id;
        assert!(session < 0);
        assert_eq!(
            s.image_preview(session, s.revision, false, 512).unwrap().id,
            session
        );
        assert!(!s.persistent_sync_pending);
        let text = add(&mut s, "text");
        assert_eq!(
            s.image_preview(text, s.revision, false, 512)
                .unwrap_err()
                .code,
            "unsupported_preview"
        );
        assert_eq!(
            s.image_preview(i64::MAX, s.revision, false, 512)
                .unwrap_err()
                .code,
            "not_found"
        );
    }
}
