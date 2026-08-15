//! Tauri command wrappers around the frontend-independent backup core.

use crate::app_state::AppDataDir;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use tauri::State;
use tiez_core::backup::{BackupError, BackupErrorKind};

pub use tiez_core::backup::BackupInfo;

fn app_error(error: BackupError) -> AppError {
    match error.kind() {
        BackupErrorKind::Database => AppError::Database(error.message().to_owned()),
        BackupErrorKind::Io => AppError::IO(error.message().to_owned()),
        BackupErrorKind::Internal => AppError::Internal(error.message().to_owned()),
        BackupErrorKind::Validation => AppError::Validation(error.message().to_owned()),
    }
}

#[tauri::command]
pub fn create_backup(
    state: State<'_, DbState>,
    app_data: State<'_, AppDataDir>,
    destination: String,
) -> AppResult<BackupInfo> {
    let destination = PathBuf::from(destination.trim());
    let data_dir = app_data
        .0
        .lock()
        .map_err(|error| AppError::Internal(error.to_string()))?
        .clone();
    let connection = state
        .conn
        .lock()
        .map_err(|error| AppError::Database(error.to_string()))?;
    tiez_core::backup::create_backup_from_connection(
        &connection,
        &data_dir,
        &destination,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(app_error)
}

#[tauri::command]
pub fn inspect_backup(path: String) -> AppResult<BackupInfo> {
    tiez_core::backup::inspect_backup(Path::new(path.trim())).map_err(app_error)
}

#[tauri::command]
pub fn schedule_backup_restore(
    app_data: State<'_, AppDataDir>,
    path: String,
) -> AppResult<BackupInfo> {
    let data_dir = app_data
        .0
        .lock()
        .map_err(|error| AppError::Internal(error.to_string()))?
        .clone();
    tiez_core::backup::schedule_backup_restore(&data_dir, Path::new(path.trim())).map_err(app_error)
}

pub fn apply_pending_restore(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = tiez_core::backup::apply_pending_restore(data_dir)?;
    if outcome.quarantined {
        eprintln!(">>> [RESTORE] {}", outcome.message);
    } else if outcome.applied {
        eprintln!(">>> [RESTORE] {}", outcome.message);
    }
    Ok(())
}
