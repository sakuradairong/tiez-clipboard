use crate::error::{AppError, AppResult};
use std::path::Path;

fn map_error(error: tiez_core::clipboard_relay::RelayError) -> AppError {
    use tiez_core::clipboard_relay::RelayErrorKind;
    match error.kind() {
        RelayErrorKind::Validation => AppError::Validation(error.to_string()),
        RelayErrorKind::Network => AppError::Network(error.to_string()),
        RelayErrorKind::Storage => AppError::Database(error.to_string()),
        RelayErrorKind::Encryption => AppError::Encryption(error.to_string()),
        RelayErrorKind::Internal => AppError::Internal(error.to_string()),
    }
}

pub fn ensure_runtime_allowed(data_dir: Option<&Path>) -> AppResult<()> {
    tiez_core::relay_key::ensure_runtime_allowed(data_dir, cfg!(feature = "portable"))
        .map_err(map_error)
}

pub fn load() -> AppResult<Option<[u8; 32]>> {
    tiez_core::relay_key::load().map_err(map_error)
}

pub fn is_configured() -> AppResult<bool> {
    tiez_core::relay_key::is_configured().map_err(map_error)
}

pub fn store(raw: &str) -> AppResult<()> {
    if cfg!(feature = "portable") {
        ensure_runtime_allowed(None)?;
    }
    tiez_core::relay_key::store(raw).map_err(map_error)
}

pub fn generate() -> AppResult<String> {
    if cfg!(feature = "portable") {
        ensure_runtime_allowed(None)?;
    }
    tiez_core::relay_key::generate().map_err(map_error)
}

pub fn clear() -> AppResult<()> {
    if cfg!(feature = "portable") {
        ensure_runtime_allowed(None)?;
    }
    tiez_core::relay_key::clear().map_err(map_error)
}

pub fn migrate_legacy(raw: &str) -> AppResult<()> {
    if cfg!(feature = "portable") {
        ensure_runtime_allowed(None)?;
    }
    tiez_core::relay_key::migrate_legacy(raw).map_err(map_error)
}

pub fn validate_format(raw: &str) -> AppResult<()> {
    if cfg!(feature = "portable") {
        ensure_runtime_allowed(None)?;
    }
    tiez_core::relay_key::validate_format(raw).map_err(map_error)
}
