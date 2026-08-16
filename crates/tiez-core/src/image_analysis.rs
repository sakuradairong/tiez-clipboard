//! Tauri-independent image OCR and QR-code analysis for native TieZ frontends.

use crate::encryption::{decrypt_value, ENCRYPT_PREFIX};
use base64::Engine;
use image::GenericImageView;
use rqrr::PreparedImage;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const OCR_MAX_IMAGE_DIMENSION: u32 = 2600;
const MAX_INLINE_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_INLINE_IMAGE_BASE64_BYTES: usize = MAX_INLINE_IMAGE_BYTES.div_ceil(3) * 4;
const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const SENSITIVE_TAGS: &[&str] = &["sensitive", "密码", "password"];
static TEMPORARY_IMAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAnalysisErrorKind {
    Storage,
    NotFound,
    Validation,
    Io,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageAnalysisError {
    kind: ImageAnalysisErrorKind,
    message: String,
}

impl ImageAnalysisError {
    fn new(kind: ImageAnalysisErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ImageAnalysisErrorKind {
        self.kind
    }
}

impl fmt::Display for ImageAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ImageAnalysisError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAnalysisResult {
    pub text: String,
    pub qr_codes: Vec<String>,
    pub language: Option<String>,
    pub analyzed_at: i64,
    pub cached: bool,
    pub persisted: bool,
    pub ocr_available: bool,
    pub ocr_error: Option<String>,
}

#[derive(Debug)]
pub struct ImageAnalysisWork {
    entry_id: i64,
    content_hash: i64,
    content: String,
    sensitive: bool,
}

#[derive(Debug)]
pub enum PreparedImageAnalysis {
    Cached(ImageAnalysisResult),
    Pending(ImageAnalysisWork),
}

struct TemporaryImage {
    path: PathBuf,
    remove_on_drop: bool,
}

impl TemporaryImage {
    fn borrowed(path: PathBuf) -> Self {
        Self {
            path,
            remove_on_drop: false,
        }
    }

    fn owned(path: PathBuf) -> Self {
        Self {
            path,
            remove_on_drop: true,
        }
    }
}

impl Drop for TemporaryImage {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn get_image_analysis(
    connection: &Connection,
    entry_id: i64,
) -> Result<Option<ImageAnalysisResult>, ImageAnalysisError> {
    let row = connection
        .query_row(
            "SELECT content_hash, tags, content
             FROM clipboard_history
             WHERE id = ?1 AND content_type = 'image'",
            [entry_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;

    let Some((content_hash, tags_json, content)) = row else {
        return Ok(None);
    };
    if tags_are_sensitive(&parse_tags(&tags_json)) || content.starts_with(ENCRYPT_PREFIX) {
        return Ok(None);
    }

    read_cached_analysis(connection, entry_id, content_hash)
}

pub fn prepare_image_analysis(
    connection: &Connection,
    entry_id: i64,
    force: bool,
) -> Result<PreparedImageAnalysis, ImageAnalysisError> {
    let row = connection
        .query_row(
            "SELECT content_type, content, content_hash, tags
             FROM clipboard_history
             WHERE id = ?1",
            [entry_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| {
            ImageAnalysisError::new(ImageAnalysisErrorKind::NotFound, "找不到图片条目")
        })?;

    let (content_type, mut content, content_hash, tags_json) = row;
    if content_type != "image" {
        return Err(ImageAnalysisError::new(
            ImageAnalysisErrorKind::Validation,
            "只有图片条目可以执行 OCR",
        ));
    }

    let sensitive =
        tags_are_sensitive(&parse_tags(&tags_json)) || content.starts_with(ENCRYPT_PREFIX);
    if !force && !sensitive {
        if let Some(cached) = read_cached_analysis(connection, entry_id, content_hash)? {
            return Ok(PreparedImageAnalysis::Cached(cached));
        }
    }

    if content.starts_with(ENCRYPT_PREFIX) {
        content = decrypt_value(&content).ok_or_else(|| {
            ImageAnalysisError::new(
                ImageAnalysisErrorKind::Validation,
                "当前 Windows 账户无法解密此图片条目",
            )
        })?;
    }

    Ok(PreparedImageAnalysis::Pending(ImageAnalysisWork {
        entry_id,
        content_hash,
        content,
        sensitive,
    }))
}

pub fn analyze_prepared_image(
    work: &ImageAnalysisWork,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let source = image_file_from_content(&work.content)?;
    let qr_path = source.path.clone();
    let ocr_image = prepare_ocr_image(&source.path)?;
    let qr_codes = decode_qr_codes(&qr_path);
    let ocr = recognize_text(&ocr_image.path);
    let (text, language, ocr_error) = match ocr {
        Ok((text, language)) => (text, language, None),
        Err(error) => (String::new(), None, Some(error)),
    };

    Ok(ImageAnalysisResult {
        text,
        qr_codes,
        language,
        analyzed_at: now_ms(),
        cached: false,
        persisted: false,
        ocr_available: cfg!(target_os = "windows"),
        ocr_error,
    })
}

pub fn finish_image_analysis(
    connection: &Connection,
    work: &ImageAnalysisWork,
    mut result: ImageAnalysisResult,
    allow_persist: bool,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    if !allow_persist || work.sensitive {
        return Ok(result);
    }

    let current = connection
        .query_row(
            "SELECT content_hash, tags, content
             FROM clipboard_history
             WHERE id = ?1 AND content_type = 'image'",
            [work.entry_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((current_hash, current_tags, current_content)) = current else {
        return Ok(result);
    };
    if current_hash != work.content_hash
        || tags_are_sensitive(&parse_tags(&current_tags))
        || current_content.starts_with(ENCRYPT_PREFIX)
    {
        return Ok(result);
    }

    connection
        .execute(
            "INSERT INTO clipboard_image_analysis
                (entry_id, content_hash, ocr_text, qr_codes, language, analyzed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(entry_id) DO UPDATE SET
                content_hash = excluded.content_hash,
                ocr_text = excluded.ocr_text,
                qr_codes = excluded.qr_codes,
                language = excluded.language,
                analyzed_at = excluded.analyzed_at",
            params![
                work.entry_id,
                work.content_hash,
                result.text,
                serde_json::to_string(&result.qr_codes).unwrap_or_else(|_| "[]".to_owned()),
                result.language,
                result.analyzed_at,
            ],
        )
        .map_err(storage_error)?;
    result.persisted = true;
    Ok(result)
}

pub fn analyze_image_entry(
    connection: &Connection,
    entry_id: i64,
    force: bool,
    allow_persist: bool,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    match prepare_image_analysis(connection, entry_id, force)? {
        PreparedImageAnalysis::Cached(result) => Ok(result),
        PreparedImageAnalysis::Pending(work) => {
            let result = analyze_prepared_image(&work)?;
            finish_image_analysis(connection, &work, result, allow_persist)
        }
    }
}

pub fn get_image_analysis_from_database(
    database_path: &Path,
    entry_id: i64,
) -> Result<Option<ImageAnalysisResult>, ImageAnalysisError> {
    let connection = open_database(database_path, true)?;
    get_image_analysis(&connection, entry_id)
}

pub fn analyze_image_entry_from_database(
    database_path: &Path,
    entry_id: i64,
    force: bool,
    read_only: bool,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let connection = open_database(database_path, read_only)?;
    analyze_image_entry(&connection, entry_id, force, !read_only)
}

fn open_database(path: &Path, read_only: bool) -> Result<Connection, ImageAnalysisError> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(path, flags).map_err(storage_error)
}

fn read_cached_analysis(
    connection: &Connection,
    entry_id: i64,
    content_hash: i64,
) -> Result<Option<ImageAnalysisResult>, ImageAnalysisError> {
    if !image_analysis_table_exists(connection)? {
        return Ok(None);
    }
    connection
        .query_row(
            "SELECT ocr_text, qr_codes, language, analyzed_at
             FROM clipboard_image_analysis
             WHERE entry_id = ?1 AND content_hash = ?2",
            params![entry_id, content_hash],
            |row| {
                let qr_json: String = row.get(1)?;
                Ok(ImageAnalysisResult {
                    text: row.get(0)?,
                    qr_codes: serde_json::from_str(&qr_json).unwrap_or_default(),
                    language: row.get(2)?,
                    analyzed_at: row.get(3)?,
                    cached: true,
                    persisted: true,
                    ocr_available: cfg!(target_os = "windows"),
                    ocr_error: None,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}

fn image_analysis_table_exists(connection: &Connection) -> Result<bool, ImageAnalysisError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM sqlite_master
                WHERE type = 'table' AND name = 'clipboard_image_analysis'
             )",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

fn image_file_from_content(content: &str) -> Result<TemporaryImage, ImageAnalysisError> {
    if !content.starts_with("data:image/") {
        let path = PathBuf::from(content);
        let metadata = std::fs::metadata(&path).map_err(|error| {
            ImageAnalysisError::new(
                ImageAnalysisErrorKind::Io,
                format!("图片文件不存在或无法读取：{error}"),
            )
        })?;
        if !metadata.is_file() {
            return Err(ImageAnalysisError::new(
                ImageAnalysisErrorKind::Validation,
                "图片文件不存在或已被删除",
            ));
        }
        if metadata.len() > MAX_IMAGE_FILE_BYTES {
            return Err(ImageAnalysisError::new(
                ImageAnalysisErrorKind::Validation,
                "图片文件超过 64 MB 分析上限",
            ));
        }
        return Ok(TemporaryImage::borrowed(path));
    }

    let encoded = content
        .split_once(',')
        .map(|(_, payload)| payload)
        .ok_or_else(|| {
            ImageAnalysisError::new(ImageAnalysisErrorKind::Validation, "图片数据格式无效")
        })?;
    if encoded.len() > MAX_INLINE_IMAGE_BASE64_BYTES {
        return Err(ImageAnalysisError::new(
            ImageAnalysisErrorKind::Validation,
            "内嵌图片超过 32 MB 分析上限",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| {
            ImageAnalysisError::new(
                ImageAnalysisErrorKind::Validation,
                format!("图片数据解码失败：{error}"),
            )
        })?;
    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return Err(ImageAnalysisError::new(
            ImageAnalysisErrorKind::Validation,
            "内嵌图片超过 32 MB 分析上限",
        ));
    }
    let image = image::load_from_memory(&bytes).map_err(image_error)?;
    let path = temporary_png_path();
    image.save(&path).map_err(image_error)?;
    Ok(TemporaryImage::owned(path))
}

fn prepare_ocr_image(path: &Path) -> Result<TemporaryImage, ImageAnalysisError> {
    let image = image::open(path).map_err(image_error)?;
    let (width, height) = image.dimensions();
    if width <= OCR_MAX_IMAGE_DIMENSION && height <= OCR_MAX_IMAGE_DIMENSION {
        return Ok(TemporaryImage::borrowed(path.to_path_buf()));
    }

    let resized = image.thumbnail(OCR_MAX_IMAGE_DIMENSION, OCR_MAX_IMAGE_DIMENSION);
    let output = temporary_png_path();
    resized.save(&output).map_err(image_error)?;
    Ok(TemporaryImage::owned(output))
}

fn decode_qr_codes(path: &Path) -> Vec<String> {
    let Ok(image) = image::open(path) else {
        return Vec::new();
    };
    let mut prepared = PreparedImage::prepare(image.to_luma8());
    let mut values = Vec::new();
    for grid in prepared.detect_grids() {
        if let Ok((_, value)) = grid.decode() {
            if !value.trim().is_empty() && !values.contains(&value) {
                values.push(value);
            }
        }
    }
    values
}

#[cfg(target_os = "windows")]
fn recognize_text(path: &Path) -> Result<(String, Option<String>), String> {
    use windows::core::HSTRING;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    struct WinRtApartment(bool);
    impl Drop for WinRtApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { RoUninitialize() };
            }
        }
    }

    let apartment = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let apartment = WinRtApartment(apartment.is_ok());
    let _ = &apartment;
    let canonical = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    let normalized = canonical.to_string_lossy().replace("\\\\?\\", "");
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(normalized))
        .map_err(|error| error.to_string())?
        .get()
        .map_err(|error| error.to_string())?;
    let stream = file
        .OpenAsync(FileAccessMode::Read)
        .map_err(|error| error.to_string())?
        .get()
        .map_err(|error| error.to_string())?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|error| error.to_string())?
        .get()
        .map_err(|error| error.to_string())?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|error| error.to_string())?
        .get()
        .map_err(|error| error.to_string())?;
    let engine =
        OcrEngine::TryCreateFromUserProfileLanguages().map_err(|error| error.to_string())?;
    let language = engine
        .RecognizerLanguage()
        .and_then(|value| value.LanguageTag())
        .ok()
        .map(|value| value.to_string_lossy());
    let text = engine
        .RecognizeAsync(&bitmap)
        .map_err(|error| error.to_string())?
        .get()
        .and_then(|result| result.Text())
        .map_err(|error| error.to_string())?
        .to_string_lossy();
    Ok((text.trim().to_owned(), language))
}

#[cfg(not(target_os = "windows"))]
fn recognize_text(_path: &Path) -> Result<(String, Option<String>), String> {
    Err("当前平台暂不支持系统 OCR".to_owned())
}

fn temporary_png_path() -> PathBuf {
    let sequence = TEMPORARY_IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tiez-ocr-{}-{}-{sequence}.png",
        std::process::id(),
        now_ms()
    ))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn parse_tags(value: &str) -> Vec<String> {
    serde_json::from_str(value).unwrap_or_default()
}

fn tags_are_sensitive(tags: &[String]) -> bool {
    tags.iter().any(|tag| {
        SENSITIVE_TAGS
            .iter()
            .any(|sensitive| sensitive.eq_ignore_ascii_case(tag))
    })
}

fn storage_error(error: rusqlite::Error) -> ImageAnalysisError {
    ImageAnalysisError::new(ImageAnalysisErrorKind::Storage, error.to_string())
}

fn image_error(error: image::ImageError) -> ImageAnalysisError {
    ImageAnalysisError::new(ImageAnalysisErrorKind::Io, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE clipboard_history (
                    id INTEGER PRIMARY KEY,
                    content_type TEXT NOT NULL,
                    content TEXT NOT NULL,
                    content_hash INTEGER NOT NULL,
                    tags TEXT NOT NULL DEFAULT '[]'
                 );
                 CREATE TABLE clipboard_image_analysis (
                    entry_id INTEGER PRIMARY KEY,
                    content_hash INTEGER NOT NULL,
                    ocr_text TEXT NOT NULL DEFAULT '',
                    qr_codes TEXT NOT NULL DEFAULT '[]',
                    language TEXT,
                    analyzed_at INTEGER NOT NULL
                 );",
            )
            .unwrap();
        connection
    }

    fn temporary_image(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tiez-image-analysis-test-{name}-{}-{}.png",
            std::process::id(),
            TEMPORARY_IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        image::DynamicImage::new_rgb8(8, 8).save(&path).unwrap();
        path
    }

    fn insert_image(connection: &Connection, path: &Path, tags: &str) {
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, content_hash, tags)
                 VALUES (1, 'image', ?1, 42, ?2)",
                params![path.to_string_lossy(), tags],
            )
            .unwrap();
    }

    #[test]
    fn matching_cached_analysis_is_returned() {
        let connection = connection();
        let path = temporary_image("cache");
        insert_image(&connection, &path, "[]");
        connection
            .execute(
                "INSERT INTO clipboard_image_analysis
                    (entry_id, content_hash, ocr_text, qr_codes, language, analyzed_at)
                 VALUES (1, 42, '中文 OCR', '[\"https://example.com\"]', 'zh-CN', 7)",
                [],
            )
            .unwrap();

        let result = get_image_analysis(&connection, 1).unwrap().unwrap();

        assert_eq!(result.text, "中文 OCR");
        assert_eq!(result.qr_codes, vec!["https://example.com"]);
        assert!(result.cached);
        assert!(result.persisted);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cached_analysis_is_hidden_after_an_entry_becomes_sensitive() {
        let connection = connection();
        let path = temporary_image("hidden-cache");
        insert_image(&connection, &path, "[]");
        connection
            .execute(
                "INSERT INTO clipboard_image_analysis
                    (entry_id, content_hash, ocr_text, qr_codes, language, analyzed_at)
                 VALUES (1, 42, 'secret OCR', '[\"secret QR\"]', 'en-US', 7)",
                [],
            )
            .unwrap();
        connection
            .execute("UPDATE clipboard_history SET tags = '[\"sensitive\"]'", [])
            .unwrap();

        assert!(get_image_analysis(&connection, 1).unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_analysis_table_is_treated_as_an_empty_cache() {
        let connection = connection();
        let path = temporary_image("missing-table");
        insert_image(&connection, &path, "[]");
        connection
            .execute("DROP TABLE clipboard_image_analysis", [])
            .unwrap();

        assert!(get_image_analysis(&connection, 1).unwrap().is_none());
        let result = analyze_image_entry(&connection, 1, true, false).unwrap();
        assert!(!result.persisted);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn analysis_persists_only_non_sensitive_unchanged_entries() {
        let connection = connection();
        let path = temporary_image("persist");
        insert_image(&connection, &path, "[]");

        let result = analyze_image_entry(&connection, 1, true, true).unwrap();

        assert!(!result.cached);
        assert!(result.persisted);
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_image_analysis WHERE entry_id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn analysis_never_persists_sensitive_or_newly_protected_entries() {
        let connection = connection();
        let path = temporary_image("sensitive");
        insert_image(&connection, &path, "[\"密码\"]");
        let sensitive = analyze_image_entry(&connection, 1, true, true).unwrap();
        assert!(!sensitive.persisted);

        connection
            .execute("UPDATE clipboard_history SET tags = '[]'", [])
            .unwrap();
        let work = match prepare_image_analysis(&connection, 1, true).unwrap() {
            PreparedImageAnalysis::Pending(work) => work,
            PreparedImageAnalysis::Cached(_) => panic!("forced analysis must not use cache"),
        };
        let result = analyze_prepared_image(&work).unwrap();
        connection
            .execute("UPDATE clipboard_history SET tags = '[\"sensitive\"]'", [])
            .unwrap();
        let protected = finish_image_analysis(&connection, &work, result, true).unwrap();
        assert!(!protected.persisted);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM clipboard_image_analysis", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn non_image_entries_are_rejected() {
        let connection = connection();
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, content_hash, tags)
                 VALUES (1, 'text', 'hello', 1, '[]')",
                [],
            )
            .unwrap();

        let error = prepare_image_analysis(&connection, 1, false).unwrap_err();

        assert_eq!(error.kind(), ImageAnalysisErrorKind::Validation);
        assert!(error.to_string().contains("只有图片条目"));
    }
}
