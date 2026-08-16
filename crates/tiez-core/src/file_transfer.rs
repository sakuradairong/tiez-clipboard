//! Shared, Tauri-independent policy and persistence for LAN file transfer.
//!
//! The HTTP runtime belongs to the native host. This module owns the stable
//! settings contract and validates every value that may influence disk paths
//! or resource consumption.

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 12_345;
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CHUNKS: usize = 65_536;
pub const MAX_MESSAGES: usize = 500;

const ENABLED_KEY: &str = "file_server_enabled";
const PORT_KEY: &str = "file_server_port";
const PATH_KEY: &str = "file_transfer_path";
const AUTO_COPY_KEY: &str = "file_transfer_auto_copy";
const AUTO_OPEN_KEY: &str = "file_transfer_auto_open";
const AUTO_CLOSE_KEY: &str = "file_transfer_auto_close";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileTransferPreferencesSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub enabled: bool,
    pub port: u16,
    pub receive_directory: String,
    pub auto_copy: bool,
    pub auto_open: bool,
    pub auto_close: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct FileTransferPreferencesUpdate {
    pub enabled: Option<bool>,
    pub port: Option<u16>,
    pub receive_directory: Option<String>,
    pub auto_copy: Option<bool>,
    pub auto_open: Option<bool>,
    pub auto_close: Option<bool>,
}

#[derive(Debug)]
enum PreferencesAdapter {
    Memory(FileTransferValues),
    Sqlite {
        database_path: PathBuf,
        read_only: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileTransferValues {
    enabled: bool,
    port: u16,
    receive_directory: PathBuf,
    auto_copy: bool,
    auto_open: bool,
    auto_close: bool,
}

#[derive(Debug)]
pub struct FileTransferPreferences {
    adapter: PreferencesAdapter,
    default_receive_directory: PathBuf,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileTransferErrorKind {
    InvalidDatabase,
    Storage,
    Validation,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTransferError {
    kind: FileTransferErrorKind,
    message: String,
}

impl FileTransferError {
    fn new(kind: FileTransferErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> FileTransferErrorKind {
        self.kind
    }
}

impl fmt::Display for FileTransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FileTransferError {}

impl FileTransferPreferences {
    pub fn in_memory(default_receive_directory: impl Into<PathBuf>) -> Self {
        let default_receive_directory = default_receive_directory.into();
        Self {
            adapter: PreferencesAdapter::Memory(FileTransferValues::defaults(
                default_receive_directory.clone(),
            )),
            default_receive_directory,
            generation: 1,
        }
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        default_receive_directory: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, FileTransferError> {
        let database_path = database_path.into();
        let connection = open_connection(&database_path, read_only)?;
        connection
            .query_row("SELECT 1 FROM settings LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(|error| storage_error("无法读取设置表", error))?;
        Ok(Self {
            adapter: PreferencesAdapter::Sqlite {
                database_path,
                read_only,
            },
            default_receive_directory: default_receive_directory.into(),
            generation: 1,
        })
    }

    pub fn snapshot(&self) -> Result<FileTransferPreferencesSnapshot, FileTransferError> {
        let (adapter, read_only, values) = match &self.adapter {
            PreferencesAdapter::Memory(values) => ("memory", false, values.clone()),
            PreferencesAdapter::Sqlite {
                database_path,
                read_only,
            } => {
                let connection = open_connection(database_path, *read_only)?;
                let values = FileTransferValues {
                    enabled: read_bool(&connection, ENABLED_KEY, false)?,
                    port: read_port(&connection)?,
                    receive_directory: read_directory(
                        &connection,
                        &self.default_receive_directory,
                    )?,
                    auto_copy: read_bool(&connection, AUTO_COPY_KEY, false)?,
                    auto_open: read_bool(&connection, AUTO_OPEN_KEY, false)?,
                    auto_close: read_bool(&connection, AUTO_CLOSE_KEY, false)?,
                };
                (
                    if *read_only {
                        "sqlite-read-only"
                    } else {
                        "sqlite"
                    },
                    *read_only,
                    values,
                )
            }
        };
        Ok(FileTransferPreferencesSnapshot {
            adapter,
            read_only,
            generation: self.generation,
            enabled: values.enabled,
            port: values.port,
            receive_directory: values.receive_directory.to_string_lossy().into_owned(),
            auto_copy: values.auto_copy,
            auto_open: values.auto_open,
            auto_close: values.auto_close,
        })
    }

    pub fn update(
        &mut self,
        update: FileTransferPreferencesUpdate,
    ) -> Result<FileTransferPreferencesSnapshot, FileTransferError> {
        let mut values = self.values()?;
        apply_update(&mut values, update)?;
        match &mut self.adapter {
            PreferencesAdapter::Memory(current) => *current = values,
            PreferencesAdapter::Sqlite {
                database_path: _,
                read_only: true,
            } => {
                return Err(FileTransferError::new(
                    FileTransferErrorKind::ReadOnly,
                    "当前数据库为只读，不能修改文件传输设置",
                ));
            }
            PreferencesAdapter::Sqlite { database_path, .. } => {
                let mut connection = open_connection(database_path, false)?;
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|error| storage_error("无法开始文件传输设置事务", error))?;
                write_setting(&transaction, ENABLED_KEY, bool_text(values.enabled))?;
                write_setting(&transaction, PORT_KEY, &values.port.to_string())?;
                write_setting(
                    &transaction,
                    PATH_KEY,
                    &values.receive_directory.to_string_lossy(),
                )?;
                write_setting(&transaction, AUTO_COPY_KEY, bool_text(values.auto_copy))?;
                write_setting(&transaction, AUTO_OPEN_KEY, bool_text(values.auto_open))?;
                write_setting(&transaction, AUTO_CLOSE_KEY, bool_text(values.auto_close))?;
                transaction
                    .commit()
                    .map_err(|error| storage_error("无法保存文件传输设置", error))?;
            }
        }
        self.generation = self.generation.saturating_add(1);
        self.snapshot()
    }

    fn values(&self) -> Result<FileTransferValues, FileTransferError> {
        let snapshot = self.snapshot()?;
        Ok(FileTransferValues {
            enabled: snapshot.enabled,
            port: snapshot.port,
            receive_directory: PathBuf::from(snapshot.receive_directory),
            auto_copy: snapshot.auto_copy,
            auto_open: snapshot.auto_open,
            auto_close: snapshot.auto_close,
        })
    }
}

impl FileTransferValues {
    fn defaults(receive_directory: PathBuf) -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            receive_directory,
            auto_copy: false,
            auto_open: false,
            auto_close: false,
        }
    }
}

fn apply_update(
    values: &mut FileTransferValues,
    update: FileTransferPreferencesUpdate,
) -> Result<(), FileTransferError> {
    if let Some(enabled) = update.enabled {
        values.enabled = enabled;
    }
    if let Some(port) = update.port {
        if port == 0 {
            return Err(validation_error("端口必须在 1 到 65535 之间"));
        }
        values.port = port;
    }
    if let Some(directory) = update.receive_directory {
        let trimmed = directory.trim();
        if trimmed.is_empty() || trimmed.len() > 1024 {
            return Err(validation_error("接收目录不能为空且不能超过 1024 个字符"));
        }
        values.receive_directory = PathBuf::from(trimmed);
    }
    if let Some(auto_copy) = update.auto_copy {
        values.auto_copy = auto_copy;
    }
    if let Some(auto_open) = update.auto_open {
        values.auto_open = auto_open;
    }
    if let Some(auto_close) = update.auto_close {
        values.auto_close = auto_close;
    }
    Ok(())
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection, FileTransferError> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    };
    Connection::open_with_flags(path, flags).map_err(|error| {
        FileTransferError::new(
            FileTransferErrorKind::InvalidDatabase,
            format!("无法打开文件传输设置数据库 {}：{error}", path.display()),
        )
    })
}

fn read_setting(connection: &Connection, key: &str) -> Result<Option<String>, FileTransferError> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| storage_error(&format!("无法读取设置 {key}"), error))
}

fn read_bool(
    connection: &Connection,
    key: &str,
    fallback: bool,
) -> Result<bool, FileTransferError> {
    Ok(match read_setting(connection, key)?.as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => fallback,
    })
}

fn read_port(connection: &Connection) -> Result<u16, FileTransferError> {
    Ok(read_setting(connection, PORT_KEY)?
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_PORT))
}

fn read_directory(connection: &Connection, fallback: &Path) -> Result<PathBuf, FileTransferError> {
    Ok(read_setting(connection, PATH_KEY)?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && value.len() <= 1024)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.to_path_buf()))
}

fn write_setting(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    value: &str,
) -> Result<(), FileTransferError> {
    transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)\n             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, value),
        )
        .map(|_| ())
        .map_err(|error| storage_error(&format!("无法写入设置 {key}"), error))
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn validation_error(message: impl Into<String>) -> FileTransferError {
    FileTransferError::new(FileTransferErrorKind::Validation, message)
}

fn storage_error(context: &str, error: rusqlite::Error) -> FileTransferError {
    FileTransferError::new(
        FileTransferErrorKind::Storage,
        format!("{context}：{error}"),
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferMessage {
    pub id: u64,
    pub direction: String,
    pub msg_type: String,
    pub content: String,
    pub timestamp: i64,
    pub sender_id: String,
    pub sender_name: String,
    pub file_path: Option<String>,
}

#[derive(Debug)]
pub struct TransferMessageStore {
    messages: VecDeque<TransferMessage>,
    next_id: u64,
    capacity: usize,
}

impl Default for TransferMessageStore {
    fn default() -> Self {
        Self::new(MAX_MESSAGES)
    }
}

impl TransferMessageStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            next_id: 1,
            capacity: capacity.max(1),
        }
    }

    pub fn push(&mut self, mut message: TransferMessage) -> TransferMessage {
        message.id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.messages.push_back(message.clone());
        while self.messages.len() > self.capacity {
            self.messages.pop_front();
        }
        message
    }

    pub fn since(&self, after_id: u64) -> Vec<TransferMessage> {
        self.messages
            .iter()
            .filter(|message| message.id > after_id)
            .cloned()
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferFileKind {
    Image,
    Video,
    File,
}

impl TransferFileKind {
    pub fn message_type(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::File => "file",
        }
    }
}

pub fn classify_file_name(file_name: &str) -> TransferFileKind {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "avif" | "heic"
    ) {
        TransferFileKind::Image
    } else if matches!(
        extension.as_str(),
        "mp4" | "webm" | "mov" | "avi" | "mkv" | "m4v"
    ) {
        TransferFileKind::Video
    } else {
        TransferFileKind::File
    }
}

pub fn validate_transfer_text(content: &str) -> Result<&str, FileTransferError> {
    if content.is_empty() {
        return Err(validation_error("消息不能为空"));
    }
    if content.len() > MAX_TEXT_BYTES {
        return Err(validation_error(format!(
            "消息不能超过 {} KiB",
            MAX_TEXT_BYTES / 1024
        )));
    }
    Ok(content)
}

pub fn validate_upload_id(upload_id: &str) -> Result<(), FileTransferError> {
    if upload_id.is_empty()
        || upload_id.len() > 80
        || !upload_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(validation_error("上传标识格式无效"));
    }
    Ok(())
}

pub fn sanitize_file_name(input: &str) -> String {
    let base = input
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("未命名文件");
    let mut sanitized = String::with_capacity(base.len().min(180));
    for character in base.chars() {
        if sanitized.chars().count() >= 120 {
            break;
        }
        if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        {
            sanitized.push('_');
        } else {
            sanitized.push(character);
        }
    }
    let trimmed = sanitized.trim().trim_end_matches([' ', '.']).to_owned();
    let mut result = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "未命名文件".to_owned()
    } else {
        trimmed
    };
    let stem = Path::new(&result)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_uppercase();
    if is_windows_reserved_name(&stem) {
        result.insert(0, '_');
    }
    result
}

fn is_windows_reserved_name(stem: &str) -> bool {
    matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

pub fn allocate_receive_path(
    receive_directory: &Path,
    requested_file_name: &str,
) -> Result<PathBuf, FileTransferError> {
    fs::create_dir_all(receive_directory).map_err(|error| {
        FileTransferError::new(
            FileTransferErrorKind::Storage,
            format!("无法创建接收目录 {}：{error}", receive_directory.display()),
        )
    })?;
    let safe_name = sanitize_file_name(requested_file_name);
    let safe_path = Path::new(&safe_name);
    let stem = safe_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("未命名文件");
    let extension = safe_path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());
    for index in 0..10_000_u32 {
        let name = if index == 0 {
            safe_name.clone()
        } else if let Some(extension) = extension {
            format!("{stem} ({index}).{extension}")
        } else {
            format!("{stem} ({index})")
        };
        let candidate = receive_directory.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(validation_error("接收目录中同名文件过多"))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ChunkMetadata {
    pub upload_id: String,
    pub chunk_index: usize,
    pub total_chunks: usize,
    pub file_name: String,
    pub total_size: u64,
}

pub fn validate_chunk(
    metadata: &ChunkMetadata,
    expected_index: usize,
    chunk_size: usize,
) -> Result<(), FileTransferError> {
    validate_upload_id(&metadata.upload_id)?;
    if metadata.total_chunks == 0 || metadata.total_chunks > MAX_CHUNKS {
        return Err(validation_error("分片总数无效"));
    }
    if metadata.chunk_index >= metadata.total_chunks || metadata.chunk_index != expected_index {
        return Err(validation_error(format!(
            "分片顺序无效：应为 {expected_index}，实际为 {}",
            metadata.chunk_index
        )));
    }
    if chunk_size == 0 || chunk_size > MAX_CHUNK_BYTES {
        return Err(validation_error("分片大小无效"));
    }
    if metadata.total_size == 0 || metadata.total_size > MAX_FILE_BYTES {
        return Err(validation_error("文件大小超出允许范围"));
    }
    if metadata.file_name.trim().is_empty() || metadata.file_name.len() > 1024 {
        return Err(validation_error("文件名无效"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tiez-file-transfer-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn message(content: &str) -> TransferMessage {
        TransferMessage {
            id: 0,
            direction: "in".to_owned(),
            msg_type: "text".to_owned(),
            content: content.to_owned(),
            timestamp: 1,
            sender_id: "phone".to_owned(),
            sender_name: "手机".to_owned(),
            file_path: None,
        }
    }

    #[test]
    fn memory_preferences_preserve_legacy_defaults_and_validate_updates() {
        let mut preferences = FileTransferPreferences::in_memory("C:/TieZ/接收");
        let initial = preferences.snapshot().expect("snapshot");
        assert_eq!(initial.port, DEFAULT_PORT);
        assert!(!initial.enabled);
        assert_eq!(initial.receive_directory, "C:/TieZ/接收");

        let updated = preferences
            .update(FileTransferPreferencesUpdate {
                enabled: Some(true),
                port: Some(19_876),
                auto_close: Some(true),
                ..FileTransferPreferencesUpdate::default()
            })
            .expect("update");
        assert!(updated.enabled);
        assert!(updated.auto_close);
        assert_eq!(updated.port, 19_876);
        assert!(preferences
            .update(FileTransferPreferencesUpdate {
                port: Some(0),
                ..FileTransferPreferencesUpdate::default()
            })
            .is_err());
    }

    #[test]
    fn sqlite_preferences_are_compatible_and_read_only_rejects_writes() {
        let root = temporary_directory("sqlite");
        fs::create_dir_all(&root).expect("root");
        let database = root.join("clipboard.db");
        let connection = Connection::open(&database).expect("database");
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);\n                 INSERT INTO settings VALUES ('file_server_enabled', 'true');\n                 INSERT INTO settings VALUES ('file_server_port', '23456');\n                 INSERT INTO settings VALUES ('file_transfer_auto_copy', 'true');",
            )
            .expect("settings");
        drop(connection);

        let mut preferences =
            FileTransferPreferences::open_sqlite(&database, root.join("received"), true)
                .expect("preferences");
        let snapshot = preferences.snapshot().expect("snapshot");
        assert!(snapshot.enabled);
        assert!(snapshot.auto_copy);
        assert_eq!(snapshot.port, 23_456);
        assert_eq!(
            preferences
                .update(FileTransferPreferencesUpdate {
                    enabled: Some(false),
                    ..FileTransferPreferencesUpdate::default()
                })
                .expect_err("read-only")
                .kind(),
            FileTransferErrorKind::ReadOnly
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn file_names_cannot_escape_or_use_windows_device_names() {
        assert_eq!(sanitize_file_name("../../秘密.txt"), "秘密.txt");
        assert_eq!(sanitize_file_name("..\\..\\CON.txt"), "_CON.txt");
        assert_eq!(
            sanitize_file_name("folder/hello<world>.txt"),
            "hello_world_.txt"
        );
        assert_eq!(sanitize_file_name("..."), "未命名文件");
    }

    #[test]
    fn receive_paths_are_confined_and_unique() {
        let root = temporary_directory("paths");
        let first = allocate_receive_path(&root, "../../图片.png").expect("first");
        assert_eq!(first.parent(), Some(root.as_path()));
        fs::write(&first, b"first").expect("write");
        let second = allocate_receive_path(&root, "../../图片.png").expect("second");
        assert_eq!(second.parent(), Some(root.as_path()));
        assert_ne!(first, second);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn chunks_must_be_bounded_and_strictly_ordered() {
        let metadata = ChunkMetadata {
            upload_id: "upload-123".to_owned(),
            chunk_index: 1,
            total_chunks: 3,
            file_name: "video.mp4".to_owned(),
            total_size: 100,
        };
        assert!(validate_chunk(&metadata, 1, 50).is_ok());
        assert!(validate_chunk(&metadata, 0, 50).is_err());
        assert!(validate_upload_id("../escape").is_err());
    }

    #[test]
    fn message_store_caps_history_and_keeps_monotonic_ids() {
        let mut store = TransferMessageStore::new(2);
        assert_eq!(store.push(message("一")).id, 1);
        assert_eq!(store.push(message("二")).id, 2);
        assert_eq!(store.push(message("三")).id, 3);
        let messages = store.since(0);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, 2);
        assert_eq!(store.since(2)[0].content, "三");
    }

    #[test]
    fn classification_matches_mobile_message_contract() {
        assert_eq!(classify_file_name("photo.WEBP").message_type(), "image");
        assert_eq!(classify_file_name("clip.mp4").message_type(), "video");
        assert_eq!(classify_file_name("archive.zip").message_type(), "file");
    }
}
