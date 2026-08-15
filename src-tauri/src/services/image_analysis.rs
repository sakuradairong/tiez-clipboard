use crate::database::DbState;
use crate::error::{AppError, AppResult};
use tauri::State;
use tiez_core::image_analysis::{
    analyze_prepared_image, finish_image_analysis, get_image_analysis as get_shared_analysis,
    prepare_image_analysis, ImageAnalysisError, ImageAnalysisErrorKind, PreparedImageAnalysis,
};

pub use tiez_core::image_analysis::ImageAnalysisResult;

fn map_analysis_error(error: ImageAnalysisError) -> AppError {
    match error.kind() {
        ImageAnalysisErrorKind::Storage => AppError::Database(error.to_string()),
        ImageAnalysisErrorKind::NotFound | ImageAnalysisErrorKind::Validation => {
            AppError::Validation(error.to_string())
        }
        ImageAnalysisErrorKind::Io => AppError::IO(error.to_string()),
    }
}

#[tauri::command]
pub fn get_image_analysis(
    state: State<'_, DbState>,
    id: i64,
) -> AppResult<Option<ImageAnalysisResult>> {
    let connection = state
        .conn
        .lock()
        .map_err(|error| AppError::Database(error.to_string()))?;
    get_shared_analysis(&connection, id).map_err(map_analysis_error)
}

#[tauri::command]
pub async fn analyze_image_entry(
    state: State<'_, DbState>,
    id: i64,
    force: Option<bool>,
) -> AppResult<ImageAnalysisResult> {
    let prepared = {
        let connection = state
            .conn
            .lock()
            .map_err(|error| AppError::Database(error.to_string()))?;
        prepare_image_analysis(&connection, id, force.unwrap_or(false))
            .map_err(map_analysis_error)?
    };

    let work = match prepared {
        PreparedImageAnalysis::Cached(result) => return Ok(result),
        PreparedImageAnalysis::Pending(work) => work,
    };
    let (work, result) = tokio::task::spawn_blocking(move || {
        let result = analyze_prepared_image(&work)?;
        Ok::<_, ImageAnalysisError>((work, result))
    })
    .await
    .map_err(|error| AppError::Internal(format!("图片识别任务失败: {error}")))?
    .map_err(map_analysis_error)?;

    let connection = state
        .conn
        .lock()
        .map_err(|error| AppError::Database(error.to_string()))?;
    finish_image_analysis(&connection, &work, result, true).map_err(map_analysis_error)
}
