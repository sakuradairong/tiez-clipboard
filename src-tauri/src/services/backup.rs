use crate::app_state::AppDataDir;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::State;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const BACKUP_FORMAT_VERSION: u32 = 1;
const MANIFEST_NAME: &str = "manifest.json";
const DATABASE_NAME: &str = "clipboard.db";
const PENDING_BACKUP_NAME: &str = ".tiez-restore-pending.tiez-backup";
const MAX_RESTORE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 10 * 1024 * 1024;

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

#[derive(Debug, Clone, Serialize)]
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn io_error(context: &str, err: impl std::fmt::Display) -> AppError {
    AppError::IO(format!("{context}: {err}"))
}

fn normalize_archive_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hash_file(path: &Path) -> AppResult<(u64, String)> {
    let mut file = File::open(path).map_err(|err| io_error("无法读取备份文件", err))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| io_error("无法计算文件校验值", err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn collect_directory_files(
    root: &Path,
    relative: &Path,
    output: &mut Vec<(PathBuf, String)>,
) -> AppResult<()> {
    let directory = root.join(relative);
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&directory).map_err(|err| io_error("无法读取数据目录", err))?
    {
        let entry = entry.map_err(|err| io_error("无法读取数据目录项", err))?;
        let relative_path = relative.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| io_error("无法读取数据文件类型", err))?;
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

fn read_and_validate_manifest(path: &Path, verify_hashes: bool) -> AppResult<BackupManifest> {
    let file = File::open(path).map_err(|err| io_error("无法打开备份", err))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| AppError::Validation(format!("备份文件格式无效: {err}")))?;
    let manifest: BackupManifest = {
        let mut entry = archive
            .by_name(MANIFEST_NAME)
            .map_err(|_| AppError::Validation("备份缺少 manifest.json".to_string()))?;
        if entry.size() > MAX_MANIFEST_BYTES {
            return Err(AppError::Validation("备份清单超过安全限制".to_string()));
        }
        let mut json = String::new();
        entry
            .read_to_string(&mut json)
            .map_err(|err| io_error("无法读取备份清单", err))?;
        serde_json::from_str(&json)
            .map_err(|err| AppError::Validation(format!("备份清单无效: {err}")))?
    };

    if manifest.format_version != BACKUP_FORMAT_VERSION {
        return Err(AppError::Validation(format!(
            "不支持的备份格式版本: {}",
            manifest.format_version
        )));
    }
    if !manifest.files.iter().any(|file| file.path == DATABASE_NAME) {
        return Err(AppError::Validation("备份缺少剪贴板数据库".to_string()));
    }
    let total_bytes: u64 = manifest.files.iter().map(|entry| entry.size).sum();
    if total_bytes > MAX_RESTORE_BYTES {
        return Err(AppError::Validation(
            "备份解压后体积超过安全限制".to_string(),
        ));
    }

    for expected in &manifest.files {
        let safe_path = Path::new(&expected.path);
        if safe_path.is_absolute()
            || safe_path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(AppError::Validation("备份包含不安全路径".to_string()));
        }
        let mut entry = archive
            .by_name(&expected.path)
            .map_err(|_| AppError::Validation(format!("备份缺少文件: {}", expected.path)))?;
        if entry.is_dir() || entry.size() != expected.size {
            return Err(AppError::Validation(format!(
                "备份文件大小不匹配: {}",
                expected.path
            )));
        }
        if verify_hashes {
            let mut hasher = Sha256::new();
            std::io::copy(&mut entry, &mut hasher)
                .map_err(|err| io_error("无法校验备份内容", err))?;
            let actual = format!("{:x}", hasher.finalize());
            if actual != expected.sha256 {
                return Err(AppError::Validation(format!(
                    "备份文件校验失败: {}",
                    expected.path
                )));
            }
        }
    }
    Ok(manifest)
}

fn validate_database(path: &Path) -> AppResult<()> {
    let conn = Connection::open(path)?;
    let quick_check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        return Err(AppError::Validation(format!(
            "备份数据库完整性检查失败: {quick_check}"
        )));
    }
    let has_history: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clipboard_history')",
        [],
        |row| row.get(0),
    )?;
    if !has_history {
        return Err(AppError::Validation("备份数据库缺少历史记录表".to_string()));
    }
    Ok(())
}

fn extract_backup(path: &Path, destination: &Path, manifest: &BackupManifest) -> AppResult<()> {
    fs::create_dir_all(destination).map_err(|err| io_error("无法创建恢复暂存目录", err))?;
    let file = File::open(path).map_err(|err| io_error("无法打开待恢复备份", err))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| AppError::Validation(format!("备份文件格式无效: {err}")))?;
    for expected in &manifest.files {
        let mut entry = archive
            .by_name(&expected.path)
            .map_err(|_| AppError::Validation(format!("备份缺少文件: {}", expected.path)))?;
        let output = destination.join(&expected.path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|err| io_error("无法创建恢复目录", err))?;
        }
        let mut writer = File::create(&output).map_err(|err| io_error("无法写入恢复文件", err))?;
        std::io::copy(&mut entry, &mut writer).map_err(|err| io_error("无法解压恢复文件", err))?;
    }
    validate_database(&destination.join(DATABASE_NAME))
}

fn move_if_exists(source: &Path, destination: &Path) -> AppResult<()> {
    if !source.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error("无法创建回滚目录", err))?;
    }
    fs::rename(source, destination).map_err(|err| io_error("无法移动数据文件", err))
}

#[tauri::command]
pub fn create_backup(
    state: State<'_, DbState>,
    app_data: State<'_, AppDataDir>,
    destination: String,
) -> AppResult<BackupInfo> {
    let destination = PathBuf::from(destination.trim());
    if destination.as_os_str().is_empty() {
        return Err(AppError::Validation("未选择备份保存位置".to_string()));
    }
    let data_dir = app_data
        .0
        .lock()
        .map_err(|err| AppError::Internal(err.to_string()))?
        .clone();
    for protected in [
        data_dir.join(DATABASE_NAME),
        data_dir.join("attachments"),
        data_dir.join("emoji_favorites"),
    ] {
        if destination == protected || destination.starts_with(&protected) {
            return Err(AppError::Validation(
                "备份文件不能保存在 TieZ 管理的数据目录内".to_string(),
            ));
        }
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error("无法创建备份目录", err))?;
    }

    let temp_root = std::env::temp_dir().join(format!("tiez-backup-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_root).map_err(|err| io_error("无法创建备份暂存目录", err))?;
    let snapshot = temp_root.join(DATABASE_NAME);
    let entry_count = {
        let conn = state
            .conn
            .lock()
            .map_err(|err| AppError::Database(err.to_string()))?;
        conn.execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()])?;
        conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
            row.get(0)
        })?
    };

    let mut sources = vec![(snapshot.clone(), DATABASE_NAME.to_string())];
    collect_directory_files(&data_dir, Path::new("attachments"), &mut sources)?;
    collect_directory_files(&data_dir, Path::new("emoji_favorites"), &mut sources)?;
    let mut manifest_files = Vec::with_capacity(sources.len());
    for (source, archive_path) in &sources {
        let (size, sha256) = hash_file(source)?;
        manifest_files.push(BackupFileEntry {
            path: archive_path.clone(),
            size,
            sha256,
        });
    }
    let manifest = BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: now_ms(),
        source_data_path: data_dir.to_string_lossy().to_string(),
        entry_count,
        files: manifest_files,
    };

    let temp_archive = destination.with_extension("tiez-backup.tmp");
    if temp_archive.exists() {
        fs::remove_file(&temp_archive).map_err(|err| io_error("无法清理旧备份临时文件", err))?;
    }
    let archive_file =
        File::create(&temp_archive).map_err(|err| io_error("无法创建备份文件", err))?;
    let mut zip = ZipWriter::new(archive_file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (source, archive_path) in &sources {
        zip.start_file(archive_path, options)
            .map_err(|err| io_error("无法写入备份条目", err))?;
        let mut reader = File::open(source).map_err(|err| io_error("无法读取待备份文件", err))?;
        std::io::copy(&mut reader, &mut zip).map_err(|err| io_error("无法写入备份内容", err))?;
    }
    zip.start_file(MANIFEST_NAME, options)
        .map_err(|err| io_error("无法写入备份清单", err))?;
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| AppError::Internal(format!("无法生成备份清单: {err}")))?;
    zip.write_all(&manifest_json)
        .map_err(|err| io_error("无法写入备份清单", err))?;
    zip.finish().map_err(|err| io_error("无法完成备份", err))?;

    if destination.exists() {
        fs::remove_file(&destination).map_err(|err| io_error("无法覆盖旧备份", err))?;
    }
    fs::rename(&temp_archive, &destination).map_err(|err| io_error("无法保存备份", err))?;
    let _ = fs::remove_dir_all(&temp_root);
    Ok(info_from_manifest(&manifest, &destination))
}

#[tauri::command]
pub fn inspect_backup(path: String) -> AppResult<BackupInfo> {
    let path = PathBuf::from(path.trim());
    let manifest = read_and_validate_manifest(&path, true)?;
    Ok(info_from_manifest(&manifest, &path))
}

#[tauri::command]
pub fn schedule_backup_restore(
    app_data: State<'_, AppDataDir>,
    path: String,
) -> AppResult<BackupInfo> {
    let source = PathBuf::from(path.trim());
    let manifest = read_and_validate_manifest(&source, true)?;
    let data_dir = app_data
        .0
        .lock()
        .map_err(|err| AppError::Internal(err.to_string()))?
        .clone();
    let pending = data_dir.join(PENDING_BACKUP_NAME);
    let temporary = data_dir.join(format!("{PENDING_BACKUP_NAME}.tmp"));
    fs::copy(&source, &temporary).map_err(|err| io_error("无法暂存待恢复备份", err))?;
    if pending.exists() {
        fs::remove_file(&pending).map_err(|err| io_error("无法替换待恢复备份", err))?;
    }
    fs::rename(&temporary, &pending).map_err(|err| io_error("无法安排恢复", err))?;
    Ok(info_from_manifest(&manifest, &source))
}

pub fn apply_pending_restore(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let pending = data_dir.join(PENDING_BACKUP_NAME);
    if !pending.is_file() {
        return Ok(());
    }

    let quarantine_pending = |reason: &str| {
        eprintln!(">>> [RESTORE] Quarantining invalid pending backup: {reason}");
        let failed = data_dir.join(format!("restore-failed-{}.tiez-backup", now_ms()));
        let _ = fs::rename(&pending, failed);
    };

    let manifest = match read_and_validate_manifest(&pending, true) {
        Ok(manifest) => manifest,
        Err(error) => {
            quarantine_pending(&error.to_string());
            return Ok(());
        }
    };
    let staging = data_dir.join(format!(".tiez-restore-staging-{}", uuid::Uuid::new_v4()));
    if let Err(error) = extract_backup(&pending, &staging, &manifest) {
        quarantine_pending(&error.to_string());
        let _ = fs::remove_dir_all(&staging);
        return Ok(());
    }

    let staged_db = staging.join(DATABASE_NAME);
    let old_base = PathBuf::from(&manifest.source_data_path);
    let prepare_result = (|| -> AppResult<()> {
        crate::app::commands::system_cmd::rewrite_attachment_paths_in_db(
            &staged_db, &old_base, data_dir,
        )?;
        crate::app::commands::system_cmd::rewrite_emoji_favorites_in_db(
            &staged_db, &old_base, data_dir,
        )?;
        crate::app::commands::system_cmd::rewrite_custom_background_in_db(
            &staged_db, &old_base, data_dir,
        )?;
        validate_database(&staged_db)
    })();
    if let Err(error) = prepare_result {
        quarantine_pending(&error.to_string());
        let _ = fs::remove_dir_all(&staging);
        return Ok(());
    }

    let rollback = data_dir.join(format!("restore-rollback-{}", now_ms()));
    fs::create_dir_all(&rollback)?;
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
                    let _ = fs::remove_dir_all(&staging);
                    return Err(Box::new(error));
                }
            }
        }
    }

    let install_result = (|| -> AppResult<()> {
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
        let _ = fs::remove_dir_all(&staging);
        return Err(Box::new(error));
    }

    let _ = fs::remove_dir_all(&staging);
    fs::remove_file(&pending)?;
    // Keep the rollback directory for seven days so a user can recover manually.
    if let Ok(entries) = fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("restore-rollback-") || entry.path() == rollback {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .map(|age| age > Duration::from_secs(7 * 24 * 60 * 60))
                .unwrap_or(false);
            if old {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::repository::migrations::run_migrations;

    fn test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tiez-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn write_test_backup(
        archive_path: &Path,
        source_base: &Path,
        content: &str,
        valid_hash: bool,
    ) -> BackupManifest {
        let build_dir = test_dir("backup-build");
        let database = build_dir.join(DATABASE_NAME);
        let conn = Connection::open(&database).expect("open test backup database");
        run_migrations(&conn).expect("migrate test backup database");
        let attachment_path = source_base.join("attachments").join("image.png");
        conn.execute(
            "INSERT INTO clipboard_history
                (content_type, content, source_app, timestamp, preview, content_hash)
             VALUES ('image', ?1, 'test', 1, 'image', 7)",
            [attachment_path.to_string_lossy().as_ref()],
        )
        .expect("insert test image entry");
        drop(conn);

        let attachment = build_dir.join("image.png");
        fs::write(&attachment, content).expect("write test attachment");
        let sources = vec![
            (database, DATABASE_NAME.to_string()),
            (attachment, "attachments/image.png".to_string()),
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
            app_version: "test".to_string(),
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
        let _ = fs::remove_dir_all(build_dir);
        manifest
    }

    #[test]
    fn validates_and_extracts_complete_backup() {
        let root = test_dir("backup-validation");
        let source_base = root.join("old-data");
        let archive = root.join("valid.tiez-backup");
        write_test_backup(&archive, &source_base, "attachment", true);

        let manifest = read_and_validate_manifest(&archive, true).expect("validate backup");
        assert_eq!(manifest.entry_count, 1);
        let extracted = root.join("extracted");
        extract_backup(&archive, &extracted, &manifest).expect("extract backup");
        assert_eq!(
            fs::read_to_string(extracted.join("attachments/image.png"))
                .expect("read restored attachment"),
            "attachment"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_backup_with_wrong_checksum() {
        let root = test_dir("backup-checksum");
        let archive = root.join("invalid.tiez-backup");
        write_test_backup(&archive, &root.join("old-data"), "attachment", false);
        let error = read_and_validate_manifest(&archive, true).expect_err("reject bad checksum");
        assert!(error.to_string().contains("校验失败"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn applies_pending_restore_and_keeps_rollback() {
        let root = test_dir("backup-restore");
        let data_dir = root.join("current-data");
        fs::create_dir_all(&data_dir).expect("create current data directory");
        let current_db = data_dir.join(DATABASE_NAME);
        let conn = Connection::open(&current_db).expect("open current database");
        run_migrations(&conn).expect("migrate current database");
        conn.execute(
            "INSERT INTO clipboard_history
                (content_type, content, source_app, timestamp, preview, content_hash)
             VALUES ('text', 'old entry', 'test', 1, 'old', 1)",
            [],
        )
        .expect("insert current entry");
        drop(conn);

        let source_base = root.join("source-data");
        let pending = data_dir.join(PENDING_BACKUP_NAME);
        write_test_backup(&pending, &source_base, "restored attachment", true);
        apply_pending_restore(&data_dir).expect("apply pending restore");

        assert!(!pending.exists());
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
        let rollback_count = fs::read_dir(&data_dir)
            .expect("list restore data directory")
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("restore-rollback-")
            })
            .count();
        assert_eq!(rollback_count, 1);
        let _ = fs::remove_dir_all(root);
    }
}
