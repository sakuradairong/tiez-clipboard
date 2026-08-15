use crate::clipboard_capture::{classify_snapshot, CapturedPayload, ClipboardSnapshot};
use crate::content_identity::{
    calc_image_hash, calc_text_hash, is_text_type, uses_text_content_hash,
};
use crate::database_mutation::{
    delete_record, load_stored_record, save_prepared_record, set_pinned, DeleteRecordPlan,
    update_pinned_orders, PreparedClipboardRecord, StoredClipboardRecord,
};
use crate::encryption::{decrypt_value, encrypt_value, ENCRYPT_PREFIX};
use crate::privacy::contains_sensitive_info;
use base64::Engine;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_HISTORY_LIMIT: i64 = 200;
const SESSION_HISTORY_LIMIT: usize = 500;
const SENSITIVE_PREVIEW: &str = "Sensitive entry — open in the production TieZ UI";
const SENSITIVE_TAGS: &[&str] = &["sensitive", "密码", "password"];
const RICH_IMAGE_FALLBACK_PREFIX: &str = "<!--TIEZ_RICH_IMAGE:";
const RICH_IMAGE_FALLBACK_SUFFIX: &str = "-->";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryItem {
    pub id: i64,
    pub content_type: String,
    pub preview: String,
    pub source_app: String,
    pub captured_at: String,
    pub is_pinned: bool,
    pub tags: Vec<String>,
    pub is_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistorySnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub total: usize,
    pub query: String,
    pub last_action: String,
    pub items: Vec<HistoryItem>,
}

/// Full clipboard payload resolved by stable entry ID.
///
/// Sensitive or encrypted entries keep their metadata but deliberately return
/// no payload until a platform adapter capable of applying TieZ's decryption
/// and privacy policy is connected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryContent {
    pub id: i64,
    pub content_type: String,
    pub content: String,
    pub html_content: Option<String>,
    pub available: bool,
    pub is_sensitive: bool,
    pub unavailable_reason: Option<String>,
}

/// Structured outcome of one clipboard-history mutation.
///
/// `effective_id` is absent after deletion. `replacement_id` remains `None`
/// for the memory adapter and is reserved for production session-only entries
/// that receive a stable positive ID during persistence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryMutationResult {
    pub adapter: &'static str,
    pub action: String,
    pub requested_id: i64,
    pub effective_id: Option<i64>,
    pub replacement_id: Option<i64>,
    pub removed: bool,
    pub generation: u64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PinnedOrderResult {
    pub adapter: &'static str,
    pub action: String,
    pub ordered_ids: Vec<i64>,
    pub generation: u64,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryErrorKind {
    InvalidDatabase,
    Storage,
    NotFound,
    UnsupportedAction,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryError {
    kind: HistoryErrorKind,
    message: String,
}

impl HistoryError {
    fn new(kind: HistoryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> HistoryErrorKind {
        self.kind
    }
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HistoryError {}

#[derive(Debug)]
struct MemoryHistory {
    generation: u64,
    last_action: String,
    items: Vec<HistoryItem>,
    html_by_id: HashMap<i64, String>,
    payloads: HashMap<i64, String>,
}

#[derive(Debug)]
struct SqliteHistory {
    database_path: PathBuf,
    read_only: bool,
    generation: u64,
    last_action: String,
    session: VecDeque<SessionHistoryEntry>,
}

#[derive(Clone, Debug)]
struct SessionHistoryEntry {
    id: i64,
    content_type: String,
    content: String,
    html_content: Option<String>,
    source_app: String,
    timestamp: i64,
    preview: String,
    is_pinned: bool,
    tags: Vec<String>,
    is_external: bool,
    pinned_order: i64,
    use_count: i64,
}

#[derive(Debug)]
enum HistoryAdapter {
    Memory(MemoryHistory),
    Sqlite(SqliteHistory),
}

/// Read/search clipboard history without exposing Tauri or storage details.
///
/// Adapters: mutable in-memory history for development/tests, a production-schema
/// SQLite reader, and an opt-in SQLite writer for the WinUI probe when it is
/// the only process using `clipboard.db`.
#[derive(Debug)]
pub struct ClipboardHistory {
    adapter: HistoryAdapter,
}

impl ClipboardHistory {
    pub fn synthetic() -> Self {
        let mut history = Self::in_memory(sample_items());
        if let HistoryAdapter::Memory(adapter) = &mut history.adapter {
            adapter.html_by_id.insert(
                101,
                "<p><b>WinUI 3</b> main-window probe is connected to Rust through a C ABI.</p>"
                    .to_owned(),
            );
        }
        history
    }

    pub fn in_memory(items: Vec<HistoryItem>) -> Self {
        Self {
            adapter: HistoryAdapter::Memory(MemoryHistory {
                generation: 1,
                last_action: "Rust memory adapter ready".to_owned(),
                items,
                html_by_id: HashMap::new(),
                payloads: HashMap::new(),
            }),
        }
    }

    pub fn open_sqlite_read_only(database_path: impl Into<PathBuf>) -> Result<Self, HistoryError> {
        Self::open_sqlite(database_path, true)
    }

    pub fn open_sqlite_read_write(database_path: impl Into<PathBuf>) -> Result<Self, HistoryError> {
        Self::open_sqlite(database_path, false)
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, HistoryError> {
        let adapter = SqliteHistory::open(database_path.into(), read_only)?;
        Ok(Self {
            adapter: HistoryAdapter::Sqlite(adapter),
        })
    }

    pub fn snapshot(&self, query: &str) -> Result<HistorySnapshot, HistoryError> {
        match &self.adapter {
            HistoryAdapter::Memory(adapter) => Ok(adapter.snapshot(query)),
            HistoryAdapter::Sqlite(adapter) => adapter.snapshot(query),
        }
    }

    pub fn content(&self, entry_id: i64) -> Result<HistoryContent, HistoryError> {
        match &self.adapter {
            HistoryAdapter::Memory(adapter) => adapter.content(entry_id),
            HistoryAdapter::Sqlite(adapter) => adapter.content(entry_id),
        }
    }

    pub fn apply_action(
        &mut self,
        entry_id: i64,
        action: &str,
    ) -> Result<HistoryMutationResult, HistoryError> {
        match &mut self.adapter {
            HistoryAdapter::Memory(adapter) => adapter.apply_action(entry_id, action),
            HistoryAdapter::Sqlite(adapter) => adapter.apply_action(entry_id, action),
        }
    }

    pub fn update_tags(
        &mut self,
        entry_id: i64,
        tags: Vec<String>,
    ) -> Result<HistoryMutationResult, HistoryError> {
        match &mut self.adapter {
            HistoryAdapter::Memory(adapter) => adapter.update_tags(entry_id, tags),
            HistoryAdapter::Sqlite(adapter) => adapter.update_tags(entry_id, tags),
        }
    }

    pub fn reorder_pinned(
        &mut self,
        ordered_ids: Vec<i64>,
    ) -> Result<PinnedOrderResult, HistoryError> {
        match &mut self.adapter {
            HistoryAdapter::Memory(adapter) => adapter.reorder_pinned(ordered_ids),
            HistoryAdapter::Sqlite(adapter) => adapter.reorder_pinned(ordered_ids),
        }
    }

    pub fn ingest_text(
        &mut self,
        content: String,
        source_app: impl Into<String>,
    ) -> Result<HistoryMutationResult, HistoryError> {
        let payload = classify_snapshot(ClipboardSnapshot {
            text: Some(content.clone()),
            ..ClipboardSnapshot::default()
        })
        .unwrap_or(CapturedPayload::Text {
            content,
            content_type: "text".to_owned(),
        });
        self.ingest(payload, source_app)
    }

    pub fn ingest(
        &mut self,
        payload: CapturedPayload,
        source_app: impl Into<String>,
    ) -> Result<HistoryMutationResult, HistoryError> {
        let source_app = source_app.into();
        match &mut self.adapter {
            HistoryAdapter::Memory(adapter) => Ok(adapter.ingest(payload, source_app)),
            HistoryAdapter::Sqlite(adapter) => adapter.ingest(payload, source_app),
        }
    }
}

impl MemoryHistory {
    fn snapshot(&self, query: &str) -> HistorySnapshot {
        let mut items = filter_items(&self.items, query);
        redact_sensitive_previews(&mut items);
        HistorySnapshot {
            adapter: "memory",
            read_only: false,
            generation: self.generation,
            total: items.len(),
            query: query.to_owned(),
            last_action: self.last_action.clone(),
            items,
        }
    }

    fn apply_action(
        &mut self,
        entry_id: i64,
        action: &str,
    ) -> Result<HistoryMutationResult, HistoryError> {
        let (effective_id, removed) = match action {
            "pin" => {
                let item = self
                    .items
                    .iter_mut()
                    .find(|item| item.id == entry_id)
                    .ok_or_else(|| entry_not_found(entry_id))?;
                item.is_pinned = !item.is_pinned;
                self.last_action = format!(
                    "Entry {entry_id} {}",
                    if item.is_pinned { "pinned" } else { "unpinned" }
                );
                (Some(entry_id), false)
            }
            "delete" => {
                let previous_len = self.items.len();
                self.items.retain(|item| item.id != entry_id);
                if previous_len == self.items.len() {
                    return Err(entry_not_found(entry_id));
                }
                self.html_by_id.remove(&entry_id);
                self.payloads.remove(&entry_id);
                self.last_action = format!("Entry {entry_id} deleted");
                (None, true)
            }
            "paste-plain" | "paste-rich" | "copy-plain" | "copy-rich" => {
                ensure_item_exists(&self.items, entry_id)?;
                self.last_action = format!("{action} requested for entry {entry_id}");
                (Some(entry_id), false)
            }
            _ => {
                return Err(HistoryError::new(
                    HistoryErrorKind::UnsupportedAction,
                    format!("unsupported action: {action}"),
                ));
            }
        };

        self.generation += 1;
        Ok(HistoryMutationResult {
            adapter: "memory",
            action: action.to_owned(),
            requested_id: entry_id,
            effective_id,
            replacement_id: None,
            removed,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn ingest(&mut self, payload: CapturedPayload, source_app: String) -> HistoryMutationResult {
        let next_id = self.items.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        let preview = payload.preview();
        let content_type = payload.content_type().to_owned();
        if let Some(html) = payload.html() {
            self.html_by_id.insert(next_id, html.to_owned());
        }
        self.payloads.insert(next_id, payload.content());
        self.items.insert(
            0,
            HistoryItem {
                id: next_id,
                content_type,
                preview,
                source_app: source_app.clone(),
                captured_at: "Just now".to_owned(),
                is_pinned: false,
                tags: Vec::new(),
                is_sensitive: false,
            },
        );
        self.generation += 1;
        self.last_action = format!("Captured {} from {source_app}", payload.content_type());
        HistoryMutationResult {
            adapter: "memory",
            action: format!("ingest-{}", payload.content_type()),
            requested_id: next_id,
            effective_id: Some(next_id),
            replacement_id: None,
            removed: false,
            generation: self.generation,
            message: self.last_action.clone(),
        }
    }

    fn update_tags(
        &mut self,
        entry_id: i64,
        tags: Vec<String>,
    ) -> Result<HistoryMutationResult, HistoryError> {
        let tags = clean_history_tags(tags);
        let item = self
            .items
            .iter_mut()
            .find(|item| item.id == entry_id)
            .ok_or_else(|| entry_not_found(entry_id))?;
        item.is_sensitive = tags_are_sensitive(&tags);
        item.tags = tags;
        self.generation = self.generation.saturating_add(1);
        self.last_action = format!("Tags updated for entry {entry_id}");
        Ok(HistoryMutationResult {
            adapter: "memory",
            action: "update-tags".to_owned(),
            requested_id: entry_id,
            effective_id: Some(entry_id),
            replacement_id: None,
            removed: false,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn reorder_pinned(
        &mut self,
        ordered_ids: Vec<i64>,
    ) -> Result<PinnedOrderResult, HistoryError> {
        validate_pinned_order(
            self.items
                .iter()
                .filter(|item| item.is_pinned)
                .map(|item| item.id),
            &ordered_ids,
        )?;
        let positions: HashMap<i64, usize> = ordered_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        self.items.sort_by_key(|item| {
            if item.is_pinned {
                (0, positions.get(&item.id).copied().unwrap_or(usize::MAX))
            } else {
                (1, usize::MAX)
            }
        });
        self.generation = self.generation.saturating_add(1);
        self.last_action = format!("Reordered {} pinned entries", ordered_ids.len());
        Ok(PinnedOrderResult {
            adapter: "memory",
            action: "reorder-pinned".to_owned(),
            ordered_ids,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn content(&self, entry_id: i64) -> Result<HistoryContent, HistoryError> {
        let item = self
            .items
            .iter()
            .find(|item| item.id == entry_id)
            .ok_or_else(|| entry_not_found(entry_id))?;

        if item.is_sensitive {
            return Ok(redacted_content(
                item.id,
                item.content_type.clone(),
                "Sensitive memory entry requires the production privacy adapter",
            ));
        }

        Ok(HistoryContent {
            id: item.id,
            content_type: item.content_type.clone(),
            content: self
                .payloads
                .get(&item.id)
                .cloned()
                .unwrap_or_else(|| item.preview.clone()),
            html_content: self.html_by_id.get(&item.id).cloned(),
            available: true,
            is_sensitive: false,
            unavailable_reason: None,
        })
    }
}

impl SqliteHistory {
    fn open(database_path: PathBuf, read_only: bool) -> Result<Self, HistoryError> {
        if !database_path.is_file() {
            return Err(HistoryError::new(
                HistoryErrorKind::InvalidDatabase,
                format!(
                    "database path does not point to a clipboard database file: {}",
                    database_path.display()
                ),
            ));
        }

        let connection = open_sqlite_connection(&database_path, read_only)?;
        let table_exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'clipboard_history')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| storage_error("failed to inspect clipboard database", error))?;
        if !table_exists {
            return Err(HistoryError::new(
                HistoryErrorKind::InvalidDatabase,
                format!(
                    "clipboard_history table was not found in {}",
                    database_path.display()
                ),
            ));
        }

        let last_action = if read_only {
            format!("Read-only snapshot from {}", database_path.display())
        } else {
            format!("SQLite write adapter ready ({})", database_path.display())
        };

        Ok(Self {
            database_path,
            read_only,
            generation: 1,
            last_action,
            session: VecDeque::new(),
        })
    }

    fn adapter_name(&self) -> &'static str {
        if self.read_only {
            "sqlite-read-only"
        } else {
            "sqlite"
        }
    }

    fn snapshot(&self, query: &str) -> Result<HistorySnapshot, HistoryError> {
        let connection = open_sqlite_connection(&self.database_path, self.read_only)?;
        let mut items = load_snapshot_items(&connection, query, DEFAULT_HISTORY_LIMIT)?;
        if !self.read_only && !self.session.is_empty() {
            merge_session_snapshot(&mut items, &self.session, query, DEFAULT_HISTORY_LIMIT);
        }
        redact_sensitive_previews(&mut items);

        let generation = if self.read_only {
            database_generation(&self.database_path)
        } else {
            self.generation
        };
        let last_action = if self.read_only {
            format!("Read-only snapshot from {}", self.database_path.display())
        } else {
            self.last_action.clone()
        };

        Ok(HistorySnapshot {
            adapter: self.adapter_name(),
            read_only: self.read_only,
            generation,
            total: items.len(),
            query: query.to_owned(),
            last_action,
            items,
        })
    }

    fn content(&self, entry_id: i64) -> Result<HistoryContent, HistoryError> {
        if entry_id < 0 {
            let entry = self
                .session
                .iter()
                .find(|entry| entry.id == entry_id)
                .ok_or_else(|| entry_not_found(entry_id))?;
            return Ok(entry.history_content());
        }

        let connection = open_sqlite_connection(&self.database_path, self.read_only)?;
        let mut statement = connection
            .prepare(
                "SELECT content_type, content, html_content, tags
                 FROM clipboard_history
                 WHERE id = ?1",
            )
            .map_err(|error| storage_error("failed to prepare clipboard content query", error))?;

        let result = statement.query_row([entry_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        });

        let (content_type, mut content, mut html_content, tags) = match result {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(entry_not_found(entry_id)),
            Err(error) => {
                return Err(storage_error("failed to read clipboard content row", error));
            }
        };

        let is_encrypted = content.starts_with(ENCRYPT_PREFIX)
            || html_content
                .as_deref()
                .is_some_and(|html| html.starts_with(ENCRYPT_PREFIX));
        let is_sensitive = has_sensitive_tag(&tags) || is_encrypted;
        if self.read_only && is_sensitive {
            return Ok(redacted_content(
                entry_id,
                content_type,
                if is_encrypted {
                    "Encrypted entry requires the production Windows decryption adapter"
                } else {
                    "Sensitive entry requires the production privacy adapter"
                },
            ));
        }
        if content.starts_with(ENCRYPT_PREFIX) {
            let Some(decrypted) = decrypt_value(&content) else {
                return Ok(redacted_content(
                    entry_id,
                    content_type,
                    "Encrypted entry could not be decrypted for this Windows user",
                ));
            };
            content = decrypted;
        }
        if html_content
            .as_deref()
            .is_some_and(|html| html.starts_with(ENCRYPT_PREFIX))
        {
            let Some(decrypted) = html_content.as_deref().and_then(decrypt_value) else {
                return Ok(redacted_content(
                    entry_id,
                    content_type,
                    "Encrypted HTML could not be decrypted for this Windows user",
                ));
            };
            html_content = Some(decrypted);
        }

        Ok(HistoryContent {
            id: entry_id,
            content_type,
            content,
            html_content,
            available: true,
            is_sensitive,
            unavailable_reason: None,
        })
    }

    fn apply_action(
        &mut self,
        entry_id: i64,
        action: &str,
    ) -> Result<HistoryMutationResult, HistoryError> {
        if self.read_only {
            return Err(HistoryError::new(
                HistoryErrorKind::ReadOnly,
                format!("action {action} is disabled for sqlite-read-only history"),
            ));
        }

        let connection = open_sqlite_connection(&self.database_path, false)?;
        let (effective_id, removed, replacement_id) = match action {
            "pin" => {
                if entry_id < 0 {
                    let replacement_id = self.persist_pinned_session(&connection, entry_id)?;
                    (Some(replacement_id), false, Some(replacement_id))
                } else {
                    self.toggle_pin(&connection, entry_id)?;
                    (Some(entry_id), false, None)
                }
            }
            "delete" => {
                if entry_id < 0 {
                    let previous_len = self.session.len();
                    self.session.retain(|entry| entry.id != entry_id);
                    if previous_len == self.session.len() {
                        return Err(entry_not_found(entry_id));
                    }
                } else {
                    self.delete(&connection, entry_id)?;
                }
                self.last_action = format!("Entry {entry_id} deleted");
                (None, true, None)
            }
            "paste-plain" | "paste-rich" | "copy-plain" | "copy-rich" => {
                if entry_id < 0 {
                    if !self.session.iter().any(|entry| entry.id == entry_id) {
                        return Err(entry_not_found(entry_id));
                    }
                } else {
                    ensure_sqlite_entry(&connection, entry_id)?;
                }
                self.last_action = format!("{action} requested for entry {entry_id}");
                (Some(entry_id), false, None)
            }
            _ => {
                return Err(HistoryError::new(
                    HistoryErrorKind::UnsupportedAction,
                    format!("unsupported action: {action}"),
                ));
            }
        };

        self.generation = self.generation.saturating_add(1);
        Ok(HistoryMutationResult {
            adapter: self.adapter_name(),
            action: action.to_owned(),
            requested_id: entry_id,
            effective_id,
            replacement_id,
            removed,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn update_tags(
        &mut self,
        entry_id: i64,
        tags: Vec<String>,
    ) -> Result<HistoryMutationResult, HistoryError> {
        if self.read_only {
            return Err(HistoryError::new(
                HistoryErrorKind::ReadOnly,
                "tag updates are disabled for sqlite-read-only history",
            ));
        }

        let connection = open_sqlite_connection(&self.database_path, false)?;
        let cleaned_tags = clean_history_tags(tags);
        let replacement_id = if entry_id < 0 {
            let mut entry = self
                .session
                .iter()
                .find(|entry| entry.id == entry_id)
                .cloned()
                .ok_or_else(|| entry_not_found(entry_id))?;
            entry.tags = cleaned_tags;
            let replacement_id = self.persist_entry(&connection, &entry, 0)?;
            self.session.retain(|entry| entry.id != entry_id);
            Some(replacement_id)
        } else {
            update_persisted_tags(&connection, entry_id, &cleaned_tags)?;
            None
        };

        let effective_id = replacement_id.unwrap_or(entry_id);
        self.generation = self.generation.saturating_add(1);
        self.last_action = if let Some(replacement_id) = replacement_id {
            format!("Entry {entry_id} tagged and persisted as {replacement_id}")
        } else {
            format!("Tags updated for entry {entry_id}")
        };
        Ok(HistoryMutationResult {
            adapter: self.adapter_name(),
            action: "update-tags".to_owned(),
            requested_id: entry_id,
            effective_id: Some(effective_id),
            replacement_id,
            removed: false,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn reorder_pinned(
        &mut self,
        ordered_ids: Vec<i64>,
    ) -> Result<PinnedOrderResult, HistoryError> {
        if self.read_only {
            return Err(HistoryError::new(
                HistoryErrorKind::ReadOnly,
                "pinned reordering is disabled for sqlite-read-only history",
            ));
        }

        let connection = open_sqlite_connection(&self.database_path, false)?;
        let persisted_ids = connection
            .prepare("SELECT id FROM clipboard_history WHERE is_pinned = 1")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|error| storage_error("failed to load pinned entries for reordering", error))?;
        validate_pinned_order(persisted_ids.into_iter(), &ordered_ids)?;
        let item_count = i64::try_from(ordered_ids.len()).unwrap_or(i64::MAX);
        let orders: Vec<(i64, i64)> = ordered_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, id)| {
                let index = i64::try_from(index).unwrap_or(i64::MAX);
                (id, item_count.saturating_sub(index))
            })
            .collect();
        update_pinned_orders(&connection, &orders, now_unix_ms()).map_err(|error| {
            HistoryError::new(
                HistoryErrorKind::Storage,
                format!("failed to update pinned order: {error}"),
            )
        })?;

        self.generation = self.generation.saturating_add(1);
        self.last_action = format!("Reordered {} pinned entries", ordered_ids.len());
        Ok(PinnedOrderResult {
            adapter: self.adapter_name(),
            action: "reorder-pinned".to_owned(),
            ordered_ids,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn ingest(
        &mut self,
        payload: CapturedPayload,
        source_app: String,
    ) -> Result<HistoryMutationResult, HistoryError> {
        if self.read_only {
            return Err(HistoryError::new(
                HistoryErrorKind::ReadOnly,
                "ingest is disabled for sqlite-read-only history",
            ));
        }

        let connection = open_sqlite_connection(&self.database_path, false)?;
        let timestamp = now_unix_ms();
        let mut entry = SessionHistoryEntry::from_payload(
            payload,
            source_app.clone(),
            timestamp,
            &load_privacy_policy(&connection)?,
        );
        let persistent = load_bool_setting(&connection, "app.persistent", false)?;
        let deduplicate = load_bool_setting(&connection, "app.deduplicate", true)?;
        let content_type = entry.content_type.clone();

        let entry_id = if persistent {
            let existing_id = if deduplicate {
                find_persisted_duplicate(&connection, &entry)?
            } else {
                None
            };
            let entry_id = self.persist_entry(&connection, &entry, existing_id.unwrap_or(0))?;
            if deduplicate {
                self.session
                    .retain(|candidate| !candidate.same_content_as(&entry));
            }
            let _ = self.enforce_persistent_limit(&connection);
            entry_id
        } else if deduplicate {
            if let Some(existing_id) = self.refresh_session_duplicate(&entry) {
                existing_id
            } else {
                entry.id = next_session_id(&self.session, timestamp);
                let entry_id = entry.id;
                self.push_session(entry);
                entry_id
            }
        } else {
            entry.id = next_session_id(&self.session, timestamp);
            let entry_id = entry.id;
            self.push_session(entry);
            entry_id
        };
        self.generation = self.generation.saturating_add(1);
        self.last_action = format!("Captured {content_type} from {source_app}");
        Ok(HistoryMutationResult {
            adapter: self.adapter_name(),
            action: format!("ingest-{content_type}"),
            requested_id: entry_id,
            effective_id: Some(entry_id),
            replacement_id: None,
            removed: false,
            generation: self.generation,
            message: self.last_action.clone(),
        })
    }

    fn push_session(&mut self, entry: SessionHistoryEntry) {
        self.session.push_back(entry);
        while self.session.len() > SESSION_HISTORY_LIMIT {
            self.session.pop_front();
        }
    }

    fn refresh_session_duplicate(&mut self, incoming: &SessionHistoryEntry) -> Option<i64> {
        let reuse_id = self
            .session
            .iter()
            .rev()
            .find(|candidate| candidate.same_content_as(incoming))?
            .id;
        let existing = self
            .session
            .iter()
            .find(|candidate| candidate.id == reuse_id)?
            .clone();
        let mut updated = incoming.clone();
        updated.id = reuse_id;
        updated.is_pinned = existing.is_pinned;
        updated.pinned_order = existing.pinned_order;
        updated.use_count = existing.use_count.saturating_add(1);
        if updated.tags.is_empty() {
            updated.tags = existing.tags;
        }

        self.session
            .retain(|candidate| candidate.id == reuse_id || !candidate.same_content_as(incoming));
        if let Some(candidate) = self
            .session
            .iter_mut()
            .find(|candidate| candidate.id == reuse_id)
        {
            *candidate = updated;
        }
        Some(reuse_id)
    }

    fn persist_entry(
        &self,
        connection: &Connection,
        entry: &SessionHistoryEntry,
        stable_id: i64,
    ) -> Result<i64, HistoryError> {
        let mut content = entry.content.clone();
        let attachment = if entry.content_type == "image" {
            let attachment = persist_image_attachment(&self.database_path, &content)?;
            content = attachment.content.clone();
            Some(attachment)
        } else {
            None
        };
        let content_hash = if entry.content_type == "image" {
            calc_image_hash(&content).unwrap_or(0)
        } else {
            calc_text_hash(&content) as i64
        };
        let should_encrypt = entry.is_sensitive();
        let stored_content = if should_encrypt {
            encrypt_sensitive_value(&content)?
        } else {
            content.clone()
        };
        let stored_preview = if should_encrypt {
            encrypt_sensitive_value(&entry.preview)?
        } else {
            entry.preview.clone()
        };
        let stored_html = if should_encrypt {
            entry
                .html_content
                .as_deref()
                .map(encrypt_sensitive_value)
                .transpose()?
        } else {
            entry.html_content.clone()
        };
        let saved = save_prepared_record(
            connection,
            &PreparedClipboardRecord {
                id: stable_id,
                content_type: &entry.content_type,
                content: &stored_content,
                identity_content: &content,
                html_content: stored_html.as_deref(),
                source_app: &entry.source_app,
                source_app_path: None,
                timestamp: entry.timestamp,
                preview: &stored_preview,
                is_pinned: entry.is_pinned,
                content_hash,
                tags: &entry.tags,
                is_external: entry.is_external || entry.content_type == "image",
                pinned_order: entry.pinned_order,
            },
        );
        match saved {
            Ok(entry_id) => Ok(entry_id),
            Err(error) => {
                if let Some(attachment) = attachment.filter(|value| value.created) {
                    let _ = std::fs::remove_file(attachment.content);
                }
                Err(HistoryError::new(
                    HistoryErrorKind::Storage,
                    format!("failed to ingest clipboard payload: {error}"),
                ))
            }
        }
    }

    fn persist_pinned_session(
        &mut self,
        connection: &Connection,
        session_id: i64,
    ) -> Result<i64, HistoryError> {
        let mut entry = self
            .session
            .iter()
            .find(|entry| entry.id == session_id)
            .cloned()
            .ok_or_else(|| entry_not_found(session_id))?;
        entry.is_pinned = true;
        let replacement_id = self.persist_entry(connection, &entry, 0)?;
        let pinned = set_pinned(connection, replacement_id, true, now_unix_ms()).map_err(|error| {
            HistoryError::new(
                HistoryErrorKind::Storage,
                format!("failed to finalize persisted pin state: {error}"),
            )
        })?;
        if !pinned {
            return Err(entry_not_found(replacement_id));
        }
        self.session.retain(|entry| entry.id != session_id);
        let _ = self.enforce_persistent_limit(connection);
        self.last_action = format!("Entry {session_id} pinned as {replacement_id}");
        Ok(replacement_id)
    }

    fn enforce_persistent_limit(&self, connection: &Connection) -> Result<Vec<i64>, HistoryError> {
        if !load_bool_setting(connection, "app.persistent_limit_enabled", true)? {
            return Ok(Vec::new());
        }
        let limit = load_i64_setting(connection, "app.persistent_limit", 500)?.max(0);
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM clipboard_history
                 WHERE is_pinned = 0 AND (tags = '[]' OR tags IS NULL)",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| storage_error("failed to count persistent history", error))?;
        if count <= limit {
            return Ok(Vec::new());
        }

        let deleted_ids = {
            let mut statement = connection
                .prepare(
                    "SELECT id FROM clipboard_history
                     WHERE is_pinned = 0 AND (tags = '[]' OR tags IS NULL)
                     ORDER BY timestamp ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|error| storage_error("failed to prepare history limit query", error))?;
            let rows = statement
                .query_map([count - limit], |row| row.get::<_, i64>(0))
                .map_err(|error| storage_error("failed to query history limit", error))?;
            rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage_error("failed to read history limit row", error))?
        };
        for entry_id in &deleted_ids {
            self.delete(connection, *entry_id)?;
        }
        Ok(deleted_ids)
    }

    fn toggle_pin(&mut self, connection: &Connection, entry_id: i64) -> Result<(), HistoryError> {
        let current: i32 = match connection.query_row(
            "SELECT is_pinned FROM clipboard_history WHERE id = ?1",
            [entry_id],
            |row| row.get(0),
        ) {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(entry_not_found(entry_id)),
            Err(error) => {
                return Err(storage_error("failed to read pin state", error));
            }
        };

        let new_pinned = current == 0;
        let updated =
            set_pinned(connection, entry_id, new_pinned, now_unix_ms()).map_err(|error| {
                HistoryError::new(
                    HistoryErrorKind::Storage,
                    format!("failed to update pin state: {error}"),
                )
            })?;
        if !updated {
            return Err(entry_not_found(entry_id));
        }

        self.last_action = format!(
            "Entry {entry_id} {}",
            if new_pinned { "pinned" } else { "unpinned" }
        );
        Ok(())
    }

    fn delete(&self, connection: &Connection, entry_id: i64) -> Result<(), HistoryError> {
        let entry = load_stored_record(connection, entry_id)
            .map_err(|error| {
                HistoryError::new(
                    HistoryErrorKind::Storage,
                    format!("failed to read clipboard entry before deletion: {error}"),
                )
            })?
            .ok_or_else(|| entry_not_found(entry_id))?;
        let attachments_dir = self
            .database_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("attachments");
        let cleanup_paths = collect_attachment_paths(&entry, &attachments_dir);
        let (content_hash, content_hash_version) = if uses_text_content_hash(&entry.content_type)
            && !entry.content.starts_with(ENCRYPT_PREFIX)
        {
            (calc_text_hash(&entry.content) as i64, 2)
        } else {
            (entry.content_hash, entry.content_hash_version)
        };

        let deleted = delete_record(
            connection,
            DeleteRecordPlan {
                id: entry_id,
                content_type: &entry.content_type,
                content_hash,
                content_hash_version,
                deleted_at: now_unix_ms(),
            },
        )
        .map_err(|error| {
            HistoryError::new(
                HistoryErrorKind::Storage,
                format!("failed to delete clipboard entry: {error}"),
            )
        })?;
        if !deleted {
            return Err(entry_not_found(entry_id));
        }
        cleanup_unreferenced_attachment_paths(connection, cleanup_paths, &attachments_dir);
        Ok(())
    }
}

impl SessionHistoryEntry {
    fn from_payload(
        payload: CapturedPayload,
        source_app: String,
        timestamp: i64,
        privacy_policy: &PrivacyPolicy,
    ) -> Self {
        let content_type = payload.content_type().to_owned();
        let content = payload.content();
        let preview = payload.preview();
        let html_content = payload.html().map(str::to_owned);
        let tags = if privacy_policy.should_protect(&content_type, &content) {
            vec!["sensitive".to_owned()]
        } else {
            Vec::new()
        };
        let is_external = matches!(content_type.as_str(), "file" | "video")
            || (content_type == "image" && !content.trim_start().starts_with("data:"));
        Self {
            id: 0,
            content_type,
            content,
            html_content,
            source_app,
            timestamp,
            preview,
            is_pinned: false,
            tags,
            is_external,
            pinned_order: 0,
            use_count: 0,
        }
    }

    fn is_sensitive(&self) -> bool {
        self.tags.iter().any(|tag| {
            SENSITIVE_TAGS
                .iter()
                .any(|sensitive| sensitive.eq_ignore_ascii_case(tag))
        })
    }

    fn same_content_as(&self, other: &Self) -> bool {
        if self.content_type == "image" || other.content_type == "image" {
            return self.content_type == "image"
                && other.content_type == "image"
                && calc_image_hash(&self.content).is_some()
                && calc_image_hash(&self.content) == calc_image_hash(&other.content);
        }
        if self.content_type == "rich_text" && other.content_type == "rich_text" {
            let normalized_html = |value: &str| value.trim().replace("\r\n", "\n");
            let html_matches = match (self.html_content.as_deref(), other.html_content.as_deref()) {
                (None, None) => true,
                (Some(left), Some(right)) => normalized_html(left) == normalized_html(right),
                _ => false,
            };
            if !html_matches {
                return false;
            }
        }
        self.content == other.content
            || calc_text_hash(&self.content) == calc_text_hash(&other.content)
    }

    fn history_item(&self) -> HistoryItem {
        HistoryItem {
            id: self.id,
            content_type: self.content_type.clone(),
            preview: self.preview.clone(),
            source_app: self.source_app.clone(),
            captured_at: format_timestamp(self.timestamp),
            is_pinned: self.is_pinned,
            tags: self.tags.clone(),
            is_sensitive: self.is_sensitive(),
        }
    }

    fn history_content(&self) -> HistoryContent {
        HistoryContent {
            id: self.id,
            content_type: self.content_type.clone(),
            content: self.content.clone(),
            html_content: self.html_content.clone(),
            available: true,
            is_sensitive: self.is_sensitive(),
            unavailable_reason: None,
        }
    }
}

#[derive(Debug)]
struct PrivacyPolicy {
    enabled: bool,
    kinds: Vec<String>,
    custom_rules: Vec<String>,
}

impl PrivacyPolicy {
    fn should_protect(&self, content_type: &str, content: &str) -> bool {
        self.enabled
            && is_text_type(content_type)
            && contains_sensitive_info(content, &self.kinds, &self.custom_rules)
    }
}

fn load_privacy_policy(connection: &Connection) -> Result<PrivacyPolicy, HistoryError> {
    let enabled = load_setting(connection, "app.privacy_protection", "true")? == "true";
    let kinds = load_setting(
        connection,
        "app.privacy_protection_kinds",
        "phone,idcard,email,secret",
    )?
    .split(',')
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_owned)
    .collect();
    let custom_rules = load_setting(
        connection,
        "app.privacy_protection_custom_rules",
        "",
    )?
    .lines()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_owned)
    .collect();
    Ok(PrivacyPolicy {
        enabled,
        kinds,
        custom_rules,
    })
}

fn load_setting(
    connection: &Connection,
    key: &str,
    default_value: &str,
) -> Result<String, HistoryError> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map(|value| value.unwrap_or_else(|| default_value.to_owned()))
        .map_err(|error| storage_error(&format!("failed to read setting {key}"), error))
}

fn load_bool_setting(
    connection: &Connection,
    key: &str,
    default_value: bool,
) -> Result<bool, HistoryError> {
    let value = load_setting(
        connection,
        key,
        if default_value { "true" } else { "false" },
    )?;
    Ok(value.eq_ignore_ascii_case("true") || value == "1")
}

fn load_i64_setting(
    connection: &Connection,
    key: &str,
    default_value: i64,
) -> Result<i64, HistoryError> {
    let value = load_setting(connection, key, &default_value.to_string())?;
    value.parse::<i64>().map_err(|error| {
        HistoryError::new(
            HistoryErrorKind::Storage,
            format!("setting {key} is not a valid integer: {error}"),
        )
    })
}

fn next_session_id(session: &VecDeque<SessionHistoryEntry>, timestamp: i64) -> i64 {
    let mut candidate = -timestamp.max(1);
    while session.iter().any(|entry| entry.id == candidate) {
        candidate = candidate.saturating_sub(1);
    }
    candidate
}

fn merge_session_snapshot(
    persisted: &mut Vec<HistoryItem>,
    session: &VecDeque<SessionHistoryEntry>,
    query: &str,
    limit: i64,
) {
    let mut session_items: Vec<HistoryItem> = session
        .iter()
        .rev()
        .filter(|entry| session_entry_matches_query(entry, query))
        .map(SessionHistoryEntry::history_item)
        .collect();
    if session_items.is_empty() {
        return;
    }
    let insertion_index = persisted
        .iter()
        .position(|item| !item.is_pinned)
        .unwrap_or(persisted.len());
    persisted.splice(insertion_index..insertion_index, session_items.drain(..));
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if persisted.len() > limit {
        persisted.truncate(limit);
    }
}

fn session_entry_matches_query(entry: &SessionHistoryEntry, query: &str) -> bool {
    let matches_text = |text: &str| {
        let normalized = text.trim().to_lowercase();
        normalized.is_empty()
            || entry.content.to_lowercase().contains(&normalized)
            || entry.source_app.to_lowercase().contains(&normalized)
            || entry.content_type.to_lowercase().contains(&normalized)
            || entry
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(&normalized))
    };
    match parse_snapshot_query(query) {
        SnapshotQuery::Latest => true,
        SnapshotQuery::Type { content_type, text } => {
            entry.content_type.eq_ignore_ascii_case(content_type) && matches_text(text)
        }
        SnapshotQuery::Text(text) => matches_text(text),
    }
}

fn find_persisted_duplicate(
    connection: &Connection,
    entry: &SessionHistoryEntry,
) -> Result<Option<i64>, HistoryError> {
    let content_hash = if entry.content_type == "image" {
        calc_image_hash(&entry.content).unwrap_or(0)
    } else {
        calc_text_hash(&entry.content) as i64
    };
    let content_types: &[&str] = if entry.content_type == "rich_text" {
        &["rich_text", "text", "code", "url"]
    } else {
        &[entry.content_type.as_str()]
    };

    for content_type in content_types {
        let mut statement = connection
            .prepare(
                "SELECT id, html_content FROM clipboard_history
                 WHERE content_type = ?1 AND (content_hash = ?2 OR content = ?3)
                 ORDER BY timestamp DESC, id DESC",
            )
            .map_err(|error| storage_error("failed to prepare deduplication query", error))?;
        let rows = statement
            .query_map(
                rusqlite::params![content_type, content_hash, entry.content.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(|error| storage_error("failed to query duplicate history", error))?;
        for row in rows {
            let (id, stored_html) =
                row.map_err(|error| storage_error("failed to read duplicate row", error))?;
            if entry.content_type == "rich_text" && *content_type == "rich_text" {
                let stored_html = match stored_html {
                    Some(value) if value.starts_with(ENCRYPT_PREFIX) => decrypt_value(&value),
                    value => value,
                };
                let normalize = |value: &str| value.trim().replace("\r\n", "\n");
                let matches = match (entry.html_content.as_deref(), stored_html.as_deref()) {
                    (None, None) => true,
                    (Some(left), Some(right)) => normalize(left) == normalize(right),
                    _ => false,
                };
                if !matches {
                    continue;
                }
            }
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn collect_attachment_paths(entry: &StoredClipboardRecord, attachments_dir: &Path) -> Vec<PathBuf> {
    let mut paths = HashSet::new();
    if entry.is_external {
        if let Some(content) = decrypt_storage_value(&entry.content) {
            let path = PathBuf::from(content);
            if path_is_within(&path, attachments_dir) {
                paths.insert(path);
            }
        }
    }
    if let Some(html) = entry
        .html_content
        .as_deref()
        .and_then(decrypt_storage_value)
    {
        if let Some(path) = rich_image_fallback_path(&html) {
            if path_is_within(&path, attachments_dir) {
                paths.insert(path);
            }
        }
    }
    paths.into_iter().collect()
}

fn cleanup_unreferenced_attachment_paths(
    connection: &Connection,
    cleanup_paths: impl IntoIterator<Item = PathBuf>,
    attachments_dir: &Path,
) {
    let candidates: HashSet<PathBuf> = cleanup_paths
        .into_iter()
        .filter(|path| path_is_within(path, attachments_dir))
        .collect();
    if candidates.is_empty() {
        return;
    }

    let mut statement = match connection
        .prepare("SELECT content, html_content, is_external FROM clipboard_history")
    {
        Ok(statement) => statement,
        Err(_) => return,
    };
    let rows = match statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i32>(2)? == 1,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return,
    };

    let mut referenced = HashSet::new();
    for row in rows {
        let Ok((content_raw, html_raw, is_external)) = row else {
            return;
        };
        if is_external {
            let Some(content) = decrypt_storage_value(&content_raw) else {
                return;
            };
            referenced.insert(path_identity(Path::new(&content)));
        }
        if let Some(html_raw) = html_raw {
            let Some(html) = decrypt_storage_value(&html_raw) else {
                return;
            };
            if let Some(path) = rich_image_fallback_path(&html) {
                referenced.insert(path_identity(&path));
            }
        }
    }

    for path in candidates {
        if path.exists() && !referenced.contains(&path_identity(&path)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn decrypt_storage_value(value: &str) -> Option<String> {
    if value.starts_with(ENCRYPT_PREFIX) {
        decrypt_value(value)
    } else {
        Some(value.to_owned())
    }
}

fn rich_image_fallback_path(html: &str) -> Option<PathBuf> {
    let start = html.rfind(RICH_IMAGE_FALLBACK_PREFIX)? + RICH_IMAGE_FALLBACK_PREFIX.len();
    let end = start + html[start..].find(RICH_IMAGE_FALLBACK_SUFFIX)?;
    let payload = html[start..end].trim();
    if payload.is_empty() || payload.starts_with("data:image/") {
        return None;
    }
    let raw = payload.strip_prefix("file://").unwrap_or(payload);
    let raw = if raw.starts_with('/') && raw.chars().nth(2) == Some(':') {
        &raw[1..]
    } else {
        raw
    };
    Some(PathBuf::from(percent_decode(raw)))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push(((high << 4) | low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn path_identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_is_within(path: &Path, directory: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    path_identity(path).starts_with(path_identity(directory))
}

fn encrypt_sensitive_value(value: &str) -> Result<String, HistoryError> {
    encrypt_value(value).ok_or_else(|| {
        HistoryError::new(
            HistoryErrorKind::Storage,
            "failed to protect sensitive clipboard content with Windows DPAPI",
        )
    })
}

fn update_persisted_tags(
    connection: &Connection,
    entry_id: i64,
    tags: &[String],
) -> Result<(), HistoryError> {
    let row = connection.query_row(
        "SELECT content_type, content, preview, html_content, content_hash,
                content_hash_version
         FROM clipboard_history WHERE id = ?1",
        [entry_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    );
    let (content_type, content, preview, html_content, old_hash, old_hash_version) = match row {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(entry_not_found(entry_id)),
        Err(error) => return Err(storage_error("failed to read entry before tagging", error)),
    };

    let plaintext_content = decrypt_storage_value(&content).ok_or_else(|| {
        HistoryError::new(
            HistoryErrorKind::Storage,
            "encrypted clipboard content could not be decrypted before updating tags",
        )
    })?;
    let plaintext_preview = decrypt_storage_value(&preview).ok_or_else(|| {
        HistoryError::new(
            HistoryErrorKind::Storage,
            "encrypted clipboard preview could not be decrypted before updating tags",
        )
    })?;
    let plaintext_html = html_content
        .as_deref()
        .map(|value| {
            decrypt_storage_value(value).ok_or_else(|| {
                HistoryError::new(
                    HistoryErrorKind::Storage,
                    "encrypted HTML could not be decrypted before updating tags",
                )
            })
        })
        .transpose()?;
    let sensitive = tags_are_sensitive(tags);
    let stored_content = if sensitive {
        if content.starts_with(ENCRYPT_PREFIX) {
            content
        } else {
            encrypt_sensitive_value(&plaintext_content)?
        }
    } else {
        plaintext_content.clone()
    };
    let stored_preview = if sensitive {
        if preview.starts_with(ENCRYPT_PREFIX) {
            preview
        } else {
            encrypt_sensitive_value(&plaintext_preview)?
        }
    } else {
        plaintext_preview
    };
    let stored_html = match (sensitive, html_content, plaintext_html) {
        (true, Some(stored), _) if stored.starts_with(ENCRYPT_PREFIX) => Some(stored),
        (true, _, Some(plaintext)) => Some(encrypt_sensitive_value(&plaintext)?),
        (false, _, plaintext) => plaintext,
        _ => None,
    };
    let (content_hash, content_hash_version) = if uses_text_content_hash(&content_type) {
        (calc_text_hash(&plaintext_content) as i64, 2)
    } else if content_type == "image" {
        (
            calc_image_hash(&plaintext_content).unwrap_or(old_hash),
            old_hash_version,
        )
    } else {
        (old_hash, old_hash_version)
    };
    let tags_json = serde_json::to_string(tags)
        .map_err(|error| storage_error("failed to serialize entry tags", error))?;

    connection
        .execute_batch("SAVEPOINT tiez_winui_update_tags")
        .map_err(|error| storage_error("failed to start tag update", error))?;
    let result = (|| {
        let updated = connection
            .execute(
                "UPDATE clipboard_history
                 SET content = ?1,
                     preview = ?2,
                     html_content = ?3,
                     tags = ?4,
                     content_hash = ?5,
                     content_hash_version = ?6,
                     sync_updated_at = ?7,
                     sync_updated_by = COALESCE(
                         (SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                 WHERE id = ?8",
                rusqlite::params![
                    stored_content,
                    stored_preview,
                    stored_html,
                    tags_json,
                    content_hash,
                    content_hash_version,
                    now_unix_ms(),
                    entry_id,
                ],
            )
            .map_err(|error| storage_error("failed to update entry tags", error))?;
        if updated != 1 {
            return Err(entry_not_found(entry_id));
        }

        connection
            .execute("DELETE FROM entry_tags WHERE entry_id = ?1", [entry_id])
            .map_err(|error| storage_error("failed to replace normalized entry tags", error))?;
        for tag in tags {
            connection
                .execute(
                    "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                    rusqlite::params![entry_id, tag],
                )
                .map_err(|error| storage_error("failed to store normalized entry tag", error))?;
        }
        if sensitive {
            connection
                .execute(
                    "DELETE FROM clipboard_image_analysis WHERE entry_id = ?1",
                    [entry_id],
                )
                .map_err(|error| storage_error("failed to remove sensitive OCR index", error))?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => connection
            .execute_batch("RELEASE SAVEPOINT tiez_winui_update_tags")
            .map_err(|error| storage_error("failed to commit tag update", error)),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK TO SAVEPOINT tiez_winui_update_tags");
            let _ = connection.execute_batch("RELEASE SAVEPOINT tiez_winui_update_tags");
            Err(error)
        }
    }
}

fn clean_history_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .filter_map(|tag| {
            let tag = tag.trim();
            if tag.is_empty() || !seen.insert(tag.to_owned()) {
                None
            } else {
                Some(tag.to_owned())
            }
        })
        .collect()
}

fn tags_are_sensitive(tags: &[String]) -> bool {
    tags.iter().any(|tag| {
        SENSITIVE_TAGS
            .iter()
            .any(|sensitive| sensitive.eq_ignore_ascii_case(tag))
    })
}

#[derive(Debug)]
struct PersistedImageAttachment {
    content: String,
    created: bool,
}

fn persist_image_attachment(
    database_path: &Path,
    content: &str,
) -> Result<PersistedImageAttachment, HistoryError> {
    let trimmed = content.trim();
    let source_path = Path::new(trimmed);
    let bytes = if source_path.is_file() {
        std::fs::read(source_path)
            .map_err(|error| storage_error("failed to read captured image", error))?
    } else {
        let payload = trimmed
            .split_once(',')
            .map(|(_, payload)| payload)
            .unwrap_or(trimmed)
            .replace('\r', "")
            .replace('\n', "");
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .map_err(|error| {
                HistoryError::new(
                    HistoryErrorKind::Storage,
                    format!("failed to decode captured image: {error}"),
                )
            })?
    };
    if bytes.is_empty() {
        return Err(HistoryError::new(
            HistoryErrorKind::Storage,
            "captured image is empty",
        ));
    }

    let data_dir = database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            HistoryError::new(
                HistoryErrorKind::InvalidDatabase,
                "clipboard database has no data directory",
            )
        })?;
    let attachments_dir = data_dir.join("attachments");
    std::fs::create_dir_all(&attachments_dir)
        .map_err(|error| storage_error("failed to create image attachment directory", error))?;

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let destination = attachments_dir.join(format!("img_{:x}.png", hasher.finish()));
    let created = !destination.exists();
    if created {
        std::fs::write(&destination, bytes)
            .map_err(|error| storage_error("failed to persist captured image", error))?;
    }

    Ok(PersistedImageAttachment {
        content: destination.to_string_lossy().into_owned(),
        created,
    })
}

fn open_sqlite_connection(path: &Path, read_only: bool) -> Result<Connection, HistoryError> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(path, flags).map_err(|error| {
        storage_error(
            &format!(
                "failed to open {} {}",
                path.display(),
                if read_only { "read-only" } else { "read-write" }
            ),
            error,
        )
    })
}

fn ensure_sqlite_entry(connection: &Connection, entry_id: i64) -> Result<(), HistoryError> {
    let exists: i64 = match connection.query_row(
        "SELECT 1 FROM clipboard_history WHERE id = ?1",
        [entry_id],
        |row| row.get(0),
    ) {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(entry_not_found(entry_id)),
        Err(error) => return Err(storage_error("failed to look up clipboard entry", error)),
    };
    let _ = exists;
    Ok(())
}

fn database_generation(path: &Path) -> u64 {
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let wal_path = PathBuf::from(wal_path);
    [path, wal_path.as_path()]
        .into_iter()
        .filter_map(|candidate| std::fs::metadata(candidate).ok())
        .filter_map(|metadata| metadata.modified().ok())
        .filter_map(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .max()
        .unwrap_or(1)
}

fn load_latest_history(
    connection: &Connection,
    limit: i64,
) -> Result<Vec<HistoryItem>, HistoryError> {
    query_history(
        connection,
        "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
         FROM clipboard_history
         ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
         LIMIT ?1",
        QueryArgs::Limit(limit),
    )
}

fn load_snapshot_items(
    connection: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<HistoryItem>, HistoryError> {
    match parse_snapshot_query(query) {
        SnapshotQuery::Latest => load_latest_history(connection, limit),
        SnapshotQuery::Type { content_type, text } if text.is_empty() => query_history(
            connection,
            "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
             FROM clipboard_history
             WHERE lower(content_type) = lower(?1)
             ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
             LIMIT ?2",
            QueryArgs::Type {
                content_type,
                text: None,
                limit,
            },
        ),
        SnapshotQuery::Type { content_type, text } => {
            search_history_by_type(connection, content_type, text, limit)
        }
        SnapshotQuery::Text(text) => search_latest_history(connection, text, limit),
    }
}

fn search_history_by_type(
    connection: &Connection,
    content_type: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<HistoryItem>, HistoryError> {
    let sql = if has_image_analysis_table(connection)? {
        "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
         FROM clipboard_history
         WHERE lower(content_type) = lower(?1)
           AND (
                instr(lower(source_app), lower(?2)) > 0
                OR instr(lower(preview), lower(?2)) > 0
                OR instr(lower(tags), lower(?2)) > 0
                OR EXISTS (
                    SELECT 1 FROM clipboard_image_analysis analysis
                    WHERE analysis.entry_id = clipboard_history.id
                      AND (
                           instr(lower(analysis.ocr_text), lower(?2)) > 0
                           OR instr(lower(analysis.qr_codes), lower(?2)) > 0
                      )
                )
           )
         ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
         LIMIT ?3"
    } else {
        "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
         FROM clipboard_history
         WHERE lower(content_type) = lower(?1)
           AND (
                instr(lower(source_app), lower(?2)) > 0
                OR instr(lower(preview), lower(?2)) > 0
                OR instr(lower(tags), lower(?2)) > 0
           )
         ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
         LIMIT ?3"
    };
    query_history(
        connection,
        sql,
        QueryArgs::Type {
            content_type,
            text: Some(query),
            limit,
        },
    )
}

fn search_latest_history(
    connection: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<HistoryItem>, HistoryError> {
    let sql = if has_image_analysis_table(connection)? {
        "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
         FROM clipboard_history
         WHERE instr(lower(content_type), lower(?1)) > 0
            OR instr(lower(source_app), lower(?1)) > 0
            OR instr(lower(preview), lower(?1)) > 0
            OR instr(lower(tags), lower(?1)) > 0
            OR EXISTS (
                SELECT 1 FROM clipboard_image_analysis analysis
                WHERE analysis.entry_id = clipboard_history.id
                  AND (
                       instr(lower(analysis.ocr_text), lower(?1)) > 0
                       OR instr(lower(analysis.qr_codes), lower(?1)) > 0
                  )
            )
         ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
         LIMIT ?2"
    } else {
        "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
         FROM clipboard_history
         WHERE instr(lower(content_type), lower(?1)) > 0
            OR instr(lower(source_app), lower(?1)) > 0
            OR instr(lower(preview), lower(?1)) > 0
            OR instr(lower(tags), lower(?1)) > 0
         ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
         LIMIT ?2"
    };
    query_history(connection, sql, QueryArgs::Search { query, limit })
}

fn has_image_analysis_table(connection: &Connection) -> Result<bool, HistoryError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type = 'table' AND name = 'clipboard_image_analysis'
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("failed to inspect image-analysis schema", error))
}

enum SnapshotQuery<'a> {
    Latest,
    Text(&'a str),
    Type {
        content_type: &'a str,
        text: &'a str,
    },
}

enum QueryArgs<'a> {
    Limit(i64),
    Search {
        query: &'a str,
        limit: i64,
    },
    Type {
        content_type: &'a str,
        text: Option<&'a str>,
        limit: i64,
    },
}

fn parse_snapshot_query(query: &str) -> SnapshotQuery<'_> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return SnapshotQuery::Latest;
    }
    if let Some(rest) = strip_type_prefix(trimmed) {
        let rest = rest.trim_start();
        if rest.is_empty() {
            return SnapshotQuery::Text(trimmed);
        }
        let (content_type, text) = match rest.split_once(char::is_whitespace) {
            Some((content_type, text)) => (content_type, text.trim()),
            None => (rest, ""),
        };
        if content_type.is_empty() {
            return SnapshotQuery::Text(trimmed);
        }
        return SnapshotQuery::Type { content_type, text };
    }
    SnapshotQuery::Text(trimmed)
}

fn strip_type_prefix(query: &str) -> Option<&str> {
    let bytes = query.as_bytes();
    if bytes.len() >= 5 && bytes[..5].eq_ignore_ascii_case(b"type:") {
        Some(&query[5..])
    } else {
        None
    }
}

fn redact_sensitive_previews(items: &mut [HistoryItem]) {
    for item in items {
        if item.is_sensitive || item.preview.starts_with(ENCRYPT_PREFIX) {
            item.preview = SENSITIVE_PREVIEW.to_owned();
            item.is_sensitive = true;
        }
    }
}

fn query_history(
    connection: &Connection,
    sql: &str,
    args: QueryArgs<'_>,
) -> Result<Vec<HistoryItem>, HistoryError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| storage_error("failed to prepare history query", error))?;

    let map_row = |row: &rusqlite::Row<'_>| {
        let timestamp = row.get::<_, i64>(4)?;
        let tags = parse_tags_json(&row.get::<_, String>(6)?);
        Ok(HistoryItem {
            id: row.get(0)?,
            content_type: row.get(1)?,
            preview: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            source_app: row.get(3)?,
            captured_at: format_timestamp(timestamp),
            is_pinned: row.get::<_, i32>(5)? == 1,
            is_sensitive: tags_are_sensitive(&tags),
            tags,
        })
    };

    let rows = match args {
        QueryArgs::Limit(limit) => statement
            .query_map([limit], map_row)
            .map_err(|error| storage_error("failed to load clipboard history", error))?,
        QueryArgs::Search { query, limit } => statement
            .query_map(rusqlite::params![query, limit], map_row)
            .map_err(|error| storage_error("failed to search clipboard history", error))?,
        QueryArgs::Type {
            content_type,
            text: None,
            limit,
        } => statement
            .query_map(rusqlite::params![content_type, limit], map_row)
            .map_err(|error| storage_error("failed to filter clipboard history by type", error))?,
        QueryArgs::Type {
            content_type,
            text: Some(text),
            limit,
        } => statement
            .query_map(rusqlite::params![content_type, text, limit], map_row)
            .map_err(|error| storage_error("failed to search clipboard history by type", error))?,
    };

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage_error("failed to read clipboard history row", error))
}

fn has_sensitive_tag(tags_json: &str) -> bool {
    tags_are_sensitive(&parse_tags_json(tags_json))
}

fn parse_tags_json(tags_json: &str) -> Vec<String> {
    serde_json::from_str(tags_json).unwrap_or_default()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn format_timestamp(timestamp: i64) -> String {
    let now_ms = now_unix_ms();
    let now_ms = if now_ms == 0 { timestamp } else { now_ms };
    let age_ms = now_ms.saturating_sub(timestamp);

    match age_ms {
        age if age < 60_000 => "Just now".to_owned(),
        age if age < 3_600_000 => format!("{} minutes ago", age / 60_000),
        age if age < 86_400_000 => format!("{} hours ago", age / 3_600_000),
        age if age < 604_800_000 => format!("{} days ago", age / 86_400_000),
        _ => timestamp.to_string(),
    }
}

fn filter_items(items: &[HistoryItem], query: &str) -> Vec<HistoryItem> {
    items
        .iter()
        .filter(|item| item_matches_query(item, query))
        .cloned()
        .collect()
}

fn item_matches_query(item: &HistoryItem, query: &str) -> bool {
    match parse_snapshot_query(query) {
        SnapshotQuery::Latest => true,
        SnapshotQuery::Type {
            content_type,
            text,
        } => {
            item.content_type.eq_ignore_ascii_case(content_type)
                && (text.is_empty() || item_matches_text(item, text))
        }
        SnapshotQuery::Text(text) => item_matches_text(item, text),
    }
}

fn item_matches_text(item: &HistoryItem, query: &str) -> bool {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return true;
    }
    item.preview.to_lowercase().contains(&normalized_query)
        || item.source_app.to_lowercase().contains(&normalized_query)
        || item.content_type.to_lowercase().contains(&normalized_query)
        || item
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(&normalized_query))
}

fn validate_pinned_order(
    current_ids: impl IntoIterator<Item = i64>,
    ordered_ids: &[i64],
) -> Result<(), HistoryError> {
    let current: HashSet<i64> = current_ids.into_iter().collect();
    let requested: HashSet<i64> = ordered_ids.iter().copied().collect();
    if ordered_ids.iter().any(|id| *id <= 0)
        || requested.len() != ordered_ids.len()
        || requested != current
    {
        return Err(HistoryError::new(
            HistoryErrorKind::NotFound,
            "the pinned entry set changed; refresh before reordering",
        ));
    }
    Ok(())
}

fn ensure_item_exists(items: &[HistoryItem], entry_id: i64) -> Result<(), HistoryError> {
    items
        .iter()
        .any(|item| item.id == entry_id)
        .then_some(())
        .ok_or_else(|| entry_not_found(entry_id))
}

fn entry_not_found(entry_id: i64) -> HistoryError {
    HistoryError::new(
        HistoryErrorKind::NotFound,
        format!("clipboard entry {entry_id} was not found"),
    )
}

fn redacted_content(
    entry_id: i64,
    content_type: String,
    reason: impl Into<String>,
) -> HistoryContent {
    HistoryContent {
        id: entry_id,
        content_type,
        content: String::new(),
        html_content: None,
        available: false,
        is_sensitive: true,
        unavailable_reason: Some(reason.into()),
    }
}

fn storage_error(context: &str, error: impl fmt::Display) -> HistoryError {
    HistoryError::new(HistoryErrorKind::Storage, format!("{context}: {error}"))
}

fn sample_items() -> Vec<HistoryItem> {
    vec![
        HistoryItem {
            id: 101,
            content_type: "text".to_owned(),
            preview: "WinUI 3 main-window probe is connected to Rust through a C ABI.".to_owned(),
            source_app: "Visual Studio".to_owned(),
            captured_at: "Just now".to_owned(),
            is_pinned: true,
            tags: vec!["迁移".to_owned()],
            is_sensitive: false,
        },
        HistoryItem {
            id: 102,
            content_type: "code".to_owned(),
            preview: "tiez_core_get_snapshot_json(handle, query);".to_owned(),
            source_app: "Windows Terminal".to_owned(),
            captured_at: "1 minute ago".to_owned(),
            is_pinned: false,
            tags: vec!["代码".to_owned()],
            is_sensitive: false,
        },
        HistoryItem {
            id: 103,
            content_type: "url".to_owned(),
            preview: "https://github.com/jimuzhe/tiez-clipboard/issues/154".to_owned(),
            source_app: "Microsoft Edge".to_owned(),
            captured_at: "3 minutes ago".to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            is_sensitive: false,
        },
        HistoryItem {
            id: 104,
            content_type: "text".to_owned(),
            preview: "中文、emoji 🚀 和 UTF-8 必须完整穿过 Rust/C++ 边界。".to_owned(),
            source_app: "TieZ".to_owned(),
            captured_at: "5 minutes ago".to_owned(),
            is_pinned: false,
            tags: vec!["中文".to_owned()],
            is_sensitive: false,
        },
        HistoryItem {
            id: 105,
            content_type: "image".to_owned(),
            preview: "Image preview placeholder · 1920 × 1080".to_owned(),
            source_app: "Snipping Tool".to_owned(),
            captured_at: "8 minutes ago".to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            is_sensitive: false,
        },
        HistoryItem {
            id: 106,
            content_type: "file".to_owned(),
            preview: "release-notes.md\nTieZ-setup.exe".to_owned(),
            source_app: "File Explorer".to_owned(),
            captured_at: "12 minutes ago".to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            is_sensitive: false,
        },
        HistoryItem {
            id: 107,
            content_type: "text".to_owned(),
            preview: "hunter2-should-not-appear-in-snapshot".to_owned(),
            source_app: "Password Manager".to_owned(),
            captured_at: "15 minutes ago".to_owned(),
            is_pinned: false,
            tags: vec!["password".to_owned()],
            is_sensitive: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::fs;

    fn temporary_database(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tiez-core-{name}-{nonce}.db"))
    }

    fn create_test_database(path: &Path) {
        let connection = Connection::open(path).unwrap();
        crate::database_migrations::run_migrations(&connection).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO settings (key, value)
                 VALUES ('app.anon_id', 'winui-test')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO settings (key, value)
                 VALUES ('app.persistent', 'true')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, pinned_order, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    1,
                    "text",
                    "hello\nfull content",
                    Option::<String>::None,
                    "Notepad",
                    10,
                    "hello world",
                    0,
                    0,
                    "[]"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, pinned_order, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    2,
                    "text",
                    ENCRYPT_PREFIX,
                    Option::<String>::None,
                    "TieZ",
                    20,
                    "dpapi:encrypted",
                    1,
                    3,
                    "[]"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, pinned_order, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    3,
                    "text",
                    "plaintext",
                    Option::<String>::None,
                    "Browser",
                    15,
                    "visible secret",
                    0,
                    0,
                    "[\"password\"]"
                ],
            )
            .unwrap();
    }

    fn set_test_setting(path: &Path, key: &str, value: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .unwrap();
    }

    fn sqlite_session_len(history: &ClipboardHistory) -> usize {
        match &history.adapter {
            HistoryAdapter::Sqlite(adapter) => adapter.session.len(),
            HistoryAdapter::Memory(_) => panic!("expected sqlite adapter"),
        }
    }

    #[test]
    fn memory_history_filters_and_applies_actions() {
        let mut history = ClipboardHistory::synthetic();

        let snapshot = history.snapshot("中文").unwrap();
        assert_eq!(snapshot.adapter, "memory");
        assert!(!snapshot.read_only);
        assert_eq!(snapshot.total, 1);
        assert_eq!(snapshot.items[0].id, 104);

        let pin_result = history.apply_action(102, "pin").unwrap();
        assert_eq!(pin_result.adapter, "memory");
        assert_eq!(pin_result.action, "pin");
        assert_eq!(pin_result.requested_id, 102);
        assert_eq!(pin_result.effective_id, Some(102));
        assert_eq!(pin_result.replacement_id, None);
        assert!(!pin_result.removed);
        assert_eq!(pin_result.generation, 2);
        let snapshot = history.snapshot("").unwrap();
        assert_eq!(snapshot.generation, 2);
        assert!(
            snapshot
                .items
                .iter()
                .find(|item| item.id == 102)
                .unwrap()
                .is_pinned
        );

        let content = history.content(104).unwrap();
        assert!(content.available);
        assert_eq!(
            content.content,
            "中文、emoji 🚀 和 UTF-8 必须完整穿过 Rust/C++ 边界。"
        );

        let delete_result = history.apply_action(101, "delete").unwrap();
        assert_eq!(delete_result.effective_id, None);
        assert!(delete_result.removed);
        assert_eq!(delete_result.generation, 3);
        assert!(!history
            .snapshot("")
            .unwrap()
            .items
            .iter()
            .any(|item| item.id == 101));
    }

    #[test]
    fn memory_history_ingests_full_text_without_trimming() {
        let mut history = ClipboardHistory::in_memory(vec![]);
        let ingested = history
            .ingest_text("  keep whitespace\nline two  ".to_owned(), "Notepad")
            .unwrap();

        assert_eq!(ingested.action, "ingest-text");
        assert_eq!(ingested.effective_id, Some(1));
        assert_eq!(
            history.snapshot("").unwrap().items[0].preview,
            "  keep whitespace\nline two  "
        );
        assert_eq!(
            history.content(1).unwrap().content,
            "  keep whitespace\nline two  "
        );
        assert_eq!(history.snapshot("").unwrap().items[0].content_type, "text");

        let url = history
            .ingest_text("https://example.com/path".to_owned(), "Edge")
            .unwrap();
        assert_eq!(url.action, "ingest-url");
        assert_eq!(
            history
                .snapshot("")
                .unwrap()
                .items
                .iter()
                .find(|item| item.id == url.effective_id.unwrap())
                .unwrap()
                .content_type,
            "url"
        );

        let rich = history
            .ingest(
                CapturedPayload::RichText {
                    content: "hello".to_owned(),
                    html: "<b>hello</b>".to_owned(),
                },
                "Word",
            )
            .unwrap();
        let rich_content = history.content(rich.effective_id.unwrap()).unwrap();
        assert_eq!(rich_content.content_type, "rich_text");
        assert_eq!(rich_content.html_content.as_deref(), Some("<b>hello</b>"));
    }

    #[test]
    fn memory_tag_updates_are_normalized_searchable_and_private() {
        let mut history = ClipboardHistory::synthetic();

        let tagged = history
            .update_tags(
                102,
                vec![
                    "  发布  ".to_owned(),
                    "发布".to_owned(),
                    "密码".to_owned(),
                ],
            )
            .unwrap();

        assert_eq!(tagged.action, "update-tags");
        assert_eq!(tagged.effective_id, Some(102));
        assert_eq!(tagged.replacement_id, None);
        let item = history
            .snapshot("发布")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == 102)
            .unwrap();
        assert_eq!(item.tags, vec!["发布", "密码"]);
        assert!(item.is_sensitive);
        assert_eq!(item.preview, SENSITIVE_PREVIEW);
        assert!(!history.content(102).unwrap().available);

        history
            .update_tags(102, vec!["发布".to_owned()])
            .unwrap();
        let item = history
            .snapshot("发布")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == 102)
            .unwrap();
        assert!(!item.is_sensitive);
        assert!(history.content(102).unwrap().available);
    }

    #[test]
    fn memory_pinned_reorder_requires_the_complete_stable_id_set() {
        let mut history = ClipboardHistory::synthetic();
        history.apply_action(102, "pin").unwrap();
        history.apply_action(103, "pin").unwrap();

        let reordered = history.reorder_pinned(vec![103, 101, 102]).unwrap();

        assert_eq!(reordered.action, "reorder-pinned");
        assert_eq!(reordered.ordered_ids, vec![103, 101, 102]);
        assert_eq!(reordered.generation, 4);
        let pinned: Vec<i64> = history
            .snapshot("")
            .unwrap()
            .items
            .into_iter()
            .filter(|item| item.is_pinned)
            .map(|item| item.id)
            .collect();
        assert_eq!(pinned, vec![103, 101, 102]);

        let error = history.reorder_pinned(vec![103, 101]).unwrap_err();
        assert_eq!(error.kind(), HistoryErrorKind::NotFound);
        assert_eq!(history.snapshot("").unwrap().generation, 4);
    }

    #[test]
    fn sqlite_history_reads_production_schema_without_writing() {
        let path = temporary_database("read-only");
        create_test_database(&path);
        let bytes_before = fs::read(&path).unwrap();
        let history = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();

        let snapshot = history.snapshot("").unwrap();

        assert_eq!(snapshot.adapter, "sqlite-read-only");
        assert!(snapshot.read_only);
        assert_eq!(snapshot.items.len(), 3);
        assert_eq!(snapshot.items[0].id, 2);
        assert_eq!(snapshot.items[0].preview, SENSITIVE_PREVIEW);
        assert!(snapshot.items[0].is_sensitive);
        assert_eq!(snapshot.items[1].id, 3);
        assert_eq!(snapshot.items[1].preview, SENSITIVE_PREVIEW);
        assert!(snapshot.items[1].is_sensitive);
        assert_eq!(history.snapshot("Notepad").unwrap().items.len(), 1);

        let content = history.content(1).unwrap();
        assert!(content.available);
        assert_eq!(content.content_type, "text");
        assert_eq!(content.content, "hello\nfull content");
        assert_eq!(content.html_content, None);

        let encrypted = history.content(2).unwrap();
        assert!(!encrypted.available);
        assert!(encrypted.is_sensitive);
        assert!(encrypted.content.is_empty());
        assert!(encrypted
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("decryption"));

        let tagged = history.content(3).unwrap();
        assert!(!tagged.available);
        assert!(tagged.is_sensitive);
        assert!(tagged.content.is_empty());
        assert!(Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).is_ok());
        assert_eq!(fs::read(&path).unwrap(), bytes_before);

        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn writable_sqlite_history_decrypts_sensitive_payloads_without_revealing_read_only_copies() {
        let path = temporary_database("encrypted-content");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        let encrypted_content = crate::encryption::encrypt_value("隐私正文").unwrap();
        let encrypted_preview = crate::encryption::encrypt_value("隐私预览").unwrap();
        let encrypted_html = crate::encryption::encrypt_value("<b>隐私正文</b>").unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, html_content, source_app, timestamp, preview,
                     is_pinned, pinned_order, tags, content_hash, content_hash_version)
                 VALUES (4, 'rich_text', ?1, ?2, '密码管理器', 30, ?3, 0, 0,
                         '[\"sensitive\"]', ?4, 2)",
                params![
                    encrypted_content,
                    encrypted_html,
                    encrypted_preview,
                    calc_text_hash("隐私正文") as i64,
                ],
            )
            .unwrap();
        drop(connection);

        let writable = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let content = writable.content(4).unwrap();
        assert!(content.available);
        assert!(content.is_sensitive);
        assert_eq!(content.content, "隐私正文");
        assert_eq!(content.html_content.as_deref(), Some("<b>隐私正文</b>"));
        drop(writable);

        let read_only = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();
        let protected = read_only.content(4).unwrap();
        assert!(!protected.available);
        assert!(protected.is_sensitive);
        assert!(protected.content.is_empty());
        drop(read_only);
        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn sqlite_tag_updates_atomically_apply_privacy_and_remove_plaintext_ocr() {
        let path = temporary_database("secure-tags");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_image_analysis
                    (entry_id, content_hash, ocr_text, qr_codes, analyzed_at)
                 VALUES (1, 7, 'plain OCR', '[]', 1)",
                [],
            )
            .unwrap();
        drop(connection);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let tagged = history
            .update_tags(
                1,
                vec![" 工作 ".to_owned(), "密码".to_owned(), "工作".to_owned()],
            )
            .unwrap();

        assert_eq!(tagged.effective_id, Some(1));
        let connection = Connection::open(&path).unwrap();
        let stored = connection
            .query_row(
                "SELECT content, preview, tags, content_hash, content_hash_version,
                        sync_updated_by
                 FROM clipboard_history WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .unwrap();
        assert!(stored.0.starts_with(ENCRYPT_PREFIX));
        assert!(stored.1.starts_with(ENCRYPT_PREFIX));
        assert_eq!(stored.2, "[\"工作\",\"密码\"]");
        assert_eq!(stored.3, calc_text_hash("hello\nfull content") as i64);
        assert_eq!(stored.4, 2);
        assert_eq!(stored.5, "winui-test");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_image_analysis WHERE entry_id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        let tags = connection
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = 1 ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(tags, vec!["工作", "密码"]);
        drop(connection);

        let snapshot = history.snapshot("工作").unwrap();
        let item = snapshot.items.iter().find(|item| item.id == 1).unwrap();
        assert_eq!(item.tags, vec!["工作", "密码"]);
        assert!(item.is_sensitive);
        assert_eq!(history.content(1).unwrap().content, "hello\nfull content");

        history.update_tags(1, vec!["工作".to_owned()]).unwrap();
        let connection = Connection::open(&path).unwrap();
        let decrypted = connection
            .query_row(
                "SELECT content, preview, tags FROM clipboard_history WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(decrypted.0, "hello\nfull content");
        assert_eq!(decrypted.1, "hello world");
        assert_eq!(decrypted.2, "[\"工作\"]");
        assert!(!history
            .snapshot("")
            .unwrap()
            .items
            .iter()
            .find(|item| item.id == 1)
            .unwrap()
            .is_sensitive);

        drop(connection);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_tag_update_rolls_back_metadata_when_normalized_tag_write_fails() {
        let path = temporary_database("tag-rollback");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_winui_tag BEFORE INSERT ON entry_tags
                 BEGIN SELECT RAISE(ABORT, 'reject tag'); END;",
            )
            .unwrap();
        drop(connection);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let error = history
            .update_tags(1, vec!["blocked".to_owned()])
            .unwrap_err();

        assert_eq!(error.kind(), HistoryErrorKind::Storage);
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT tags FROM clipboard_history WHERE id = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "[]"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM entry_tags WHERE entry_id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );

        drop(connection);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn writable_sqlite_history_fails_closed_for_foreign_dpapi_payloads() {
        let path = temporary_database("foreign-encrypted-content");
        create_test_database(&path);
        let history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let content = history.content(2).unwrap();

        assert!(!content.available);
        assert!(content.is_sensitive);
        assert!(content.content.is_empty());
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_history_rejects_mutation() {
        let path = temporary_database("mutation");
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();

        let error = history.apply_action(1, "delete").unwrap_err();

        assert_eq!(error.kind(), HistoryErrorKind::ReadOnly);
        assert_eq!(history.snapshot("").unwrap().items.len(), 3);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn content_lookup_reports_missing_entries() {
        let history = ClipboardHistory::synthetic();

        let error = history.content(999).unwrap_err();

        assert_eq!(error.kind(), HistoryErrorKind::NotFound);
    }

    #[test]
    fn synthetic_history_exposes_html_for_rich_paste() {
        let history = ClipboardHistory::synthetic();
        let content = history.content(101).unwrap();

        assert_eq!(
            content.html_content.as_deref(),
            Some("<p><b>WinUI 3</b> main-window probe is connected to Rust through a C ABI.</p>")
        );
        assert_eq!(history.content(102).unwrap().html_content, None);
    }

    #[test]
    fn memory_type_filter_is_exact_and_redacts_sensitive_preview() {
        let history = ClipboardHistory::synthetic();

        let typed = history.snapshot("type:text").unwrap();
        assert!(typed.items.iter().all(|item| item.content_type == "text"));
        assert!(typed.items.iter().any(|item| item.id == 101));
        assert!(typed.items.iter().any(|item| item.id == 107));
        assert!(!typed.items.iter().any(|item| item.id == 102));

        let sensitive = history
            .snapshot("")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == 107)
            .unwrap();
        assert!(sensitive.is_sensitive);
        assert_eq!(sensitive.preview, SENSITIVE_PREVIEW);
        assert!(!history
            .snapshot("")
            .unwrap()
            .items
            .iter()
            .any(|item| item.preview.contains("hunter2")));

        let content = history.content(107).unwrap();
        assert!(!content.available);
        assert!(content.is_sensitive);
    }

    #[test]
    fn sqlite_history_writes_pin_and_delete_with_stable_ids() {
        let path = temporary_database("write");
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let pin = history.apply_action(1, "pin").unwrap();
        assert_eq!(pin.adapter, "sqlite");
        assert!(!history.snapshot("").unwrap().read_only);
        assert_eq!(pin.replacement_id, None);
        assert_eq!(pin.effective_id, Some(1));
        assert!(
            history
                .snapshot("")
                .unwrap()
                .items
                .iter()
                .find(|item| item.id == 1)
                .unwrap()
                .is_pinned
        );
        let connection = Connection::open(&path).unwrap();
        let sync_revision: (i64, String) = connection
            .query_row(
                "SELECT sync_updated_at, sync_updated_by FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(sync_revision.0 > 0);
        assert_eq!(sync_revision.1, "winui-test");
        drop(connection);

        drop(history);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        assert!(
            history
                .snapshot("")
                .unwrap()
                .items
                .iter()
                .find(|item| item.id == 1)
                .unwrap()
                .is_pinned
        );

        let delete = history.apply_action(1, "delete").unwrap();
        assert!(delete.removed);
        assert_eq!(delete.effective_id, None);
        assert_eq!(delete.replacement_id, None);
        assert!(!history
            .snapshot("")
            .unwrap()
            .items
            .iter()
            .any(|item| item.id == 1));

        drop(history);
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT hash_version FROM cloud_sync_tombstones
                     WHERE content_type = 'text' AND content_hash = ?1",
                    [calc_text_hash("hello\nfull content") as i64],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        drop(connection);
        let history = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();
        assert!(!history
            .snapshot("")
            .unwrap()
            .items
            .iter()
            .any(|item| item.id == 1));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_pinned_reorder_is_atomic_and_advances_sync_metadata() {
        let path = temporary_database("pinned-reorder");
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        history.apply_action(1, "pin").unwrap();

        let reordered = history.reorder_pinned(vec![2, 1]).unwrap();

        assert_eq!(reordered.ordered_ids, vec![2, 1]);
        let snapshot_pinned: Vec<i64> = history
            .snapshot("")
            .unwrap()
            .items
            .into_iter()
            .filter(|item| item.is_pinned)
            .map(|item| item.id)
            .collect();
        assert_eq!(snapshot_pinned, vec![2, 1]);
        let connection = Connection::open(&path).unwrap();
        let orders = connection
            .prepare(
                "SELECT id, pinned_order, sync_updated_by
                 FROM clipboard_history WHERE id IN (1, 2) ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(orders, vec![(1, 1, "winui-test".to_owned()), (2, 2, "winui-test".to_owned())]);
        connection
            .execute_batch(
                "CREATE TRIGGER reject_second_pin_order
                 BEFORE UPDATE OF pinned_order ON clipboard_history
                 WHEN NEW.id = 2 AND NEW.pinned_order <> OLD.pinned_order
                 BEGIN SELECT RAISE(ABORT, 'reject pinned order'); END;",
            )
            .unwrap();
        drop(connection);
        let generation_before = history.snapshot("").unwrap().generation;

        let error = history.reorder_pinned(vec![1, 2]).unwrap_err();

        assert_eq!(error.kind(), HistoryErrorKind::Storage);
        assert_eq!(history.snapshot("").unwrap().generation, generation_before);
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT pinned_order FROM clipboard_history WHERE id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT pinned_order FROM clipboard_history WHERE id = 2",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );

        drop(connection);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_type_filter_matches_content_type_exactly() {
        let path = temporary_database("type-filter");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, pinned_order, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    4,
                    "code",
                    "fn main() {}",
                    Option::<String>::None,
                    "VS Code",
                    5,
                    "preview mentions text without being a text item",
                    0,
                    0,
                    "[]"
                ],
            )
            .unwrap();
        drop(connection);

        let history = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();
        let code = history.snapshot("type:code").unwrap();
        assert_eq!(code.items.len(), 1);
        assert_eq!(code.items[0].id, 4);
        assert_eq!(history.snapshot("type:text").unwrap().items.len(), 3);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_search_matches_cached_ocr_and_qr_without_exposing_payloads() {
        let path = temporary_database("ocr-search");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, source_app, timestamp, preview, tags)
                 VALUES (4, 'image', 'C:\\images\\receipt.png', 'Snipping Tool', 30,
                         '图片预览', '[]')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_image_analysis
                    (entry_id, content_hash, ocr_text, qr_codes, language, analyzed_at)
                 VALUES (4, 0, '发票号码 20260815', '[\"https://example.com/pay\"]',
                         'zh-CN', 31)",
                [],
            )
            .unwrap();
        drop(connection);
        let history = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();

        assert_eq!(history.snapshot("发票号码").unwrap().items[0].id, 4);
        assert_eq!(
            history.snapshot("type:image 20260815").unwrap().items[0].id,
            4
        );
        assert_eq!(history.snapshot("example.com/pay").unwrap().items[0].id, 4);

        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_search_remains_compatible_without_analysis_table() {
        let path = temporary_database("no-ocr-table");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("DROP TABLE clipboard_image_analysis", [])
            .unwrap();
        drop(connection);
        let history = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();

        let snapshot = history.snapshot("hello").unwrap();

        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].id, 1);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_history_ingests_text_and_rejects_read_only() {
        let path = temporary_database("ingest");
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let ingested = history
            .ingest_text("live copy\r\nsecond".to_owned(), "Notepad")
            .unwrap();
        let id = ingested.effective_id.unwrap();
        assert_eq!(history.content(id).unwrap().content, "live copy\nsecond");
        assert!(history
            .snapshot("")
            .unwrap()
            .items
            .iter()
            .any(|item| item.id == id));
        let connection = Connection::open(&path).unwrap();
        let identity: (i64, i64, i64, String) = connection
            .query_row(
                "SELECT content_hash, content_hash_version, sync_updated_at, sync_updated_by
                 FROM clipboard_history WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(identity.0, calc_text_hash("live copy\nsecond") as i64);
        assert_eq!(identity.1, 2);
        assert!(identity.2 > 0);
        assert_eq!(identity.3, "winui-test");
        drop(connection);

        drop(history);
        let mut read_only = ClipboardHistory::open_sqlite_read_only(path.clone()).unwrap();
        assert_eq!(
            read_only
                .ingest_text("blocked".to_owned(), "Notepad")
                .unwrap_err()
                .kind(),
            HistoryErrorKind::ReadOnly
        );
        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn sqlite_history_encrypts_new_sensitive_capture_using_database_privacy_settings() {
        let path = temporary_database("privacy-capture");
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let plain = "token=abcdefghijklmnop1234";

        let result = history
            .ingest(
                CapturedPayload::RichText {
                    content: plain.to_owned(),
                    html: format!("<b>{plain}</b>"),
                },
                "终端",
            )
            .unwrap();
        let entry_id = result.effective_id.unwrap();

        let snapshot_item = history
            .snapshot("")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == entry_id)
            .unwrap();
        assert!(snapshot_item.is_sensitive);
        assert_eq!(snapshot_item.preview, SENSITIVE_PREVIEW);
        let resolved = history.content(entry_id).unwrap();
        assert!(resolved.available);
        assert!(resolved.is_sensitive);
        assert_eq!(resolved.content, plain);
        assert_eq!(resolved.html_content, Some(format!("<b>{plain}</b>")));

        let connection = Connection::open(&path).unwrap();
        let stored: (String, String, Option<String>, String, i64) = connection
            .query_row(
                "SELECT content, preview, html_content, tags, content_hash
                 FROM clipboard_history WHERE id = ?1",
                [entry_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert!(stored.0.starts_with(ENCRYPT_PREFIX));
        assert!(stored.1.starts_with(ENCRYPT_PREFIX));
        assert!(stored.2.unwrap().starts_with(ENCRYPT_PREFIX));
        assert_eq!(stored.3, "[\"sensitive\"]");
        assert_eq!(stored.4, calc_text_hash(plain) as i64);
        assert_eq!(
            connection
                .query_row(
                    "SELECT tag FROM entry_tags WHERE entry_id = ?1",
                    [entry_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "sensitive"
        );
        drop(connection);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_history_respects_disabled_privacy_protection() {
        let path = temporary_database("privacy-disabled");
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO settings (key, value)
                 VALUES ('app.privacy_protection', 'false')",
                [],
            )
            .unwrap();
        drop(connection);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let plain = "person@example.com";

        let result = history
            .ingest_text(plain.to_owned(), "邮件")
            .unwrap();
        let entry_id = result.effective_id.unwrap();

        let connection = Connection::open(&path).unwrap();
        let stored: (String, String) = connection
            .query_row(
                "SELECT content, tags FROM clipboard_history WHERE id = ?1",
                [entry_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, (plain.to_owned(), "[]".to_owned()));
        drop(connection);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_history_persists_captured_images_beside_the_database() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tiez-core-image-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("clipboard.db");
        let source = root.join("captured.png");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lW2ZAAAAAElFTkSuQmCC";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap();
        fs::write(&source, &bytes).unwrap();
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let result = history
            .ingest(
                CapturedPayload::Image {
                    content: source.to_string_lossy().into_owned(),
                },
                "截图工具",
            )
            .unwrap();
        let entry_id = result.effective_id.unwrap();
        let stored = history.content(entry_id).unwrap().content;
        let stored_path = PathBuf::from(&stored);

        assert_eq!(
            stored_path.parent(),
            Some(root.join("attachments").as_path())
        );
        assert_eq!(fs::read(&stored_path).unwrap(), bytes);
        let connection = Connection::open(&path).unwrap();
        let identity: (i64, i64) = connection
            .query_row(
                "SELECT is_external, content_hash FROM clipboard_history WHERE id = ?1",
                [entry_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(identity.0, 1);
        assert_eq!(identity.1, calc_image_hash(&stored).unwrap());
        drop(connection);
        drop(history);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_image_ingest_removes_the_new_attachment() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tiez-core-image-failure-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("clipboard.db");
        let source = root.join("captured.png");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lW2ZAAAAAElFTkSuQmCC";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap();
        fs::write(&source, bytes).unwrap();
        create_test_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_image_history BEFORE INSERT ON clipboard_history
                 WHEN NEW.content_type = 'image'
                 BEGIN SELECT RAISE(ABORT, 'reject image'); END;",
            )
            .unwrap();
        drop(connection);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let error = history
            .ingest(
                CapturedPayload::Image {
                    content: source.to_string_lossy().into_owned(),
                },
                "截图工具",
            )
            .unwrap_err();

        assert_eq!(error.kind(), HistoryErrorKind::Storage);
        let attachments = root.join("attachments");
        assert!(attachments.is_dir());
        assert!(fs::read_dir(attachments).unwrap().next().is_none());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            3
        );
        drop(connection);
        drop(history);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_session_mode_reuses_negative_ids_and_persists_only_when_pinned() {
        let path = temporary_database("session-mode");
        create_test_database(&path);
        set_test_setting(&path, "app.persistent", "false");
        set_test_setting(&path, "app.deduplicate", "true");
        let initial_count = Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let first = history
            .ingest_text("session value".to_owned(), "Notepad")
            .unwrap();
        let session_id = first.effective_id.unwrap();
        assert!(session_id < 0);
        assert_eq!(sqlite_session_len(&history), 1);
        assert_eq!(history.content(session_id).unwrap().content, "session value");

        let duplicate = history
            .ingest_text("session value".to_owned(), "Terminal")
            .unwrap();
        assert_eq!(duplicate.effective_id, Some(session_id));
        assert_eq!(sqlite_session_len(&history), 1);
        let session_item = history
            .snapshot("")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == session_id)
            .unwrap();
        assert_eq!(session_item.source_app, "Terminal");
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            initial_count
        );

        let pinned = history.apply_action(session_id, "pin").unwrap();
        let stable_id = pinned.replacement_id.unwrap();
        assert!(stable_id > 0);
        assert_eq!(pinned.effective_id, Some(stable_id));
        assert_eq!(sqlite_session_len(&history), 0);
        let persisted_pin = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT is_pinned, pinned_order FROM clipboard_history WHERE id = ?1",
                [stable_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(persisted_pin.0, 1);
        assert!(persisted_pin.1 > 0);

        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn sensitive_session_entry_stays_in_memory_until_pin_encrypts_it() {
        let path = temporary_database("sensitive-session");
        create_test_database(&path);
        set_test_setting(&path, "app.persistent", "false");
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let plain = "token=abcdefghijklmnop1234";

        let session_id = history
            .ingest_text(plain.to_owned(), "终端")
            .unwrap()
            .effective_id
            .unwrap();
        assert!(session_id < 0);
        let snapshot = history
            .snapshot("")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == session_id)
            .unwrap();
        assert!(snapshot.is_sensitive);
        assert_eq!(snapshot.preview, SENSITIVE_PREVIEW);
        let content = history.content(session_id).unwrap();
        assert!(content.available);
        assert!(content.is_sensitive);
        assert_eq!(content.content, plain);

        let stable_id = history
            .apply_action(session_id, "pin")
            .unwrap()
            .replacement_id
            .unwrap();
        let stored = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT content, preview, tags FROM clipboard_history WHERE id = ?1",
                [stable_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert!(stored.0.starts_with(ENCRYPT_PREFIX));
        assert!(stored.1.starts_with(ENCRYPT_PREFIX));
        assert_eq!(stored.2, "[\"sensitive\"]");

        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn tagging_a_session_entry_persists_a_secure_positive_replacement() {
        let path = temporary_database("tagged-session");
        create_test_database(&path);
        set_test_setting(&path, "app.persistent", "false");
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let session_id = history
            .ingest_text("daily secret".to_owned(), "记事本")
            .unwrap()
            .effective_id
            .unwrap();
        assert!(session_id < 0);

        let tagged = history
            .update_tags(
                session_id,
                vec!["工作".to_owned(), "sensitive".to_owned()],
            )
            .unwrap();

        let replacement_id = tagged.replacement_id.unwrap();
        assert!(replacement_id > 0);
        assert_eq!(tagged.effective_id, Some(replacement_id));
        assert_eq!(sqlite_session_len(&history), 0);
        assert_eq!(
            history.content(session_id).unwrap_err().kind(),
            HistoryErrorKind::NotFound
        );
        let connection = Connection::open(&path).unwrap();
        let stored = connection
            .query_row(
                "SELECT content, preview, tags FROM clipboard_history WHERE id = ?1",
                [replacement_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert!(stored.0.starts_with(ENCRYPT_PREFIX));
        assert!(stored.1.starts_with(ENCRYPT_PREFIX));
        assert_eq!(stored.2, "[\"工作\",\"sensitive\"]");
        let item = history
            .snapshot("工作")
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.id == replacement_id)
            .unwrap();
        assert!(item.is_sensitive);
        assert_eq!(item.tags, vec!["工作", "sensitive"]);

        drop(connection);
        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_session_mode_caps_history_at_five_hundred_entries() {
        let path = temporary_database("session-cap");
        create_test_database(&path);
        set_test_setting(&path, "app.persistent", "false");
        set_test_setting(&path, "app.deduplicate", "false");
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let first_id = history
            .ingest_text("session 0".to_owned(), "Notepad")
            .unwrap()
            .effective_id
            .unwrap();
        for index in 1..=500 {
            history
                .ingest_text(format!("session {index}"), "Notepad")
                .unwrap();
        }

        assert_eq!(sqlite_session_len(&history), SESSION_HISTORY_LIMIT);
        assert_eq!(
            history.content(first_id).unwrap_err().kind(),
            HistoryErrorKind::NotFound
        );
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            3
        );

        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_persistent_mode_reuses_existing_rows_and_enforces_unprotected_limit() {
        let path = temporary_database("persistent-dedup-limit");
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();

        let duplicate = history
            .ingest_text("hello\nfull content".to_owned(), "Terminal")
            .unwrap();
        assert_eq!(duplicate.effective_id, Some(1));
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            3
        );

        let connection = Connection::open(&path).unwrap();
        connection.execute("DELETE FROM entry_tags", []).unwrap();
        connection
            .execute("DELETE FROM clipboard_history", [])
            .unwrap();
        drop(connection);
        set_test_setting(&path, "app.deduplicate", "false");
        set_test_setting(&path, "app.persistent_limit", "2");
        let pinned_id = history
            .ingest_text("pinned".to_owned(), "Notepad")
            .unwrap()
            .effective_id
            .unwrap();
        history.apply_action(pinned_id, "pin").unwrap();
        let oldest_unprotected = history
            .ingest_text("oldest".to_owned(), "Notepad")
            .unwrap()
            .effective_id
            .unwrap();
        let retained = history
            .ingest_text("retained".to_owned(), "Notepad")
            .unwrap()
            .effective_id
            .unwrap();
        let newest = history
            .ingest_text("newest".to_owned(), "Notepad")
            .unwrap()
            .effective_id
            .unwrap();

        assert_eq!(
            history.content(oldest_unprotected).unwrap_err().kind(),
            HistoryErrorKind::NotFound
        );
        assert_eq!(history.content(pinned_id).unwrap().content, "pinned");
        assert_eq!(history.content(retained).unwrap().content, "retained");
        assert_eq!(history.content(newest).unwrap().content, "newest");
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            3
        );

        drop(history);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_delete_removes_only_the_last_reference_to_an_attachment() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tiez-core-attachment-cleanup-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("clipboard.db");
        let source = root.join("captured.png");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lW2ZAAAAAElFTkSuQmCC";
        fs::write(
            &source,
            base64::engine::general_purpose::STANDARD
                .decode(png)
                .unwrap(),
        )
        .unwrap();
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let original_id = history
            .ingest(
                CapturedPayload::Image {
                    content: source.to_string_lossy().into_owned(),
                },
                "截图工具",
            )
            .unwrap()
            .effective_id
            .unwrap();
        let attachment = history.content(original_id).unwrap().content;
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (content_type, content, source_app, timestamp, preview, is_external,
                     content_hash, content_hash_version, tags)
                 VALUES ('image', ?1, '共享引用', 1, 'shared', 1, ?2, 2, '[]')",
                params![attachment.as_str(), calc_image_hash(&attachment).unwrap()],
            )
            .unwrap();
        let shared_id = connection.last_insert_rowid();
        drop(connection);

        history.apply_action(original_id, "delete").unwrap();
        assert!(Path::new(&attachment).is_file());
        history.apply_action(shared_id, "delete").unwrap();
        assert!(!Path::new(&attachment).exists());

        drop(history);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attachment_cleanup_fails_closed_when_a_surviving_path_cannot_be_decrypted() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tiez-core-attachment-fail-closed-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("clipboard.db");
        let source = root.join("captured.png");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lW2ZAAAAAElFTkSuQmCC";
        fs::write(
            &source,
            base64::engine::general_purpose::STANDARD
                .decode(png)
                .unwrap(),
        )
        .unwrap();
        create_test_database(&path);
        let mut history = ClipboardHistory::open_sqlite_read_write(path.clone()).unwrap();
        let image_id = history
            .ingest(
                CapturedPayload::Image {
                    content: source.to_string_lossy().into_owned(),
                },
                "截图工具",
            )
            .unwrap()
            .effective_id
            .unwrap();
        let attachment = history.content(image_id).unwrap().content;
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (content_type, content, source_app, timestamp, preview, is_external, tags)
                 VALUES ('file', ?1, '旧版加密', 1, 'unreadable', 1, '[]')",
                [ENCRYPT_PREFIX],
            )
            .unwrap();
        drop(connection);

        history.apply_action(image_id, "delete").unwrap();
        assert!(Path::new(&attachment).is_file());

        drop(history);
        fs::remove_dir_all(root).unwrap();
    }
}
