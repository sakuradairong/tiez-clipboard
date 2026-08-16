use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use rusqlite::params;
use tauri::{AppHandle, Manager};
use tiez_core::clipboard_relay::{RelayConfig, RelayError, RelayErrorKind, RelayReceiptStore};
pub use tiez_core::clipboard_relay::{RelayFetchResult, RelaySendResult};

fn map_error(error: RelayError) -> AppError {
    match error.kind() {
        RelayErrorKind::Validation => AppError::Validation(error.to_string()),
        RelayErrorKind::Network => AppError::Network(error.to_string()),
        RelayErrorKind::Storage => AppError::Database(error.to_string()),
        RelayErrorKind::Encryption => AppError::Encryption(error.to_string()),
        RelayErrorKind::Internal => AppError::Internal(error.to_string()),
    }
}

fn relay_error(error: impl std::fmt::Display) -> RelayError {
    RelayError::new(RelayErrorKind::Internal, error.to_string())
}

struct TauriRelayReceiptStore<'a>(&'a DbState);

impl RelayReceiptStore for TauriRelayReceiptStore<'_> {
    fn prune(&self, now: i64) -> Result<(), RelayError> {
        self.0
            .conn
            .lock()
            .map_err(relay_error)?
            .execute(
                "DELETE FROM clipboard_relay_receipts WHERE expires_at <= ?1",
                params![now],
            )
            .map(|_| ())
            .map_err(relay_error)
    }

    fn state(&self, message_id: &str) -> Result<Option<String>, RelayError> {
        let connection = self.0.conn.lock().map_err(relay_error)?;
        let result = connection.query_row(
            "SELECT state FROM clipboard_relay_receipts WHERE message_id = ?1",
            params![message_id],
            |row| row.get(0),
        );
        match result {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(relay_error(error)),
        }
    }

    fn reserve(&self, message_id: &str, expires_at: i64, now: i64) -> Result<bool, RelayError> {
        self.0
            .conn
            .lock()
            .map_err(relay_error)?
            .execute(
                "INSERT OR IGNORE INTO clipboard_relay_receipts
                    (message_id, expires_at, state, ack_json, updated_at)
                 VALUES (?1, ?2, 'reserved', '', ?3)",
                params![message_id, expires_at, now],
            )
            .map(|inserted| inserted == 1)
            .map_err(relay_error)
    }

    fn remove_reserved(&self, message_id: &str) -> Result<(), RelayError> {
        self.0
            .conn
            .lock()
            .map_err(relay_error)?
            .execute(
                "DELETE FROM clipboard_relay_receipts
                 WHERE message_id = ?1 AND state = 'reserved'",
                params![message_id],
            )
            .map(|_| ())
            .map_err(relay_error)
    }

    fn persist_pending_ack(
        &self,
        message_id: &str,
        ack_json: &str,
        now: i64,
    ) -> Result<(), RelayError> {
        self.0
            .conn
            .lock()
            .map_err(relay_error)?
            .execute(
                "UPDATE clipboard_relay_receipts
                 SET state = 'copied_pending_ack', ack_json = ?2, updated_at = ?3
                 WHERE message_id = ?1",
                params![message_id, ack_json, now],
            )
            .map(|_| ())
            .map_err(relay_error)
    }

    fn mark_acked(&self, message_id: &str, now: i64) -> Result<(), RelayError> {
        self.0
            .conn
            .lock()
            .map_err(relay_error)?
            .execute(
                "UPDATE clipboard_relay_receipts SET state = 'acked', updated_at = ?2
                 WHERE message_id = ?1",
                params![message_id, now],
            )
            .map(|_| ())
            .map_err(relay_error)
    }

    fn pending_ack_json(&self, message_id: &str) -> Result<Option<String>, RelayError> {
        let connection = self.0.conn.lock().map_err(relay_error)?;
        let result: Result<(String, String), rusqlite::Error> = connection.query_row(
            "SELECT state, ack_json FROM clipboard_relay_receipts WHERE message_id = ?1",
            params![message_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((state, ack_json)) if state == "copied_pending_ack" => Ok(Some(ack_json)),
            Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(relay_error(error)),
        }
    }
}

fn get_config(app: &AppHandle) -> AppResult<RelayConfig> {
    let app_data_dir = app
        .try_state::<crate::app_state::AppDataDir>()
        .and_then(|state| state.0.lock().ok().map(|value| value.clone()));
    crate::services::relay_key::ensure_runtime_allowed(app_data_dir.as_deref())?;
    let db = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("database state unavailable".to_string()))?;
    let setting =
        |key: &str| -> String { db.settings_repo.get(key).ok().flatten().unwrap_or_default() };

    let configured_url = setting("cloud_sync_webdav_url");
    let webdav_url = if configured_url.trim().is_empty() {
        setting("cloud_sync_server")
    } else {
        configured_url
    };
    if webdav_url.trim().is_empty() {
        return Err(AppError::Validation(
            "请先在云同步设置中配置 WebDAV 地址".to_string(),
        ));
    }

    let configured_password = setting("cloud_sync_webdav_password");
    let webdav_password = if configured_password.trim().is_empty() {
        setting("cloud_sync_api_key")
    } else {
        configured_password
    };
    let stored_device_id = setting("app.anon_id");
    let device_id = crate::app::system::normalize_anon_id(&stored_device_id).unwrap_or_else(|| {
        crate::app::system::build_anon_id(&crate::app::system::get_machine_id())
    });
    if stored_device_id.trim() != device_id {
        db.settings_repo
            .set("app.anon_id", &device_id)
            .map_err(AppError::from)?;
    }
    let shared_key = crate::services::relay_key::load()?
        .ok_or_else(|| AppError::Validation("请先配置剪贴板接力共享密钥".to_string()))?;

    RelayConfig::new(
        webdav_url,
        setting("cloud_sync_webdav_username"),
        webdav_password,
        setting("cloud_sync_webdav_base_path"),
        device_id,
        shared_key,
    )
    .map_err(map_error)
}

pub async fn send_current_clipboard(app: &AppHandle) -> AppResult<RelaySendResult> {
    let text = crate::services::clipboard_ops::read_plain_text_exact()?;
    let config = get_config(app)?;
    tiez_core::clipboard_relay::send_text(&config, &text)
        .await
        .map_err(map_error)
}

pub async fn fetch_latest_to_clipboard(app: &AppHandle) -> AppResult<RelayFetchResult> {
    let config = get_config(app)?;
    let db = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("database state unavailable".to_string()))?;
    let store = TauriRelayReceiptStore(&db);
    tiez_core::clipboard_relay::fetch_latest_to_clipboard(&config, &store, |content| async move {
        crate::services::clipboard_ops::set_plain_text_from_app(&content)
            .await
            .map_err(relay_error)
    })
    .await
    .map_err(map_error)
}
