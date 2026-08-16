//! Shared, frontend-independent TieZ backup and restore workflow.
//!
//! Both the Tauri fallback and the native WinUI runtime use this module so a
//! backup made by either frontend has the same manifest, validation, rollback,
//! and path-rewrite behavior.

use crate::encryption::{decrypt_value, encrypt_value, ENCRYPT_PREFIX};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const BACKUP_FORMAT_VERSION: u32 = 1;
pub const DATABASE_NAME: &str = "clipboard.db";
pub const PENDING_BACKUP_NAME: &str = ".tiez-restore-pending.tiez-backup";

const MANIFEST_NAME: &str = "manifest.json";
const MAX_RESTORE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 10 * 1024 * 1024;
const MAX_BACKUP_FILES: usize = 100_000;
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupErrorKind {
    Database,
    Io,
    Internal,
    Validation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupError {
    kind: BackupErrorKind,
    message: String,
}

impl BackupError {
    fn new(kind: BackupErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> BackupErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackupError {}

impl From<rusqlite::Error> for BackupError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(BackupErrorKind::Database, error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupFileEntry {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    format_version: u32,
    app_version: String,
    created_at: i64,
    source_data_path: String,
    entry_count: i64,
    files: Vec<BackupFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub format_version: u32,
    pub app_version: String,
    pub created_at: i64,
    pub entry_count: i64,
    pub file_count: usize,
    pub total_bytes: u64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    pub applied: bool,
    pub quarantined: bool,
    pub rollback_path: Option<String>,
    pub quarantine_path: Option<String>,
    pub message: String,
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn create(label: &str) -> Result<Self, BackupError> {
        let path = std::env::temp_dir().join(format!("tiez-{label}-{}", unique_suffix()));
        fs::create_dir_all(&path).map_err(|error| io_error("无法创建备份暂存目录", error))?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn unique_suffix() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{timestamp}-{sequence}", std::process::id())
}

fn io_error(context: &str, error: impl fmt::Display) -> BackupError {
    BackupError::new(BackupErrorKind::Io, format!("{context}: {error}"))
}

fn validation_error(message: impl Into<String>) -> BackupError {
    BackupError::new(BackupErrorKind::Validation, message)
}

fn internal_error(message: impl Into<String>) -> BackupError {
    BackupError::new(BackupErrorKind::Internal, message)
}

fn normalize_archive_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hash_reader(reader: &mut impl Read, context: &str) -> Result<(u64, String), BackupError> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| io_error(context, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| validation_error("备份文件体积溢出"))?;
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn hash_file(path: &Path) -> Result<(u64, String), BackupError> {
    let mut file = File::open(path).map_err(|error| io_error("无法读取备份文件", error))?;
    hash_reader(&mut file, "无法计算文件校验值")
}

fn collect_directory_files(
    root: &Path,
    relative: &Path,
    output: &mut Vec<(PathBuf, String)>,
) -> Result<(), BackupError> {
    let directory = root.join(relative);
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&directory).map_err(|error| io_error("无法读取数据目录", error))?
    {
        let entry = entry.map_err(|error| io_error("无法读取数据目录项", error))?;
        let relative_path = relative.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| io_error("无法读取数据文件类型", error))?;
        if file_type.is_dir() {
            collect_directory_files(root, &relative_path, output)?;
        } else if file_type.is_file() {
            output.push((entry.path(), normalize_archive_path(&relative_path)));
        }
    }
    Ok(())
}

fn info_from_manifest(manifest: &BackupManifest, path: &Path) -> BackupInfo {
    BackupInfo {
        format_version: manifest.format_version,
        app_version: manifest.app_version.clone(),
        created_at: manifest.created_at,
        entry_count: manifest.entry_count,
        file_count: manifest.files.len(),
        total_bytes: manifest.files.iter().map(|file| file.size).sum(),
        path: path.to_string_lossy().to_string(),
    }
}

fn validate_manifest_shape(manifest: &BackupManifest) -> Result<(), BackupError> {
    if manifest.format_version != BACKUP_FORMAT_VERSION {
        return Err(validation_error(format!(
            "不支持的备份格式版本: {}",
            manifest.format_version
        )));
    }
    if manifest.app_version.trim().is_empty() || manifest.source_data_path.trim().is_empty() {
        return Err(validation_error("备份清单缺少版本或原始数据路径"));
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_BACKUP_FILES {
        return Err(validation_error("备份文件数量超过安全限制"));
    }
    if !manifest.files.iter().any(|file| file.path == DATABASE_NAME) {
        return Err(validation_error("备份缺少剪贴板数据库"));
    }

    let mut paths = HashSet::with_capacity(manifest.files.len());
    let mut total_bytes = 0_u64;
    for entry in &manifest.files {
        let safe_path = Path::new(&entry.path);
        if entry.path.is_empty()
            || entry.path == MANIFEST_NAME
            || safe_path.is_absolute()
            || safe_path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(validation_error("备份包含不安全路径"));
        }
        if !paths.insert(entry.path.clone()) {
            return Err(validation_error(format!(
                "备份清单包含重复文件: {}",
                entry.path
            )));
        }
        if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|value| value.is_ascii_hexdigit())
        {
            return Err(validation_error(format!("备份校验值无效: {}", entry.path)));
        }
        total_bytes = total_bytes
            .checked_add(entry.size)
            .ok_or_else(|| validation_error("备份解压体积溢出"))?;
    }
    if total_bytes > MAX_RESTORE_BYTES {
        return Err(validation_error("备份解压后体积超过安全限制"));
    }
    Ok(())
}

fn read_and_validate_manifest(
    path: &Path,
    verify_hashes: bool,
) -> Result<BackupManifest, BackupError> {
    let file = File::open(path).map_err(|error| io_error("无法打开备份", error))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| validation_error(format!("备份文件格式无效: {error}")))?;
    let manifest: BackupManifest = {
        let mut entry = archive
            .by_name(MANIFEST_NAME)
            .map_err(|_| validation_error("备份缺少 manifest.json"))?;
        if entry.size() > MAX_MANIFEST_BYTES {
            return Err(validation_error("备份清单超过安全限制"));
        }
        let mut json = String::new();
        entry
            .read_to_string(&mut json)
            .map_err(|error| io_error("无法读取备份清单", error))?;
        serde_json::from_str(&json)
            .map_err(|error| validation_error(format!("备份清单无效: {error}")))?
    };
    validate_manifest_shape(&manifest)?;

    let expected_names = manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .chain(std::iter::once(MANIFEST_NAME))
        .collect::<HashSet<_>>();
    let mut archive_names = HashSet::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| validation_error(format!("无法读取备份目录: {error}")))?;
        let name = entry.name().to_owned();
        if entry.is_dir() || !expected_names.contains(name.as_str()) {
            return Err(validation_error(format!("备份包含未声明文件: {name}")));
        }
        if !archive_names.insert(name.clone()) {
            return Err(validation_error(format!("备份包含重复文件: {name}")));
        }
    }
    if archive_names.len() != expected_names.len() {
        return Err(validation_error("备份目录与清单不一致"));
    }

    for expected in &manifest.files {
        let mut entry = archive
            .by_name(&expected.path)
            .map_err(|_| validation_error(format!("备份缺少文件: {}", expected.path)))?;
        if entry.is_dir() || entry.size() != expected.size {
            return Err(validation_error(format!(
                "备份文件大小不匹配: {}",
                expected.path
            )));
        }
        if verify_hashes {
            let (size, actual) = hash_reader(&mut entry, "无法校验备份内容")?;
            if size != expected.size || !actual.eq_ignore_ascii_case(&expected.sha256) {
                return Err(validation_error(format!(
                    "备份文件校验失败: {}",
                    expected.path
                )));
            }
        }
    }
    Ok(manifest)
}

fn validate_database(path: &Path) -> Result<(), BackupError> {
    let connection = Connection::open(path)?;
    let quick_check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        return Err(validation_error(format!(
            "备份数据库完整性检查失败: {quick_check}"
        )));
    }
    let has_history: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clipboard_history')",
        [],
        |row| row.get(0),
    )?;
    if !has_history {
        return Err(validation_error("备份数据库缺少历史记录表"));
    }
    Ok(())
}

fn extract_backup(
    path: &Path,
    destination: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    fs::create_dir_all(destination).map_err(|error| io_error("无法创建恢复暂存目录", error))?;
    let file = File::open(path).map_err(|error| io_error("无法打开待恢复备份", error))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| validation_error(format!("备份文件格式无效: {error}")))?;
    for expected in &manifest.files {
        let mut entry = archive
            .by_name(&expected.path)
            .map_err(|_| validation_error(format!("备份缺少文件: {}", expected.path)))?;
        let output = destination.join(&expected.path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error("无法创建恢复目录", error))?;
        }
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| io_error("无法写入恢复文件", error))?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|error| io_error("无法解压恢复文件", error))?;
            if read == 0 {
                break;
            }
            writer
                .write_all(&buffer[..read])
                .map_err(|error| io_error("无法写入恢复文件", error))?;
            hasher.update(&buffer[..read]);
            size = size
                .checked_add(read as u64)
                .ok_or_else(|| validation_error("恢复文件体积溢出"))?;
        }
        let actual = format!("{:x}", hasher.finalize());
        if size != expected.size || !actual.eq_ignore_ascii_case(&expected.sha256) {
            return Err(validation_error(format!(
                "备份文件校验失败: {}",
                expected.path
            )));
        }
    }
    validate_database(&destination.join(DATABASE_NAME))
}

fn move_if_exists(source: &Path, destination: &Path) -> Result<(), BackupError> {
    if !source.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error("无法创建回滚目录", error))?;
    }
    fs::rename(source, destination).map_err(|error| io_error("无法移动数据文件", error))
}

fn replace_file(temporary: &Path, destination: &Path, context: &str) -> Result<(), BackupError> {
    if !destination.exists() {
        return fs::rename(temporary, destination).map_err(|error| io_error(context, error));
    }
    if destination.is_dir() {
        return Err(validation_error("目标位置是目录，无法覆盖"));
    }
    let previous = destination.with_extension(format!("replace-{}", unique_suffix()));
    fs::rename(destination, &previous).map_err(|error| io_error("无法暂存旧文件", error))?;
    if let Err(error) = fs::rename(temporary, destination) {
        let _ = fs::rename(&previous, destination);
        return Err(io_error(context, error));
    }
    let _ = fs::remove_file(previous);
    Ok(())
}

fn ensure_destination_outside_data_dir(
    data_dir: &Path,
    destination: &Path,
) -> Result<(), BackupError> {
    if destination.as_os_str().is_empty() {
        return Err(validation_error("未选择备份保存位置"));
    }
    let data_dir = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let destination = if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| io_error("无法解析备份保存位置", error))?
            .join(destination)
    };
    let normalized_destination = destination
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| destination.file_name().map(|name| parent.join(name)))
        .unwrap_or(destination);
    if normalized_destination.starts_with(&data_dir) {
        return Err(validation_error("备份文件不能保存在 TieZ 管理的数据目录内"));
    }
    Ok(())
}

pub fn create_backup(
    database_path: &Path,
    data_dir: &Path,
    destination: &Path,
    app_version: &str,
) -> Result<BackupInfo, BackupError> {
    let connection = Connection::open(database_path)?;
    create_backup_from_connection(&connection, data_dir, destination, app_version)
}

pub fn create_backup_from_connection(
    connection: &Connection,
    data_dir: &Path,
    destination: &Path,
    app_version: &str,
) -> Result<BackupInfo, BackupError> {
    ensure_destination_outside_data_dir(data_dir, destination)?;
    if app_version.trim().is_empty() {
        return Err(validation_error("应用版本不能为空"));
    }
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| io_error("无法创建备份目录", error))?;
    }

    let temporary_root = TemporaryDirectory::create("backup")?;
    let snapshot = temporary_root.path.join(DATABASE_NAME);
    connection.execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()])?;
    let entry_count =
        connection.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
            row.get(0)
        })?;

    let mut sources = vec![(snapshot, DATABASE_NAME.to_owned())];
    collect_directory_files(data_dir, Path::new("attachments"), &mut sources)?;
    collect_directory_files(data_dir, Path::new("emoji_favorites"), &mut sources)?;
    sources.sort_by(|left, right| left.1.cmp(&right.1));
    if sources.len() > MAX_BACKUP_FILES {
        return Err(validation_error("待备份文件数量超过安全限制"));
    }

    let mut files = Vec::with_capacity(sources.len());
    for (source, archive_path) in &sources {
        let (size, sha256) = hash_file(source)?;
        files.push(BackupFileEntry {
            path: archive_path.clone(),
            size,
            sha256,
        });
    }
    let manifest = BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        app_version: app_version.to_owned(),
        created_at: now_ms(),
        source_data_path: data_dir.to_string_lossy().to_string(),
        entry_count,
        files,
    };
    validate_manifest_shape(&manifest)?;

    let temporary_archive =
        destination.with_extension(format!("tiez-backup.tmp-{}", unique_suffix()));
    let archive_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_archive)
        .map_err(|error| io_error("无法创建备份文件", error))?;
    let write_result = (|| -> Result<(), BackupError> {
        let mut zip = ZipWriter::new(archive_file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (source, archive_path) in &sources {
            zip.start_file(archive_path, options)
                .map_err(|error| io_error("无法写入备份条目", error))?;
            let mut reader =
                File::open(source).map_err(|error| io_error("无法读取待备份文件", error))?;
            std::io::copy(&mut reader, &mut zip)
                .map_err(|error| io_error("无法写入备份内容", error))?;
        }
        zip.start_file(MANIFEST_NAME, options)
            .map_err(|error| io_error("无法写入备份清单", error))?;
        let manifest_json = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| internal_error(format!("无法生成备份清单: {error}")))?;
        zip.write_all(&manifest_json)
            .map_err(|error| io_error("无法写入备份清单", error))?;
        zip.finish()
            .map_err(|error| io_error("无法完成备份", error))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_archive);
        return Err(error);
    }
    if let Err(error) = replace_file(&temporary_archive, destination, "无法保存备份") {
        let _ = fs::remove_file(&temporary_archive);
        return Err(error);
    }
    Ok(info_from_manifest(&manifest, destination))
}

pub fn inspect_backup(path: &Path) -> Result<BackupInfo, BackupError> {
    if path.as_os_str().is_empty() {
        return Err(validation_error("未选择备份文件"));
    }
    let manifest = read_and_validate_manifest(path, true)?;
    Ok(info_from_manifest(&manifest, path))
}

pub fn schedule_backup_restore(data_dir: &Path, source: &Path) -> Result<BackupInfo, BackupError> {
    if source.as_os_str().is_empty() {
        return Err(validation_error("未选择备份文件"));
    }
    fs::create_dir_all(data_dir).map_err(|error| io_error("无法创建数据目录", error))?;
    let pending = data_dir.join(PENDING_BACKUP_NAME);
    if source == pending {
        return Err(validation_error("不能直接选择内部待恢复文件"));
    }
    let temporary = data_dir.join(format!("{PENDING_BACKUP_NAME}.tmp-{}", unique_suffix()));
    fs::copy(source, &temporary).map_err(|error| io_error("无法暂存待恢复备份", error))?;
    let result = (|| -> Result<BackupInfo, BackupError> {
        let manifest = read_and_validate_manifest(&temporary, true)?;
        replace_file(&temporary, &pending, "无法安排恢复")?;
        Ok(info_from_manifest(&manifest, source))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn quarantine_pending_backup(
    pending: &Path,
    data_dir: &Path,
    reason: &str,
) -> Result<RestoreOutcome, BackupError> {
    let failed = data_dir.join(format!(
        "restore-failed-{}-{}.tiez-backup",
        now_ms(),
        unique_suffix()
    ));
    fs::rename(pending, &failed).map_err(|error| io_error("无法隔离损坏的待恢复备份", error))?;
    Ok(RestoreOutcome {
        applied: false,
        quarantined: true,
        rollback_path: None,
        quarantine_path: Some(failed.to_string_lossy().to_string()),
        message: format!("待恢复备份无效，已隔离：{reason}"),
    })
}

pub fn apply_pending_restore(data_dir: &Path) -> Result<RestoreOutcome, BackupError> {
    let pending = data_dir.join(PENDING_BACKUP_NAME);
    if !pending.is_file() {
        return Ok(RestoreOutcome {
            applied: false,
            quarantined: false,
            rollback_path: None,
            quarantine_path: None,
            message: "没有待恢复备份".to_owned(),
        });
    }

    let manifest = match read_and_validate_manifest(&pending, true) {
        Ok(manifest) => manifest,
        Err(error) => return quarantine_pending_backup(&pending, data_dir, &error.to_string()),
    };
    let staging = data_dir.join(format!(".tiez-restore-staging-{}", unique_suffix()));
    if let Err(error) = extract_backup(&pending, &staging, &manifest) {
        let _ = fs::remove_dir_all(&staging);
        return quarantine_pending_backup(&pending, data_dir, &error.to_string());
    }

    let staged_database = staging.join(DATABASE_NAME);
    let old_base = PathBuf::from(&manifest.source_data_path);
    if let Err(error) = rewrite_data_paths_in_database(&staged_database, &old_base, data_dir)
        .and_then(|_| validate_database(&staged_database))
    {
        let _ = fs::remove_dir_all(&staging);
        return quarantine_pending_backup(&pending, data_dir, &error.to_string());
    }

    let consumed = data_dir.join(format!(
        ".tiez-restore-consumed-{}.tiez-backup",
        unique_suffix()
    ));
    fs::rename(&pending, &consumed)
        .map_err(|error| io_error("无法标记待恢复备份为已消费", error))?;

    let rollback = data_dir.join(format!("restore-rollback-{}-{}", now_ms(), unique_suffix()));
    if let Err(error) = fs::create_dir_all(&rollback) {
        let _ = fs::rename(&consumed, &pending);
        return Err(io_error("无法创建恢复回滚目录", error));
    }
    let managed_paths = [
        DATABASE_NAME,
        "clipboard.db-wal",
        "clipboard.db-shm",
        "attachments",
        "emoji_favorites",
    ];
    let mut moved_current: Vec<&str> = Vec::new();
    for name in managed_paths {
        if data_dir.join(name).exists() {
            match move_if_exists(&data_dir.join(name), &rollback.join(name)) {
                Ok(()) => moved_current.push(name),
                Err(error) => {
                    for moved_name in moved_current.iter().rev() {
                        let _ = fs::rename(rollback.join(moved_name), data_dir.join(moved_name));
                    }
                    let _ = fs::rename(&consumed, &pending);
                    let _ = fs::remove_dir_all(&staging);
                    return Err(error);
                }
            }
        }
    }

    let install_result = (|| -> Result<(), BackupError> {
        move_if_exists(&staging.join(DATABASE_NAME), &data_dir.join(DATABASE_NAME))?;
        move_if_exists(&staging.join("attachments"), &data_dir.join("attachments"))?;
        move_if_exists(
            &staging.join("emoji_favorites"),
            &data_dir.join("emoji_favorites"),
        )?;
        Ok(())
    })();
    if let Err(error) = install_result {
        for name in [DATABASE_NAME, "attachments", "emoji_favorites"] {
            let target = data_dir.join(name);
            if target.is_dir() {
                let _ = fs::remove_dir_all(&target);
            } else if target.exists() {
                let _ = fs::remove_file(&target);
            }
        }
        for name in moved_current {
            let _ = fs::rename(rollback.join(name), data_dir.join(name));
        }
        let _ = fs::rename(&consumed, &pending);
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&consumed);
    cleanup_old_rollbacks(data_dir, &rollback);
    Ok(RestoreOutcome {
        applied: true,
        quarantined: false,
        rollback_path: Some(rollback.to_string_lossy().to_string()),
        quarantine_path: None,
        message: "备份已恢复，并保留当前数据回滚副本".to_owned(),
    })
}

fn cleanup_old_rollbacks(data_dir: &Path, current_rollback: &Path) {
    if let Ok(entries) = fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("restore-rollback-") || entry.path() == current_rollback {
                continue;
            }
            let expired = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .map(|age| age > Duration::from_secs(7 * 24 * 60 * 60))
                .unwrap_or(false);
            if expired {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn rewrite_data_paths_in_database(
    database_path: &Path,
    old_base: &Path,
    new_base: &Path,
) -> Result<(), BackupError> {
    rewrite_attachment_paths(database_path, old_base, new_base)?;
    rewrite_emoji_favorites(database_path, old_base, new_base)?;
    rewrite_custom_background(database_path, old_base, new_base)
}

fn rewrite_attachment_paths(
    database_path: &Path,
    old_base: &Path,
    new_base: &Path,
) -> Result<(), BackupError> {
    let old_prefix = old_base.join("attachments").to_string_lossy().to_string();
    let new_prefix = new_base.join("attachments").to_string_lossy().to_string();
    if old_prefix == new_prefix {
        return Ok(());
    }
    let old_slash = old_prefix.replace('\\', "/");
    let new_slash = new_prefix.replace('\\', "/");
    let connection = Connection::open(database_path)?;
    let mut statement = connection.prepare(
        "SELECT id, content, html_content FROM clipboard_history
         WHERE content_type IN ('image', 'file', 'video')
            OR is_external = 1
            OR html_content IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (id, stored_content, stored_html) = row?;
        let content = rewrite_encrypted_value(&stored_content, |plain| {
            rewrite_prefix(plain, &old_prefix, &new_prefix, &old_slash, &new_slash)
        });
        let html = stored_html.as_deref().and_then(|stored| {
            rewrite_encrypted_value(stored, |plain| {
                let updated = plain
                    .replace(&old_prefix, &new_prefix)
                    .replace(&old_slash, &new_slash);
                (updated != plain).then_some(updated)
            })
        });
        if content.is_some() || html.is_some() {
            connection.execute(
                "UPDATE clipboard_history SET content = ?1, html_content = ?2 WHERE id = ?3",
                params![
                    content.as_deref().unwrap_or(&stored_content),
                    html.as_deref().or(stored_html.as_deref()),
                    id
                ],
            )?;
        }
    }
    Ok(())
}

fn rewrite_encrypted_value(
    stored: &str,
    rewrite: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    if stored.starts_with(ENCRYPT_PREFIX) {
        let plain = decrypt_value(stored)?;
        let updated = rewrite(&plain)?;
        encrypt_value(&updated)
    } else {
        rewrite(stored)
    }
}

fn rewrite_prefix(
    value: &str,
    old_prefix: &str,
    new_prefix: &str,
    old_slash: &str,
    new_slash: &str,
) -> Option<String> {
    if let Some(suffix) = value.strip_prefix(old_prefix) {
        Some(format!("{new_prefix}{suffix}"))
    } else {
        value
            .strip_prefix(old_slash)
            .map(|suffix| format!("{new_slash}{suffix}"))
    }
}

fn rewrite_emoji_favorites(
    database_path: &Path,
    old_base: &Path,
    new_base: &Path,
) -> Result<(), BackupError> {
    let old_prefix = old_base
        .join("emoji_favorites")
        .to_string_lossy()
        .to_string();
    let new_prefix = new_base
        .join("emoji_favorites")
        .to_string_lossy()
        .to_string();
    if old_prefix == new_prefix {
        return Ok(());
    }
    let old_slash = old_prefix.replace('\\', "/");
    let new_slash = new_prefix.replace('\\', "/");
    let connection = Connection::open(database_path)?;
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = 'app.emoji_favorites'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = value else {
        return Ok(());
    };
    let Ok(paths) = serde_json::from_str::<Vec<String>>(&raw) else {
        return Ok(());
    };
    let updated = paths
        .iter()
        .map(|path| {
            rewrite_prefix(path, &old_prefix, &new_prefix, &old_slash, &new_slash)
                .unwrap_or_else(|| path.clone())
        })
        .collect::<Vec<_>>();
    if updated != paths {
        let serialized = serde_json::to_string(&updated)
            .map_err(|error| internal_error(format!("无法序列化表情收藏路径: {error}")))?;
        connection.execute(
            "UPDATE settings SET value = ?1 WHERE key = 'app.emoji_favorites'",
            [serialized],
        )?;
    }
    Ok(())
}

fn rewrite_custom_background(
    database_path: &Path,
    old_base: &Path,
    new_base: &Path,
) -> Result<(), BackupError> {
    let connection = Connection::open(database_path)?;
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = 'app.custom_background'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw_path) = value else {
        return Ok(());
    };
    let old_path = PathBuf::from(raw_path.trim());
    if old_path.as_os_str().is_empty() || !old_path.starts_with(old_base) {
        return Ok(());
    }
    let Ok(relative) = old_path.strip_prefix(old_base) else {
        return Ok(());
    };
    let new_path = new_base.join(relative);
    if old_path != new_path && old_path.exists() {
        if let Some(parent) = new_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("无法创建自定义背景目录", error))?;
        }
        if !new_path.exists() {
            if fs::rename(&old_path, &new_path).is_err() {
                fs::copy(&old_path, &new_path)
                    .map_err(|error| io_error("无法迁移自定义背景", error))?;
                let _ = fs::remove_file(&old_path);
            }
        }
    }
    let new_value = new_path.to_string_lossy().to_string();
    if new_value != raw_path {
        connection.execute(
            "UPDATE settings SET value = ?1 WHERE key = 'app.custom_background'",
            [new_value],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database_migrations::run_migrations;

    fn test_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tiez-{label}-{}", unique_suffix()));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn create_database(path: &Path, content: &str) -> Connection {
        let connection = Connection::open(path).expect("open test database");
        run_migrations(&connection).expect("migrate test database");
        connection
            .execute(
                "INSERT INTO clipboard_history
                 (content_type, content, source_app, timestamp, preview, content_hash)
                 VALUES ('text', ?1, 'test', 1, ?1, 1)",
                [content],
            )
            .expect("insert test entry");
        connection
    }

    fn write_test_backup(
        archive_path: &Path,
        source_base: &Path,
        content: &str,
        valid_hash: bool,
    ) -> BackupManifest {
        let build_directory = test_directory("backup-build");
        let database = build_directory.join(DATABASE_NAME);
        let connection = Connection::open(&database).expect("open test backup database");
        run_migrations(&connection).expect("migrate test backup database");
        let attachment_path = source_base.join("attachments").join("image.png");
        connection
            .execute(
                "INSERT INTO clipboard_history
                 (content_type, content, source_app, timestamp, preview, content_hash)
                 VALUES ('image', ?1, 'test', 1, 'image', 7)",
                [attachment_path.to_string_lossy().as_ref()],
            )
            .expect("insert test image entry");
        drop(connection);

        let attachment = build_directory.join("image.png");
        fs::write(&attachment, content).expect("write test attachment");
        let sources = vec![
            (database, DATABASE_NAME.to_owned()),
            (attachment, "attachments/image.png".to_owned()),
        ];
        let files = sources
            .iter()
            .map(|(path, name)| {
                let (size, mut sha256) = hash_file(path).expect("hash test backup file");
                if !valid_hash && name == DATABASE_NAME {
                    sha256 = "0".repeat(64);
                }
                BackupFileEntry {
                    path: name.clone(),
                    size,
                    sha256,
                }
            })
            .collect();
        let manifest = BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            app_version: "test".to_owned(),
            created_at: 1,
            source_data_path: source_base.to_string_lossy().to_string(),
            entry_count: 1,
            files,
        };

        let writer = File::create(archive_path).expect("create test backup archive");
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (source, name) in sources {
            zip.start_file(name, options)
                .expect("start test backup file");
            let mut source = File::open(source).expect("open test backup source");
            std::io::copy(&mut source, &mut zip).expect("write test backup source");
        }
        zip.start_file(MANIFEST_NAME, options)
            .expect("start test manifest");
        zip.write_all(&serde_json::to_vec(&manifest).expect("serialize test backup manifest"))
            .expect("write test manifest");
        zip.finish().expect("finish test backup");
        let _ = fs::remove_dir_all(build_directory);
        manifest
    }

    #[test]
    fn creates_inspects_and_replaces_a_complete_backup() {
        let root = test_directory("backup-create");
        let data_dir = root.join("data");
        fs::create_dir_all(data_dir.join("attachments")).expect("create attachments");
        fs::create_dir_all(data_dir.join("emoji_favorites")).expect("create emoji favorites");
        fs::write(data_dir.join("attachments/image.png"), "attachment").expect("write attachment");
        fs::write(data_dir.join("emoji_favorites/favorite.png"), "emoji").expect("write emoji");
        let database = data_dir.join(DATABASE_NAME);
        let connection = create_database(&database, "first");
        let destination = root.join("daily.tiez-backup");

        let created =
            create_backup_from_connection(&connection, &data_dir, &destination, "0.3.8-test")
                .expect("create backup");
        assert_eq!(created.entry_count, 1);
        assert_eq!(created.file_count, 3);
        assert_eq!(
            inspect_backup(&destination).expect("inspect backup"),
            created
        );

        connection
            .execute("DELETE FROM clipboard_history", [])
            .expect("clear history");
        let replaced =
            create_backup_from_connection(&connection, &data_dir, &destination, "0.3.8-test")
                .expect("replace backup");
        assert_eq!(replaced.entry_count, 0);
        assert!(destination.is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_backup_inside_the_managed_data_directory() {
        let root = test_directory("backup-destination");
        let data_dir = root.join("data");
        fs::create_dir_all(&data_dir).expect("create data directory");
        let connection = create_database(&data_dir.join(DATABASE_NAME), "entry");
        let error = create_backup_from_connection(
            &connection,
            &data_dir,
            &data_dir.join("unsafe.tiez-backup"),
            "test",
        )
        .expect_err("reject managed destination");
        assert_eq!(error.kind(), BackupErrorKind::Validation);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_backup_with_wrong_checksum() {
        let root = test_directory("backup-checksum");
        let archive = root.join("invalid.tiez-backup");
        write_test_backup(&archive, &root.join("old-data"), "attachment", false);
        let error = inspect_backup(&archive).expect_err("reject bad checksum");
        assert!(error.to_string().contains("校验失败"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unsafe_and_duplicate_manifest_paths() {
        let database = BackupFileEntry {
            path: DATABASE_NAME.to_owned(),
            size: 0,
            sha256: "0".repeat(64),
        };
        let mut manifest = BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            app_version: "test".to_owned(),
            created_at: 1,
            source_data_path: "C:/TieZ".to_owned(),
            entry_count: 0,
            files: vec![
                database.clone(),
                BackupFileEntry {
                    path: "../escape.txt".to_owned(),
                    size: 0,
                    sha256: "0".repeat(64),
                },
            ],
        };
        assert!(validate_manifest_shape(&manifest)
            .unwrap_err()
            .to_string()
            .contains("不安全路径"));

        manifest.files = vec![database.clone(), database];
        assert!(validate_manifest_shape(&manifest)
            .unwrap_err()
            .to_string()
            .contains("重复文件"));
    }

    #[test]
    fn schedules_and_applies_restore_with_rollback_and_path_rewrite() {
        let root = test_directory("backup-restore");
        let data_dir = root.join("current-data");
        fs::create_dir_all(&data_dir).expect("create current data directory");
        let current_database = data_dir.join(DATABASE_NAME);
        drop(create_database(&current_database, "old entry"));

        let source_base = root.join("source-data");
        let source = root.join("source.tiez-backup");
        write_test_backup(&source, &source_base, "restored attachment", true);
        schedule_backup_restore(&data_dir, &source).expect("schedule restore");
        assert!(data_dir.join(PENDING_BACKUP_NAME).is_file());

        let outcome = apply_pending_restore(&data_dir).expect("apply pending restore");
        assert!(outcome.applied);
        assert!(!data_dir.join(PENDING_BACKUP_NAME).exists());
        let restored = Connection::open(data_dir.join(DATABASE_NAME)).expect("open restored db");
        let restored_path: String = restored
            .query_row(
                "SELECT content FROM clipboard_history WHERE content_type = 'image'",
                [],
                |row| row.get(0),
            )
            .expect("read restored image entry");
        assert_eq!(
            PathBuf::from(restored_path),
            data_dir.join("attachments/image.png")
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("attachments/image.png"))
                .expect("read installed attachment"),
            "restored attachment"
        );
        assert!(Path::new(outcome.rollback_path.as_deref().expect("rollback path")).is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quarantines_an_invalid_pending_backup() {
        let root = test_directory("backup-quarantine");
        let data_dir = root.join("data");
        fs::create_dir_all(&data_dir).expect("create data directory");
        fs::write(data_dir.join(PENDING_BACKUP_NAME), "not a zip").expect("write invalid pending");
        let outcome = apply_pending_restore(&data_dir).expect("quarantine pending backup");
        assert!(outcome.quarantined);
        assert!(!data_dir.join(PENDING_BACKUP_NAME).exists());
        assert!(Path::new(outcome.quarantine_path.as_deref().expect("quarantine path")).is_file());
        let _ = fs::remove_dir_all(root);
    }
}
