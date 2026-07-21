use crate::database::{has_sensitive_tag, DbState};
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use base64::Engine;
use image::GenericImageView;
use rqrr::PreparedImage;
use rusqlite::{params, OptionalExtension};
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
    let persisted = !sensitive;

    if persisted {
        let conn = state
            .conn
            .lock()
            .map_err(|err| AppError::Database(err.to_string()))?;
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
                text,
                serde_json::to_string(&qr_codes).unwrap_or_else(|_| "[]".to_string()),
                language,
                analyzed_at
            ],
        )?;
    }

    Ok(ImageAnalysisResult {
        text,
        qr_codes,
        language,
        analyzed_at,
        cached: false,
        persisted,
        ocr_available: cfg!(target_os = "windows"),
        ocr_error,
    })
}
