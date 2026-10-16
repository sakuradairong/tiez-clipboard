use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Serialize)]
pub struct CoreError {
    pub code: &'static str,
    pub message: String,
}

impl CoreError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new("database", value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SettingsSnapshot {
    pub revision: u64,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct SettingsChange {
    pub changed: bool,
    pub snapshot: SettingsSnapshot,
}

pub struct SettingsService {
    connection: Connection,
}

fn defaults() -> BTreeMap<String, String> {
    [
        ("app.language", "zh"),
        ("app.color_mode", "system"),
        ("app.deduplicate", "true"),
        ("app.show_search_box", "true"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn validate(key: &str, value: &str) -> CoreResult<()> {
    let valid = match key {
        "app.language" => matches!(value, "zh" | "en" | "tw"),
        "app.color_mode" => matches!(value, "system" | "light" | "dark"),
        "app.deduplicate" | "app.show_search_box" => matches!(value, "true" | "false"),
        _ => {
            return Err(CoreError::new(
                "unknown_setting",
                format!("Unsupported setting: {key}"),
            ))
        }
    };
    if valid {
        Ok(())
    } else {
        Err(CoreError::new(
            "invalid_value",
            format!("Invalid value for {key}"),
        ))
    }
}

fn read_snapshot(connection: &Connection) -> CoreResult<SettingsSnapshot> {
    let revision: i64 = connection.query_row(
        "SELECT revision FROM native_settings_revision WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let revision = u64::try_from(revision)
        .map_err(|_| CoreError::new("database", "Invalid settings revision"))?;
    let mut values = defaults();
    for (key, value) in &mut values {
        if let Some(stored) = connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
        {
            validate(key, &stored)?;
            *value = stored;
        }
    }
    Ok(SettingsSnapshot { revision, values })
}

impl SettingsService {
    /// Opens only the explicit prototype database; never discovers production data.
    pub fn open(path: &Path) -> CoreResult<Self> {
        if !path.is_absolute() {
            return Err(CoreError::new(
                "invalid_path",
                "An absolute prototype database path is required",
            ));
        }
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(connection: Connection) -> CoreResult<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS native_settings_revision (
                 id INTEGER PRIMARY KEY CHECK (id = 1), revision INTEGER NOT NULL CHECK (revision >= 0));
             INSERT OR IGNORE INTO native_settings_revision VALUES (1, 0);")?;
        let service = Self { connection };
        service.snapshot()?;
        Ok(service)
    }

    pub fn snapshot(&self) -> CoreResult<SettingsSnapshot> {
        read_snapshot(&self.connection)
    }

    pub fn update(
        &mut self,
        expected_revision: u64,
        patch: BTreeMap<String, String>,
    ) -> CoreResult<SettingsChange> {
        for (key, value) in &patch {
            validate(key, value)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_snapshot(&transaction)?;
        if current.revision != expected_revision {
            return Err(CoreError::new(
                "revision_conflict",
                "Settings changed; reload before saving",
            ));
        }
        let changed = patch
            .iter()
            .any(|(key, value)| current.values.get(key) != Some(value));
        if !changed {
            transaction.rollback()?;
            return Ok(SettingsChange {
                changed: false,
                snapshot: current,
            });
        }
        let next_revision = current
            .revision
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or_else(|| CoreError::new("revision_overflow", "Settings revision exhausted"))?;
        for (key, value) in patch {
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        transaction.execute(
            "UPDATE native_settings_revision SET revision = ?1 WHERE id = 1",
            [next_revision as i64],
        )?;
        let snapshot = read_snapshot(&transaction)?;
        transaction.commit()?;
        Ok(SettingsChange {
            changed: true,
            snapshot,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> SettingsService {
        SettingsService::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn patch(key: &str, value: &str) -> BTreeMap<String, String> {
        [(key.into(), value.into())].into()
    }

    #[test]
    fn reads_and_noops_do_not_write_or_advance_revision() {
        let mut service = service();
        let initial = service.snapshot().unwrap();
        let writes: i64 = service
            .connection
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert!(
            !service
                .update(0, patch("app.language", "zh"))
                .unwrap()
                .changed
        );
        assert!(!service.update(0, BTreeMap::new()).unwrap().changed);
        assert_eq!(service.snapshot().unwrap(), initial);
        let after: i64 = service
            .connection
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, writes);
    }

    #[test]
    fn validation_and_stale_edits_preserve_accepted_state() {
        let mut service = service();
        for (key, value) in [
            ("mqtt_password", "secret"),
            ("app.language", "invalid"),
            ("app.deduplicate", "1"),
        ] {
            assert!(service.update(0, patch(key, value)).is_err());
        }
        assert_eq!(service.snapshot().unwrap().revision, 0);
        let accepted = service
            .update(0, patch("app.language", "en"))
            .unwrap()
            .snapshot;
        assert_eq!(
            service
                .update(0, patch("app.color_mode", "dark"))
                .unwrap_err()
                .code,
            "revision_conflict"
        );
        assert_eq!(service.snapshot().unwrap(), accepted);
    }

    #[test]
    fn failed_batch_rolls_back_values_and_revision() {
        let mut service = service();
        service
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_language BEFORE INSERT ON settings
            WHEN NEW.key = 'app.language' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
            )
            .unwrap();
        let original = service.snapshot().unwrap();
        let edits = [
            ("app.color_mode".into(), "dark".into()),
            ("app.language".into(), "en".into()),
        ]
        .into();
        assert!(service.update(0, edits).is_err());
        assert_eq!(service.snapshot().unwrap(), original);
        let count: i64 = service
            .connection
            .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn reopening_preserves_values_revision_and_unrelated_rows() {
        let path = std::env::temp_dir().join(format!(
            "TieZ-native-settings-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let mut service = SettingsService::open(&path).unwrap();
            service
                .connection
                .execute("INSERT INTO settings VALUES ('unrelated', '保留')", [])
                .unwrap();
            service.update(0, patch("app.language", "tw")).unwrap();
        }
        {
            let service = SettingsService::open(&path).unwrap();
            let snapshot = service.snapshot().unwrap();
            assert_eq!(snapshot.revision, 1);
            assert_eq!(snapshot.values["app.language"], "tw");
            assert!(!snapshot.values.contains_key("unrelated"));
            let value: String = service
                .connection
                .query_row(
                    "SELECT value FROM settings WHERE key = 'unrelated'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(value, "保留");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_relative_database_path() {
        assert!(SettingsService::open(Path::new("clipboard.db")).is_err());
    }
}
