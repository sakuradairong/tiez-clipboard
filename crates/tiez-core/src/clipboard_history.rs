use crate::clipboard_capture::{classify_snapshot, CapturedPayload, ClipboardSnapshot};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_HISTORY_LIMIT: i64 = 200;
const ENCRYPT_PREFIX: &str = "dpapi:";
const SENSITIVE_PREVIEW: &str = "Sensitive entry — open in the production TieZ UI";
const SENSITIVE_TAGS: &[&str] = &["sensitive", "密码", "password"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryItem {
    pub id: i64,
    pub content_type: String,
    pub preview: String,
    pub source_app: String,
    pub captured_at: String,
    pub is_pinned: bool,
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

        let (content_type, content, html_content, tags) = match result {
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
        if has_sensitive_tag(&tags) || is_encrypted {
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

        Ok(HistoryContent {
            id: entry_id,
            content_type,
            content,
            html_content,
            available: true,
            is_sensitive: false,
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
                self.toggle_pin(&connection, entry_id)?;
                (Some(entry_id), false, None)
            }
            "delete" => {
                let deleted = connection
                    .execute("DELETE FROM clipboard_history WHERE id = ?1", [entry_id])
                    .map_err(|error| storage_error("failed to delete clipboard entry", error))?;
                if deleted == 0 {
                    return Err(entry_not_found(entry_id));
                }
                self.last_action = format!("Entry {entry_id} deleted");
                (None, true, None)
            }
            "paste-plain" | "paste-rich" | "copy-plain" | "copy-rich" => {
                ensure_sqlite_entry(&connection, entry_id)?;
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
        let preview = payload.preview();
        let content = payload.content();
        let content_type = payload.content_type();
        let html = payload.html();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (content_type, content, html_content, source_app, timestamp, preview, is_pinned, pinned_order, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, '[]')",
                rusqlite::params![content_type, content, html, source_app, timestamp, preview],
            )
            .map_err(|error| storage_error("failed to ingest clipboard payload", error))?;
        let entry_id = connection.last_insert_rowid();
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

    fn toggle_pin(
        &mut self,
        connection: &Connection,
        entry_id: i64,
    ) -> Result<(), HistoryError> {
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
        let pinned_order: i64 = if new_pinned {
            connection
                .query_row(
                    "SELECT COALESCE(MAX(pinned_order), 0) + 1 FROM clipboard_history WHERE is_pinned = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(1)
        } else {
            0
        };

        connection
            .execute(
                "UPDATE clipboard_history SET is_pinned = ?1, pinned_order = ?2 WHERE id = ?3",
                rusqlite::params![i32::from(new_pinned), pinned_order, entry_id],
            )
            .map_err(|error| storage_error("failed to update pin state", error))?;

        self.last_action = format!(
            "Entry {entry_id} {}",
            if new_pinned { "pinned" } else { "unpinned" }
        );
        Ok(())
    }
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
        SnapshotQuery::Type {
            content_type,
            text,
        } if text.is_empty() => query_history(
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
        SnapshotQuery::Type {
            content_type,
            text,
        } => query_history(
            connection,
            "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
             FROM clipboard_history
             WHERE lower(content_type) = lower(?1)
               AND (
                    instr(lower(source_app), lower(?2)) > 0
                    OR instr(lower(preview), lower(?2)) > 0
               )
             ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
             LIMIT ?3",
            QueryArgs::Type {
                content_type,
                text: Some(text),
                limit,
            },
        ),
        SnapshotQuery::Text(text) => search_latest_history(connection, text, limit),
    }
}

fn search_latest_history(
    connection: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<HistoryItem>, HistoryError> {
    query_history(
        connection,
        "SELECT id, content_type, preview, source_app, timestamp, is_pinned, tags
         FROM clipboard_history
         WHERE instr(lower(content_type), lower(?1)) > 0
            OR instr(lower(source_app), lower(?1)) > 0
            OR instr(lower(preview), lower(?1)) > 0
         ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
         LIMIT ?2",
        QueryArgs::Search { query, limit },
    )
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
        let tags = row.get::<_, String>(6)?;
        Ok(HistoryItem {
            id: row.get(0)?,
            content_type: row.get(1)?,
            preview: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            source_app: row.get(3)?,
            captured_at: format_timestamp(timestamp),
            is_pinned: row.get::<_, i32>(5)? == 1,
            is_sensitive: has_sensitive_tag(&tags),
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
            .map_err(|error| {
                storage_error("failed to search clipboard history by type", error)
            })?,
    };

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage_error("failed to read clipboard history row", error))
}

fn has_sensitive_tag(tags_json: &str) -> bool {
    serde_json::from_str::<Vec<String>>(tags_json)
        .unwrap_or_default()
        .iter()
        .any(|tag| {
            SENSITIVE_TAGS
                .iter()
                .any(|sensitive| sensitive.eq_ignore_ascii_case(tag))
        })
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
            is_sensitive: false,
        },
        HistoryItem {
            id: 102,
            content_type: "code".to_owned(),
            preview: "tiez_core_get_snapshot_json(handle, query);".to_owned(),
            source_app: "Windows Terminal".to_owned(),
            captured_at: "1 minute ago".to_owned(),
            is_pinned: false,
            is_sensitive: false,
        },
        HistoryItem {
            id: 103,
            content_type: "url".to_owned(),
            preview: "https://github.com/jimuzhe/tiez-clipboard/issues/154".to_owned(),
            source_app: "Microsoft Edge".to_owned(),
            captured_at: "3 minutes ago".to_owned(),
            is_pinned: false,
            is_sensitive: false,
        },
        HistoryItem {
            id: 104,
            content_type: "text".to_owned(),
            preview: "中文、emoji 🚀 和 UTF-8 必须完整穿过 Rust/C++ 边界。".to_owned(),
            source_app: "TieZ".to_owned(),
            captured_at: "5 minutes ago".to_owned(),
            is_pinned: false,
            is_sensitive: false,
        },
        HistoryItem {
            id: 105,
            content_type: "image".to_owned(),
            preview: "Image preview placeholder · 1920 × 1080".to_owned(),
            source_app: "Snipping Tool".to_owned(),
            captured_at: "8 minutes ago".to_owned(),
            is_pinned: false,
            is_sensitive: false,
        },
        HistoryItem {
            id: 106,
            content_type: "files".to_owned(),
            preview: "release-notes.md\nTieZ-setup.exe".to_owned(),
            source_app: "File Explorer".to_owned(),
            captured_at: "12 minutes ago".to_owned(),
            is_pinned: false,
            is_sensitive: false,
        },
        HistoryItem {
            id: 107,
            content_type: "text".to_owned(),
            preview: "hunter2-should-not-appear-in-snapshot".to_owned(),
            source_app: "Password Manager".to_owned(),
            captured_at: "15 minutes ago".to_owned(),
            is_pinned: false,
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
        connection
            .execute_batch(
                "CREATE TABLE clipboard_history (
                    id INTEGER PRIMARY KEY,
                    content_type TEXT NOT NULL,
                    content TEXT NOT NULL,
                    html_content TEXT,
                    source_app TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    preview TEXT NOT NULL,
                    is_pinned INTEGER NOT NULL DEFAULT 0,
                    pinned_order INTEGER NOT NULL DEFAULT 0,
                    tags TEXT NOT NULL DEFAULT '[]'
                );",
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
}
