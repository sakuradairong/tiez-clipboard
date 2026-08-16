//! Shared SQLite bootstrap for Tauri and the native WinUI runtime.

use crate::database_migrations::{run_migrations, run_migrations_with_decrypt};
use rusqlite::{params, Connection, Result};
use std::path::Path;

const DEFAULT_SETTINGS: &[(&str, &str)] = &[
    ("app.theme", "mica"),
    ("app.color_mode", "system"),
    ("app.show_app_border", "true"),
    ("app.persistent", "false"),
    ("app.capture_files", "false"),
    ("app.capture_rich_text", "false"),
    ("app.rich_text_snapshot_preview", "true"),
    ("app.deduplicate", "true"),
    ("app.silent_start", "true"),
    ("app.delete_after_paste", "false"),
    ("app.move_to_top_after_paste", "true"),
    ("app.privacy_protection", "true"),
    ("app.privacy_protection_kinds", "phone,idcard,email,secret"),
    ("app.privacy_protection_custom_rules", ""),
    ("app.cleanup_rules", ""),
    ("app.app_cleanup_policies", "[]"),
    ("app.sequential_mode", "false"),
    ("app.sequential_hotkey", "Alt+V"),
    ("app.rich_paste_hotkey", "Alt+Shift+V"),
    ("app.plain_paste_hotkey", ""),
    ("app.search_hotkey", "Alt+F"),
    ("app.relay_send_hotkey", ""),
    ("app.relay_fetch_hotkey", ""),
    ("app.quick_paste_modifier", "disabled"),
    ("app.sound_enabled", "false"),
    ("app.sound_paste_enabled", "true"),
    ("app.hide_tray_icon", "false"),
    ("app.hide_dock_icon", "false"),
    ("app.edge_docking", "false"),
    ("app.arrow_key_selection", "false"),
    ("app.window_pinned", "false"),
    ("app.hotkey", "Alt+C"),
    ("app.autostart", "true"),
    ("app.custom_background", ""),
    ("app.surface_opacity", "50"),
    ("app.notice_v028_shown", "true"),
    ("file_transfer_auto_close", "false"),
    ("file_transfer_auto_copy", "false"),
    ("file_server_enabled", "false"),
    ("file_server_port", "12345"),
    ("mqtt_port", "443"),
    ("mqtt_enabled", "false"),
    ("mqtt_server", ""),
    ("mqtt_username", ""),
    ("mqtt_password", ""),
    ("mqtt_topic", ""),
    ("mqtt_protocol", "wss://"),
    ("mqtt_ssl", "true"),
    ("mqtt_client_id", ""),
    ("mqtt_ws_path", "/mqtt"),
    ("mqtt_notification_enabled", "true"),
    ("cloud_sync_enabled", "false"),
    ("cloud_sync_auto", "true"),
    ("cloud_sync_provider", "webdav"),
    ("cloud_sync_server", ""),
    ("cloud_sync_api_key", ""),
    ("cloud_sync_interval_sec", "120"),
    ("cloud_sync_snapshot_interval_min", "720"),
    ("cloud_sync_cursor", "0"),
    ("cloud_sync_settings_applied_at", "0"),
    ("cloud_sync_webdav_url", ""),
    ("cloud_sync_webdav_username", ""),
    ("cloud_sync_webdav_password", ""),
    ("cloud_sync_webdav_base_path", "tiez-sync"),
    (
        "cloud_sync_content_prefs",
        r#"{"text":true,"image":true,"file_path":true,"emoji":true}"#,
    ),
    ("cloud_sync_webdav_local_seq", "0"),
    ("cloud_sync_webdav_op_cursor_map", "{}"),
    ("cloud_sync_webdav_blob_cache", "{}"),
    ("cloud_sync_webdav_last_snapshot_push_at", "0"),
    ("cloud_sync_webdav_last_snapshot_pull_at", "0"),
    ("cloud_sync_webdav_last_head_rebuild_at", "0"),
    ("ai_enabled", "false"),
    ("ai_target_lang", "zh"),
    ("ai_enable_thinking", "false"),
    ("ai_thinking_budget", "1024"),
    ("app.persistent_limit_enabled", "true"),
    ("app.persistent_limit", "500"),
];

pub fn open_database(path: impl AsRef<Path>) -> Result<Connection> {
    let connection = Connection::open(path)?;
    initialize_connection(&connection)?;
    Ok(connection)
}

pub fn open_database_with_decrypt(
    path: impl AsRef<Path>,
    decrypt_value: impl Fn(&str) -> Option<String>,
) -> Result<Connection> {
    let connection = Connection::open(path)?;
    initialize_connection_with_decrypt(&connection, decrypt_value)?;
    Ok(connection)
}

pub fn initialize_connection(connection: &Connection) -> Result<()> {
    configure_connection(connection)?;
    run_migrations(connection)?;
    seed_defaults(connection)
}

pub fn initialize_connection_with_decrypt(
    connection: &Connection,
    decrypt_value: impl Fn(&str) -> Option<String>,
) -> Result<()> {
    configure_connection(connection)?;
    run_migrations_with_decrypt(connection, decrypt_value)?;
    seed_defaults(connection)
}

fn configure_connection(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA auto_vacuum = FULL;",
    )
}

pub fn seed_defaults(connection: &Connection) -> Result<()> {
    for (key, value) in DEFAULT_SETTINGS {
        let _ = connection.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        );
    }

    for (key, inherited_value) in [
        ("mqtt_server", "tiez.name666.top"),
        ("mqtt_username", "tiezpublic"),
        ("mqtt_password", "tiezmessage"),
    ] {
        let _ = connection.execute(
            "UPDATE settings
             SET value = ''
             WHERE key = ?1
               AND value = ?2
               AND COALESCE(
                    (SELECT value FROM settings WHERE key = 'mqtt_enabled'),
                    'false'
               ) = 'false'",
            params![key, inherited_value],
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_creates_current_schema_and_all_defaults() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_connection(&connection).unwrap();

        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        let setting_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        let privacy: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'app.privacy_protection'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(version, 15);
        assert_eq!(setting_count, DEFAULT_SETTINGS.len() as i64);
        assert_eq!(privacy, "true");
    }

    #[test]
    fn bootstrap_is_idempotent_and_preserves_user_settings() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_connection(&connection).unwrap();
        connection
            .execute(
                "UPDATE settings SET value = 'dark' WHERE key = 'app.color_mode'",
                [],
            )
            .unwrap();

        initialize_connection(&connection).unwrap();

        let color_mode: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'app.color_mode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(color_mode, "dark");
    }

    #[test]
    fn inherited_public_mqtt_credentials_are_cleared_while_disabled() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_connection(&connection).unwrap();
        connection
            .execute_batch(
                "UPDATE settings SET value = 'tiez.name666.top' WHERE key = 'mqtt_server';
                 UPDATE settings SET value = 'tiezpublic' WHERE key = 'mqtt_username';
                 UPDATE settings SET value = 'tiezmessage' WHERE key = 'mqtt_password';",
            )
            .unwrap();

        seed_defaults(&connection).unwrap();

        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM settings
                 WHERE key IN ('mqtt_server', 'mqtt_username', 'mqtt_password')
                   AND value <> ''",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
