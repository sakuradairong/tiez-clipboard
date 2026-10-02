use crate::database::{has_sensitive_tag, DbState};
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use base64::Engine;
use image::GenericImageView;
use rqrr::PreparedImage;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const OCR_MAX_IMAGE_DIMENSION: u32 = 2600;

#[derive(Debug, Clone, Serialize)]
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn temporary_png_path() -> PathBuf {
    std::env::temp_dir().join(format!("tiez-ocr-{}.png", uuid::Uuid::new_v4()))
}

fn image_file_from_content(content: &str) -> AppResult<TemporaryImage> {
    if !content.starts_with("data:image/") {
        let path = PathBuf::from(content);
        if !path.is_file() {
            return Err(AppError::Validation("图片文件不存在或已被删除".to_string()));
        }
        return Ok(TemporaryImage::borrowed(path));
    }

    let encoded = content
        .split_once(',')
        .map(|(_, payload)| payload)
        .ok_or_else(|| AppError::Validation("图片数据格式无效".to_string()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| AppError::Validation(format!("图片数据解码失败: {err}")))?;
    let image = image::load_from_memory(&bytes)?;
    let path = temporary_png_path();
    image.save(&path)?;
    Ok(TemporaryImage::owned(path))
}

fn prepare_ocr_image(path: &Path) -> AppResult<TemporaryImage> {
    let image = image::open(path)?;
    let (width, height) = image.dimensions();
    if width <= OCR_MAX_IMAGE_DIMENSION && height <= OCR_MAX_IMAGE_DIMENSION {
        return Ok(TemporaryImage::borrowed(path.to_path_buf()));
    }

    let resized = image.thumbnail(OCR_MAX_IMAGE_DIMENSION, OCR_MAX_IMAGE_DIMENSION);
    let output = temporary_png_path();
    resized.save(&output)?;
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

    // The OCR work runs on Tokio's blocking pool, so initialize WinRT for this
    // worker thread. If another apartment model is already active, the APIs can
    // still be used and this call simply must not be paired with RoUninitialize.
    let _apartment = WinRtApartment(unsafe { RoInitialize(RO_INIT_MULTITHREADED).is_ok() });

    let canonical = std::fs::canonicalize(path).map_err(|err| err.to_string())?;
    let normalized = canonical.to_string_lossy().replace("\\\\?\\", "");
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(normalized))
        .map_err(|err| err.to_string())?
        .get()
        .map_err(|err| err.to_string())?;
    let stream = file
        .OpenAsync(FileAccessMode::Read)
        .map_err(|err| err.to_string())?
        .get()
        .map_err(|err| err.to_string())?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|err| err.to_string())?
        .get()
        .map_err(|err| err.to_string())?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|err| err.to_string())?
        .get()
        .map_err(|err| err.to_string())?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|err| err.to_string())?;
    let language = engine
        .RecognizerLanguage()
        .and_then(|value| value.LanguageTag())
        .ok()
        .map(|value| value.to_string_lossy());
    let text = engine
        .RecognizeAsync(&bitmap)
        .map_err(|err| err.to_string())?
        .get()
        .and_then(|result| result.Text())
        .map_err(|err| err.to_string())?
        .to_string_lossy();
    Ok((text.trim().to_string(), language))
}

#[cfg(not(target_os = "windows"))]
fn recognize_text(_path: &Path) -> Result<(String, Option<String>), String> {
    Err("当前平台暂不支持系统 OCR".to_string())
}

fn read_cached_analysis(
    state: &DbState,
    entry_id: i64,
    content_hash: i64,
) -> AppResult<Option<ImageAnalysisResult>> {
    let conn = state
        .conn
        .lock()
        .map_err(|err| AppError::Database(err.to_string()))?;
    let row = conn
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
        .optional()?;
    Ok(row)
}

#[tauri::command]
pub fn get_image_analysis(
    state: State<'_, DbState>,
    id: i64,
) -> AppResult<Option<ImageAnalysisResult>> {
    let content_hash = {
        let conn = state
            .conn
            .lock()
            .map_err(|err| AppError::Database(err.to_string()))?;
        conn.query_row(
            "SELECT content_hash FROM clipboard_history WHERE id = ?1 AND content_type = 'image'",
            params![id],
            |row| row.get(0),
        )
        .optional()?
    };

    match content_hash {
        Some(hash) => read_cached_analysis(&state, id, hash),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn analyze_image_entry(
    state: State<'_, DbState>,
    id: i64,
    force: Option<bool>,
) -> AppResult<ImageAnalysisResult> {
    let entry = state
        .repo
        .get_entry_by_id(id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::Validation("找不到图片条目".to_string()))?;
    if entry.content_type != "image" {
        return Err(AppError::Validation("只有图片条目可以执行 OCR".to_string()));
    }

    let content_hash = {
        let conn = state
            .conn
            .lock()
            .map_err(|err| AppError::Database(err.to_string()))?;
        conn.query_row(
            "SELECT content_hash FROM clipboard_history WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?
    };

    let sensitive = has_sensitive_tag(&entry.tags);
    if !force.unwrap_or(false) && !sensitive {
        if let Some(cached) = read_cached_analysis(&state, id, content_hash)? {
            return Ok(cached);
        }
    }

    let source = image_file_from_content(&entry.content)?;
    let qr_path = source.path.clone();
    let ocr_image = prepare_ocr_image(&source.path)?;
    let ocr_path = ocr_image.path.clone();
    let (qr_codes, ocr) =
        tokio::task::spawn_blocking(move || (decode_qr_codes(&qr_path), recognize_text(&ocr_path)))
            .await
            .map_err(|err| AppError::Internal(format!("图片识别任务失败: {err}")))?;

    let analyzed_at = now_ms();
    let (text, language, ocr_error) = match ocr {
        Ok((text, language)) => (text, language, None),
        Err(error) => (String::new(), None, Some(error)),
    };
    let mut result = ImageAnalysisResult {
        text,
        qr_codes,
        language,
        analyzed_at,
        cached: false,
        persisted: false,
        ocr_available: cfg!(target_os = "windows"),
        ocr_error,
    };
    if !sensitive {
        let conn = state
            .conn
            .lock()
            .map_err(|err| AppError::Database(err.to_string()))?;
        result.persisted = persist_current_image_analysis(&conn, id, content_hash, &result)?;
    }
    Ok(result)
}

fn persist_current_image_analysis(
    conn: &Connection,
    id: i64,
    content_hash: i64,
    result: &ImageAnalysisResult,
) -> AppResult<bool> {
    // The caller holds the shared connection lock throughout this check and write.
    // A tag edit, deletion or replacement during OCR must not resurrect its index.
    let current: Option<(String, i64, String)> = conn
        .query_row(
            "SELECT content_type, content_hash, tags FROM clipboard_history WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((content_type, current_hash, tags)) = current else {
        return Ok(false);
    };
    let tags: Vec<String> = serde_json::from_str(&tags)
        .map_err(|err| AppError::Validation(format!("图片标签无效: {err}")))?;
    if content_type != "image" || current_hash != content_hash || has_sensitive_tag(&tags) {
        return Ok(false);
    }
    conn.execute(
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
            id,
            content_hash,
            result.text,
            serde_json::to_string(&result.qr_codes).unwrap_or_else(|_| "[]".to_string()),
            result.language,
            result.analyzed_at
        ],
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{persist_current_image_analysis, ImageAnalysisResult};
    use rusqlite::Connection;

    fn analysis_fixture() -> (Connection, ImageAnalysisResult) {
        let conn = Connection::open_in_memory().expect("open analysis fixture");
        crate::infrastructure::repository::migrations::run_migrations(&conn)
            .expect("migrate analysis fixture");
        conn.execute_batch(
            "INSERT INTO clipboard_history
                (id, content_type, content, content_hash, source_app, timestamp, preview)
             VALUES (1, 'image', 'image.png', 7, 'test', 1, 'image');",
        )
        .expect("insert ordinary image");
        let result = ImageAnalysisResult {
            text: "secret OCR text".into(),
            qr_codes: vec!["secret QR".into()],
            language: None,
            analyzed_at: 10,
            cached: false,
            persisted: false,
            ocr_available: true,
            ocr_error: None,
        };
        (conn, result)
    }

    #[test]
    fn finished_ocr_does_not_recreate_index_after_sensitive_tag_change() {
        let (conn, result) = analysis_fixture();
        // OCR began with ordinary tags, then the tag command removed its cache.
        conn.execute_batch(
            "UPDATE clipboard_history SET tags = '[\"Password\"]' WHERE id = 1;
             DELETE FROM clipboard_image_analysis WHERE entry_id = 1;",
        )
        .expect("make image sensitive during OCR");
        assert!(!persist_current_image_analysis(&conn, 1, 7, &result).expect("finish OCR"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM clipboard_image_analysis", [], |row| {
                row.get(0)
            })
            .expect("count protected OCR");
        assert_eq!(count, 0);
    }

    #[test]
    fn finished_ocr_does_not_index_deleted_or_replaced_images() {
        for mutation in [
            "DELETE FROM clipboard_history WHERE id = 1",
            "UPDATE clipboard_history SET content_hash = 8 WHERE id = 1",
            "UPDATE clipboard_history SET content_type = 'text' WHERE id = 1",
        ] {
            let (conn, result) = analysis_fixture();
            conn.execute(mutation, []).expect("mutate image during OCR");
            assert!(
                !persist_current_image_analysis(&conn, 1, 7, &result).expect("discard stale OCR")
            );
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM clipboard_image_analysis", [], |row| {
                    row.get(0)
                })
                .expect("count stale OCR");
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn finished_ocr_persists_current_ordinary_image() {
        let (conn, result) = analysis_fixture();
        assert!(persist_current_image_analysis(&conn, 1, 7, &result).expect("persist ordinary OCR"));
        let row: (String, String) = conn
            .query_row(
                "SELECT ocr_text, qr_codes FROM clipboard_image_analysis WHERE entry_id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read ordinary OCR");
        assert_eq!(row, ("secret OCR text".into(), "[\"secret QR\"]".into()));
    }
}
