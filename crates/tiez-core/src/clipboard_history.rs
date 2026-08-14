use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
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
}

#[derive(Debug)]
struct SqliteReadOnlyHistory {
    database_path: PathBuf,
}

#[derive(Debug)]
enum HistoryAdapter {
    Memory(MemoryHistory),
    SqliteReadOnly(SqliteReadOnlyHistory),
}

/// Read/search clipboard history without exposing Tauri or storage details.
///
/// The module currently has two real adapters: mutable in-memory history for
/// development/tests and the production-schema SQLite reader opened with
/// read-only flags. Mutation support is intentionally limited to the memory
/// adapter until the production mutation and synchronization policy is
/// extracted behind its own module.
#[derive(Debug)]
pub struct ClipboardHistory {
    adapter: HistoryAdapter,
}

impl ClipboardHistory {
    pub fn synthetic() -> Self {
        Self::in_memory(sample_items())
    }

    pub fn in_memory(items: Vec<HistoryItem>) -> Self {
        Self {
            adapter: HistoryAdapter::Memory(MemoryHistory {
                generation: 1,
                last_action: "Rust memory adapter ready".to_owned(),
                items,
            }),
        }
    }

    pub fn open_sqlite_read_only(database_path: impl Into<PathBuf>) -> Result<Self, HistoryError> {
        let adapter = SqliteReadOnlyHistory::open(database_path.into())?;
        Ok(Self {
            adapter: HistoryAdapter::SqliteReadOnly(adapter),
        })
    }

    pub fn snapshot(&self, query: &str) -> Result<HistorySnapshot, HistoryError> {
        match &self.adapter {
            HistoryAdapter::Memory(adapter) => Ok(adapter.snapshot(query)),
            HistoryAdapter::SqliteReadOnly(adapter) => adapter.snapshot(query),
        }
    }

    pub fn content(&self, entry_id: i64) -> Result<HistoryContent, HistoryError> {
        match &self.adapter {
            HistoryAdapter::Memory(adapter) => adapter.content(entry_id),
            HistoryAdapter::SqliteReadOnly(adapter) => adapter.content(entry_id),
        }
    }

    pub fn apply_action(
        &mut self,
        entry_id: i64,
        action: &str,
    ) -> Result<HistoryMutationResult, HistoryError> {
        match &mut self.adapter {
            HistoryAdapter::Memory(adapter) => adapter.apply_action(entry_id, action),
            HistoryAdapter::SqliteReadOnly(_) => Err(HistoryError::new(
                HistoryErrorKind::ReadOnly,
                format!("action {action} is disabled for sqlite-read-only history"),
            )),
        }
    }
}

impl MemoryHistory {
    fn snapshot(&self, query: &str) -> HistorySnapshot {
        let items = filter_items(&self.items, query);
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
                self.last_action = format!("Entry {entry_id} deleted");
                (None, true)
            }
            "paste-plain" => {
                ensure_item_exists(&self.items, entry_id)?;
                self.last_action = format!("Plain-text paste requested for entry {entry_id}");
                (Some(entry_id), false)
            }
            "paste-rich" => {
                ensure_item_exists(&self.items, entry_id)?;
                self.last_action = format!("Rich paste requested for entry {entry_id}");
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
            content: item.preview.clone(),
            html_content: None,
            available: true,
            is_sensitive: false,
            unavailable_reason: None,
        })
    }
}

impl SqliteReadOnlyHistory {
    fn open(database_path: PathBuf) -> Result<Self, HistoryError> {
        if !database_path.is_file() {
            return Err(HistoryError::new(
                HistoryErrorKind::InvalidDatabase,
                format!(
                    "database path does not point to a clipboard database file: {}",
                    database_path.display()
                ),
            ));
        }

        let connection = open_read_only(&database_path)?;
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

        Ok(Self { database_path })
    }

    fn snapshot(&self, query: &str) -> Result<HistorySnapshot, HistoryError> {
        let connection = open_read_only(&self.database_path)?;
        let normalized_query = query.trim();
        let mut items = if normalized_query.is_empty() {
            load_latest_history(&connection, DEFAULT_HISTORY_LIMIT)?
        } else {
            search_latest_history(&connection, normalized_query, DEFAULT_HISTORY_LIMIT)?
        };

        for item in &mut items {
            if item.is_sensitive || item.preview.starts_with(ENCRYPT_PREFIX) {
                item.preview = SENSITIVE_PREVIEW.to_owned();
                item.is_sensitive = true;
            }
        }

        Ok(HistorySnapshot {
            adapter: "sqlite-read-only",
            read_only: true,
            generation: database_generation(&self.database_path),
            total: items.len(),
            query: query.to_owned(),
            last_action: format!("Read-only snapshot from {}", self.database_path.display()),
            items,
        })
    }

    fn content(&self, entry_id: i64) -> Result<HistoryContent, HistoryError> {
        let connection = open_read_only(&self.database_path)?;
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
}

fn open_read_only(path: &Path) -> Result<Connection, HistoryError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        storage_error(
            &format!("failed to open {} read-only", path.display()),
            error,
        )
    })
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
        None,
        limit,
    )
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
        Some(query),
        limit,
    )
}

fn query_history(
    connection: &Connection,
    sql: &str,
    query: Option<&str>,
    limit: i64,
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

    let rows = match query {
        Some(value) => statement
            .query_map(rusqlite::params![value, limit], map_row)
            .map_err(|error| storage_error("failed to search clipboard history", error))?,
        None => statement
            .query_map([limit], map_row)
            .map_err(|error| storage_error("failed to load clipboard history", error))?,
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

fn format_timestamp(timestamp: i64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(timestamp);
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
    let normalized_query = query.trim().to_lowercase();
    items
        .iter()
        .filter(|item| {
            normalized_query.is_empty()
                || item.preview.to_lowercase().contains(&normalized_query)
                || item.source_app.to_lowercase().contains(&normalized_query)
                || item.content_type.to_lowercase().contains(&normalized_query)
        })
        .cloned()
        .collect()
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
}
