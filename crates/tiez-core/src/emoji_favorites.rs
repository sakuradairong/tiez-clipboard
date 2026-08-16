//! Tauri-independent storage for image Emoji favorites.
//!
//! The adapter preserves the existing TieZ contract: image files live below
//! `emoji_favorites/`, while their ordered absolute paths are stored in the
//! `app.emoji_favorites` SQLite setting. Native frontends use this module so
//! backups, data-path migration, and cloud synchronization keep working with
//! the same production data.

use image::ImageFormat;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FAVORITES_SETTING_KEY: &str = "app.emoji_favorites";
const FAVORITES_DIRECTORY: &str = "emoji_favorites";
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmojiFavoriteItem {
    pub path: String,
    pub file_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmojiFavoritesSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub items: Vec<EmojiFavoriteItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmojiFavoritesMutation {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub action: &'static str,
    pub changed: bool,
    pub path: Option<String>,
    pub items: Vec<EmojiFavoriteItem>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmojiFavoritesErrorKind {
    InvalidDatabase,
    ReadOnly,
    Storage,
    Validation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiFavoritesError {
    kind: EmojiFavoritesErrorKind,
    message: String,
}

impl EmojiFavoritesError {
    fn new(kind: EmojiFavoritesErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> EmojiFavoritesErrorKind {
        self.kind
    }
}

impl fmt::Display for EmojiFavoritesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EmojiFavoritesError {}

#[derive(Debug)]
enum EmojiFavoritesAdapter {
    Memory(Vec<String>),
    Sqlite {
        database_path: PathBuf,
        data_dir: PathBuf,
        read_only: bool,
    },
}

#[derive(Debug)]
pub struct EmojiFavorites {
    adapter: EmojiFavoritesAdapter,
    generation: u64,
}

impl EmojiFavorites {
    pub fn in_memory() -> Self {
        Self {
            adapter: EmojiFavoritesAdapter::Memory(Vec::new()),
            generation: 1,
        }
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, EmojiFavoritesError> {
        let database_path = database_path.into();
        let connection = open_connection(&database_path, read_only)?;
        connection
            .query_row("SELECT 1 FROM settings LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(|error| storage_error("failed to inspect settings table", error))?;
        Ok(Self {
            adapter: EmojiFavoritesAdapter::Sqlite {
                database_path,
                data_dir: data_dir.into(),
                read_only,
            },
            generation: 1,
        })
    }

    pub fn snapshot(&mut self) -> Result<EmojiFavoritesSnapshot, EmojiFavoritesError> {
        let paths = self.current_paths()?;
        let needs_repair = match &self.adapter {
            EmojiFavoritesAdapter::Memory(stored) => stored != &paths,
            EmojiFavoritesAdapter::Sqlite {
                database_path,
                read_only,
                ..
            } => !*read_only && read_setting_paths(database_path, false)? != paths,
        };
        if needs_repair {
            self.store_paths(&paths)?;
            self.generation = self.generation.saturating_add(1);
        }
        Ok(self.snapshot_from_paths(paths))
    }

    pub fn import_file(
        &mut self,
        source_path: impl AsRef<Path>,
    ) -> Result<EmojiFavoritesMutation, EmojiFavoritesError> {
        self.ensure_writable()?;
        let source_path = source_path.as_ref();
        if source_path.as_os_str().is_empty() || !source_path.is_file() {
            return Err(validation_error("请选择存在的图片文件"));
        }
        let bytes = fs::read(source_path)
            .map_err(|error| storage_error("failed to read Emoji favorite", error))?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return Err(validation_error("图片为空或超过 32 MiB 限制"));
        }
        let extension = image_extension(&bytes)
            .ok_or_else(|| validation_error("仅支持 PNG、JPEG、GIF 或 WebP 图片"))?;
        let mut paths = self.current_paths()?;

        let (path, created) = match &self.adapter {
            EmojiFavoritesAdapter::Memory(_) => (
                source_path
                    .canonicalize()
                    .unwrap_or_else(|_| source_path.to_path_buf())
                    .to_string_lossy()
                    .into_owned(),
                false,
            ),
            EmojiFavoritesAdapter::Sqlite { data_dir, .. } => {
                save_managed_image(data_dir, &bytes, extension)?
            }
        };

        let changed = !paths
            .iter()
            .any(|candidate| paths_refer_to_same_file(candidate, &path));
        if changed {
            paths.push(path.clone());
            if let Err(error) = self.store_paths(&paths) {
                if created {
                    let _ = fs::remove_file(&path);
                }
                return Err(error);
            }
            self.generation = self.generation.saturating_add(1);
        }

        Ok(self.mutation_from_paths(
            "add",
            changed,
            Some(path),
            paths,
            if changed {
                "已添加图片表情收藏"
            } else {
                "图片表情已在收藏中"
            },
        ))
    }

    pub fn remove(
        &mut self,
        favorite_path: impl AsRef<Path>,
    ) -> Result<EmojiFavoritesMutation, EmojiFavoritesError> {
        self.ensure_writable()?;
        let favorite_path = favorite_path.as_ref();
        let requested = favorite_path.to_string_lossy().into_owned();
        if requested.trim().is_empty() {
            return Err(validation_error("收藏路径不能为空"));
        }

        let mut paths = self.current_paths()?;
        let Some(index) = paths
            .iter()
            .position(|candidate| paths_refer_to_same_file(candidate, &requested))
        else {
            return Ok(self.mutation_from_paths(
                "remove",
                false,
                None,
                paths,
                "图片表情已不在收藏中",
            ));
        };

        let staged = self.stage_managed_file_for_removal(favorite_path)?;
        paths.remove(index);
        if let Err(error) = self.store_paths(&paths) {
            if let Some((original, staged)) = staged.as_ref() {
                let _ = fs::rename(staged, original);
            }
            return Err(error);
        }
        self.generation = self.generation.saturating_add(1);

        let cleanup_deferred = staged
            .as_ref()
            .is_some_and(|(_, staged)| fs::remove_file(staged).is_err());
        Ok(self.mutation_from_paths(
            "remove",
            true,
            Some(requested),
            paths,
            if cleanup_deferred {
                "已移除图片表情收藏；临时文件将在后续清理"
            } else {
                "已移除图片表情收藏"
            },
        ))
    }

    pub fn favorite_path_for_paste(
        &self,
        favorite_path: impl AsRef<Path>,
    ) -> Result<String, EmojiFavoritesError> {
        let requested = favorite_path.as_ref().to_string_lossy().into_owned();
        let paths = self.current_paths()?;
        let Some(stored_path) = paths
            .iter()
            .find(|candidate| paths_refer_to_same_file(candidate, &requested))
        else {
            return Err(validation_error("图片不在 Emoji 收藏列表中"));
        };
        let bytes = fs::read(stored_path)
            .map_err(|error| storage_error("failed to read Emoji favorite for paste", error))?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES || image_extension(&bytes).is_none() {
            return Err(validation_error("收藏图片不可用或格式不受支持"));
        }
        Ok(stored_path.clone())
    }

    fn ensure_writable(&self) -> Result<(), EmojiFavoritesError> {
        if self.read_only() {
            Err(EmojiFavoritesError::new(
                EmojiFavoritesErrorKind::ReadOnly,
                "只读数据副本不能修改 Emoji 收藏",
            ))
        } else {
            Ok(())
        }
    }

    fn adapter_name(&self) -> &'static str {
        match &self.adapter {
            EmojiFavoritesAdapter::Memory(_) => "memory",
            EmojiFavoritesAdapter::Sqlite {
                read_only: true, ..
            } => "sqlite-read-only",
            EmojiFavoritesAdapter::Sqlite {
                read_only: false, ..
            } => "sqlite",
        }
    }

    fn read_only(&self) -> bool {
        matches!(
            &self.adapter,
            EmojiFavoritesAdapter::Sqlite {
                read_only: true,
                ..
            }
        )
    }

    fn current_paths(&self) -> Result<Vec<String>, EmojiFavoritesError> {
        match &self.adapter {
            EmojiFavoritesAdapter::Memory(paths) => {
                Ok(normalize_existing_paths(paths.iter().cloned()))
            }
            EmojiFavoritesAdapter::Sqlite {
                database_path,
                data_dir,
                read_only,
            } => {
                let stored = read_setting_paths(database_path, *read_only)?;
                let disk = list_managed_images(data_dir)?;
                Ok(normalize_existing_paths(stored.into_iter().chain(disk)))
            }
        }
    }

    fn store_paths(&mut self, paths: &[String]) -> Result<(), EmojiFavoritesError> {
        match &mut self.adapter {
            EmojiFavoritesAdapter::Memory(stored) => {
                *stored = paths.to_vec();
                Ok(())
            }
            EmojiFavoritesAdapter::Sqlite {
                database_path,
                read_only,
                ..
            } => {
                if *read_only {
                    return Err(EmojiFavoritesError::new(
                        EmojiFavoritesErrorKind::ReadOnly,
                        "只读数据副本不能修改 Emoji 收藏",
                    ));
                }
                write_setting_paths(database_path, paths)
            }
        }
    }

    fn stage_managed_file_for_removal(
        &self,
        favorite_path: &Path,
    ) -> Result<Option<(PathBuf, PathBuf)>, EmojiFavoritesError> {
        let EmojiFavoritesAdapter::Sqlite { data_dir, .. } = &self.adapter else {
            return Ok(None);
        };
        let favorites_dir = data_dir.join(FAVORITES_DIRECTORY);
        let Ok(favorites_dir) = favorites_dir.canonicalize() else {
            return Ok(None);
        };
        let Ok(target) = favorite_path.canonicalize() else {
            return Ok(None);
        };
        if !target.starts_with(&favorites_dir) || !target.is_file() {
            return Ok(None);
        }

        let file_name = target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("favorite");
        let staged =
            target.with_file_name(format!(".{file_name}.{}.tiez-removing", unique_suffix()));
        fs::rename(&target, &staged)
            .map_err(|error| storage_error("failed to stage Emoji favorite removal", error))?;
        Ok(Some((target, staged)))
    }

    fn snapshot_from_paths(&self, paths: Vec<String>) -> EmojiFavoritesSnapshot {
        EmojiFavoritesSnapshot {
            adapter: self.adapter_name(),
            read_only: self.read_only(),
            generation: self.generation,
            items: favorite_items(paths),
        }
    }

    fn mutation_from_paths(
        &self,
        action: &'static str,
        changed: bool,
        path: Option<String>,
        paths: Vec<String>,
        message: impl Into<String>,
    ) -> EmojiFavoritesMutation {
        EmojiFavoritesMutation {
            adapter: self.adapter_name(),
            read_only: self.read_only(),
            generation: self.generation,
            action,
            changed,
            path,
            items: favorite_items(paths),
            message: message.into(),
        }
    }
}

fn favorite_items(paths: Vec<String>) -> Vec<EmojiFavoriteItem> {
    paths
        .into_iter()
        .map(|path| {
            let file_name = Path::new(&path)
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            EmojiFavoriteItem { path, file_name }
        })
        .collect()
}

fn normalize_existing_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for path in paths {
        let path = path.trim();
        if path.is_empty() || !is_supported_image_path(Path::new(path)) {
            continue;
        }
        let path = path.to_owned();
        if seen.insert(path_identity(Path::new(&path))) {
            normalized.push(path);
        }
    }
    normalized
}

fn paths_refer_to_same_file(left: impl AsRef<Path>, right: impl AsRef<Path>) -> bool {
    path_identity(left.as_ref()) == path_identity(right.as_ref())
}

fn path_identity(path: &Path) -> String {
    let normalized = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn is_supported_image_path(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|value| value.to_str())
            .and_then(normalize_extension)
            .is_some()
}

fn normalize_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpg"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::WebP => Some("webp"),
        _ => None,
    }
}

fn save_managed_image(
    data_dir: &Path,
    bytes: &[u8],
    extension: &str,
) -> Result<(String, bool), EmojiFavoritesError> {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let hash = hasher.finish();
    let favorites_dir = data_dir.join(FAVORITES_DIRECTORY);
    fs::create_dir_all(&favorites_dir)
        .map_err(|error| storage_error("failed to create Emoji favorites directory", error))?;
    let target = favorites_dir.join(format!("fav_{hash:x}.{extension}"));
    if target.is_file() {
        return Ok((target.to_string_lossy().into_owned(), false));
    }

    let temporary = favorites_dir.join(format!(".fav_{hash:x}.{}.tmp", unique_suffix()));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &target)
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        if target.is_file() {
            return Ok((target.to_string_lossy().into_owned(), false));
        }
        return Err(storage_error("failed to save Emoji favorite", error));
    }
    Ok((target.to_string_lossy().into_owned(), true))
}

fn list_managed_images(data_dir: &Path) -> Result<Vec<String>, EmojiFavoritesError> {
    let favorites_dir = data_dir.join(FAVORITES_DIRECTORY);
    if !favorites_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(&favorites_dir)
        .map_err(|error| storage_error("failed to list Emoji favorites", error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_supported_image_path(path))
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn read_setting_paths(
    database_path: &Path,
    read_only: bool,
) -> Result<Vec<String>, EmojiFavoritesError> {
    let connection = open_connection(database_path, read_only)?;
    let raw = connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [FAVORITES_SETTING_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| storage_error("failed to read Emoji favorites setting", error))?
        .unwrap_or_default();
    Ok(serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default())
}

fn write_setting_paths(database_path: &Path, paths: &[String]) -> Result<(), EmojiFavoritesError> {
    let serialized = serde_json::to_string(paths)
        .map_err(|error| storage_error("failed to serialize Emoji favorites", error))?;
    let connection = open_connection(database_path, false)?;
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [FAVORITES_SETTING_KEY, serialized.as_str()],
        )
        .map_err(|error| storage_error("failed to write Emoji favorites setting", error))?;
    Ok(())
}

fn open_connection(
    database_path: &Path,
    read_only: bool,
) -> Result<Connection, EmojiFavoritesError> {
    if !database_path.is_file() {
        return Err(EmojiFavoritesError::new(
            EmojiFavoritesErrorKind::InvalidDatabase,
            format!("database does not exist: {}", database_path.display()),
        ));
    }
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(database_path, flags)
        .map_err(|error| storage_error("failed to open Emoji favorites database", error))?;
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| storage_error("failed to configure Emoji favorites database", error))?;
    Ok(connection)
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn validation_error(message: impl Into<String>) -> EmojiFavoritesError {
    EmojiFavoritesError::new(EmojiFavoritesErrorKind::Validation, message)
}

fn storage_error(context: &str, error: impl fmt::Display) -> EmojiFavoritesError {
    EmojiFavoritesError::new(
        EmojiFavoritesErrorKind::Storage,
        format!("{context}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tiez-emoji-favorites-{name}-{}-{}",
                std::process::id(),
                unique_suffix()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn database(&self) -> PathBuf {
            let path = self.path.join("clipboard.db");
            Connection::open(&path)
                .unwrap()
                .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
                .unwrap();
            path
        }

        fn png(&self, name: &str, color: [u8; 4]) -> PathBuf {
            let path = self.path.join(name);
            RgbaImage::from_pixel(2, 2, Rgba(color))
                .save(&path)
                .unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn sqlite_import_and_remove_preserve_the_existing_setting_contract() {
        let directory = TestDirectory::new("round-trip");
        let database = directory.database();
        let source = directory.png("source.png", [20, 40, 220, 255]);
        let mut favorites = EmojiFavorites::open_sqlite(&database, &directory.path, false).unwrap();

        let added = favorites.import_file(&source).unwrap();

        assert!(added.changed);
        assert_eq!(added.items.len(), 1);
        assert!(added.items[0].path.contains("emoji_favorites"));
        assert!(added.items[0].file_name.starts_with("fav_"));
        assert!(Path::new(&added.items[0].path).is_file());
        let stored: String = Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [FAVORITES_SETTING_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&stored).unwrap(),
            vec![added.items[0].path.clone()]
        );

        let duplicate = favorites.import_file(&source).unwrap();
        assert!(!duplicate.changed);
        assert_eq!(duplicate.items, added.items);

        let removed = favorites.remove(&added.items[0].path).unwrap();
        assert!(removed.changed);
        assert!(removed.items.is_empty());
        assert!(!Path::new(&added.items[0].path).exists());
        let stored: String = Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [FAVORITES_SETTING_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "[]");
    }

    #[test]
    fn writable_snapshot_repairs_disk_items_without_losing_setting_order() {
        let directory = TestDirectory::new("repair");
        let database = directory.database();
        let favorites_dir = directory.path.join(FAVORITES_DIRECTORY);
        fs::create_dir_all(&favorites_dir).unwrap();
        let first = directory.png("first.png", [255, 0, 0, 255]);
        let first_managed = favorites_dir.join("z-first.png");
        fs::copy(first, &first_managed).unwrap();
        let second = directory.png("second.png", [0, 255, 0, 255]);
        let second_managed = favorites_dir.join("a-second.png");
        fs::copy(second, &second_managed).unwrap();
        let first_path = first_managed.to_string_lossy().into_owned();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                [
                    FAVORITES_SETTING_KEY,
                    serde_json::to_string(&vec![first_path.clone()])
                        .unwrap()
                        .as_str(),
                ],
            )
            .unwrap();
        let mut favorites = EmojiFavorites::open_sqlite(&database, &directory.path, false).unwrap();

        let snapshot = favorites.snapshot().unwrap();

        assert_eq!(snapshot.items[0].path, first_path);
        assert_eq!(snapshot.items.len(), 2);
        assert_eq!(snapshot.generation, 2);
        let stored: String = Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [FAVORITES_SETTING_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&stored).unwrap().len(),
            2
        );
    }

    #[test]
    fn read_only_adapter_rejects_mutation_and_does_not_touch_files() {
        let directory = TestDirectory::new("read-only");
        let database = directory.database();
        let source = directory.png("source.png", [100, 20, 30, 255]);
        let before = fs::read(&database).unwrap();
        let mut favorites = EmojiFavorites::open_sqlite(&database, &directory.path, true).unwrap();

        let error = favorites.import_file(&source).unwrap_err();

        assert_eq!(error.kind(), EmojiFavoritesErrorKind::ReadOnly);
        assert_eq!(fs::read(&database).unwrap(), before);
        assert!(!directory.path.join(FAVORITES_DIRECTORY).exists());
    }

    #[test]
    fn removing_an_external_setting_path_never_deletes_the_source_file() {
        let directory = TestDirectory::new("external");
        let database = directory.database();
        let external = directory.png("external.png", [5, 6, 7, 255]);
        let external_path = external.to_string_lossy().into_owned();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                [
                    FAVORITES_SETTING_KEY,
                    serde_json::to_string(&vec![external_path.clone()])
                        .unwrap()
                        .as_str(),
                ],
            )
            .unwrap();
        let mut favorites = EmojiFavorites::open_sqlite(&database, &directory.path, false).unwrap();

        let removed = favorites.remove(&external_path).unwrap();

        assert!(removed.changed);
        assert!(external.is_file());
    }

    #[test]
    fn import_rejects_extension_spoofing() {
        let directory = TestDirectory::new("spoof");
        let database = directory.database();
        let source = directory.path.join("not-an-image.png");
        fs::write(&source, b"plain text").unwrap();
        let mut favorites = EmojiFavorites::open_sqlite(&database, &directory.path, false).unwrap();

        let error = favorites.import_file(source).unwrap_err();

        assert_eq!(error.kind(), EmojiFavoritesErrorKind::Validation);
        assert!(!directory.path.join(FAVORITES_DIRECTORY).exists());
    }
}
