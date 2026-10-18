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
        ("app.hotkey", "Win+V"),
        ("app.color_mode", "system"),
        ("app.deduplicate", "true"),
        ("app.show_search_box", "true"),
        ("app.compact_mode", "false"),
        ("app.history_view_mode", "list"),
        ("app.tag_history_view_mode", "grid"),
        ("app.persistent", "false"),
        ("app.capture_files", "false"),
        ("app.capture_rich_text", "false"),
        ("app.privacy_protection", "true"),
        ("app.privacy_protection_kinds", "phone,idcard,email,secret"),
        ("app.privacy_protection_custom_rules", ""),
        ("app.cleanup_rules", ""),
        ("app.app_cleanup_policies", "[]"),
        ("app.persistent_limit_enabled", "true"),
        ("app.persistent_limit", "500"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn valid_cleanup(value: &str) -> bool {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .all(|line| {
            line.split_once("=>").is_some_and(|(pattern, _)| {
                !pattern.trim().is_empty() && regex::Regex::new(pattern.trim()).is_ok()
            })
        })
}

fn validate(key: &str, value: &str) -> CoreResult<()> {
    let valid = match key {
        "app.language" => matches!(value, "zh" | "en" | "tw"),
        "app.hotkey" => tiez_native_platform::hotkey::Shortcut::parse(value).is_ok(),
        "app.color_mode" => matches!(value, "system" | "light" | "dark"),
        "app.history_view_mode" | "app.tag_history_view_mode" => matches!(value, "list" | "grid"),
        "app.deduplicate"
        | "app.show_search_box"
        | "app.compact_mode"
        | "app.persistent"
        | "app.capture_files"
        | "app.capture_rich_text"
        | "app.privacy_protection"
        | "app.persistent_limit_enabled" => matches!(value, "true" | "false"),
        "app.persistent_limit" => value.parse::<u32>().is_ok_and(|limit| limit <= 1_000_000),
        "app.privacy_protection_kinds" => value.len() <= 4096,
        "app.privacy_protection_custom_rules" => value
            .lines()
            .filter(|line| !line.trim().is_empty())
            .all(|line| regex::Regex::new(line.trim()).is_ok()),
        "app.cleanup_rules" => valid_cleanup(value),
        "app.app_cleanup_policies" => serde_json::from_str::<
            Vec<tiez_content::utils::AppCleanupPolicy>,
        >(value)
        .is_ok_and(|policies| {
            policies.iter().all(|policy| {
                (policy.action.eq_ignore_ascii_case("ignore")
                    || policy.action.eq_ignore_ascii_case("clean"))
                    && valid_cleanup(&policy.cleanup_rules)
            })
        }),
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
        self.update_with(expected_revision, patch, |_, _| Ok(()))
            .map(|(change, _)| change)
    }

    /// A prepared effect is dropped (rolled back) if the SQLite commit fails.
    /// Callers finalize it only after receiving an accepted durable snapshot.
    pub(crate) fn update_with<P>(
        &mut self,
        expected_revision: u64,
        patch: BTreeMap<String, String>,
        prepare: impl FnOnce(&SettingsSnapshot, &SettingsSnapshot) -> CoreResult<P>,
    ) -> CoreResult<(SettingsChange, Option<P>)> {
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
            return Ok((
                SettingsChange {
                    changed: false,
                    snapshot: current,
                },
                None,
            ));
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
        let prepared = prepare(&current, &snapshot)?;
        transaction.commit()?;
        Ok((
            SettingsChange {
                changed: true,
                snapshot,
            },
            Some(prepared),
        ))
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
    fn capture_policy_validation_is_atomic_and_read_only() {
        let mut service = service();
        let original = service.snapshot().unwrap();
        assert_eq!(original.values["app.persistent"], "false");
        assert_eq!(original.values["app.capture_rich_text"], "false");
        assert_eq!(original.values["app.privacy_protection"], "true");
        for (key, value) in [
            ("app.persistent", "yes"),
            ("app.capture_files", "1"),
            ("app.persistent_limit", "-1"),
            ("app.persistent_limit", "1000001"),
            ("app.privacy_protection_custom_rules", "["),
            ("app.cleanup_rules", "[ => bad"),
            ("app.app_cleanup_policies", "{}"),
            ("app.app_cleanup_policies", r#"[{"action":"delete"}]"#),
        ] {
            let mut edits = patch("app.persistent", "true");
            edits.insert(key.into(), value.into());
            assert!(service.update(0, edits).is_err(), "accepted invalid {key}");
            assert_eq!(service.snapshot().unwrap(), original);
        }
        let accepted = service
            .update(
                0,
                [
                    ("app.persistent".into(), "true".into()),
                    ("app.cleanup_rules".into(), "^prefix => replacement".into()),
                    (
                        "app.privacy_protection_custom_rules".into(),
                        "secret[0-9]+".into(),
                    ),
                    ("app.persistent_limit".into(), "0".into()),
                ]
                .into(),
            )
            .unwrap();
        assert_eq!(accepted.snapshot.revision, 1);
        assert_eq!(accepted.snapshot.values["app.persistent_limit"], "0");
    }

    #[test]
    fn rejects_relative_database_path() {
        assert!(SettingsService::open(Path::new("clipboard.db")).is_err());
    }
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn patch() -> BTreeMap<String, String> {
        [("app.hotkey".into(), "Ctrl+Alt+F24".into())].into()
    }
    #[test]
    fn preparation_failure_rolls_back_rows_and_revision() {
        let mut service =
            SettingsService::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let original = service.snapshot().unwrap();
        let result: CoreResult<(SettingsChange, Option<()>)> =
            service.update_with(0, patch(), |_, _| {
                Err(CoreError::new("hotkey_conflict", "fixture"))
            });
        assert_eq!(result.unwrap_err().code, "hotkey_conflict");
        assert_eq!(service.snapshot().unwrap(), original);
    }
    #[test]
    fn noops_and_stale_requests_never_prepare_runtime_effects() {
        let mut service =
            SettingsService::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let (_, prepared) = service
            .update_with(0, BTreeMap::new(), |_, _| -> CoreResult<()> {
                panic!("No-op prepared runtime");
            })
            .unwrap();
        assert!(prepared.is_none());
        assert!(service
            .update_with(1, patch(), |_, _| -> CoreResult<()> {
                panic!("Stale save prepared runtime");
            })
            .is_err());
    }
    #[test]
    fn failed_commit_drops_prepared_effect_before_returning() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE parent(id INTEGER PRIMARY KEY);
            CREATE TABLE effect_child(id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
        let mut service = SettingsService::from_connection(connection).unwrap();
        service
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_effect_commit AFTER INSERT ON settings
            WHEN NEW.key='app.hotkey' BEGIN INSERT INTO effect_child VALUES(999); END;",
            )
            .unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let result = service.update_with(0, patch(), |_, _| Ok(Guard(drops.clone())));
        assert!(result.is_err());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(service.snapshot().unwrap().revision, 0);
        assert_eq!(service.snapshot().unwrap().values["app.hotkey"], "Win+V");
    }
    #[test]
    fn compact_mode_defaults_without_writes_and_requires_boolean_values() {
        let mut s =
            SettingsService::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let compact_patch = |value: &str| -> BTreeMap<String, String> {
            [("app.compact_mode".into(), value.into())].into()
        };
        let before: i64 = s
            .connection
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(s.snapshot().unwrap().values["app.compact_mode"], "false");
        assert_eq!(
            s.connection
                .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert!(s.update(0, compact_patch("1")).is_err());
        assert_eq!(s.snapshot().unwrap().revision, 0);
        assert!(s.update(0, compact_patch("true")).unwrap().changed);
        assert!(!s.update(1, compact_patch("true")).unwrap().changed);
    }
    #[test]
    fn history_layout_defaults_are_readonly_and_scopes_validate_independently() {
        let mut s =
            SettingsService::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let before: i64 = s
            .connection
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let snapshot = s.snapshot().unwrap();
        assert_eq!(snapshot.values["app.history_view_mode"], "list");
        assert_eq!(snapshot.values["app.tag_history_view_mode"], "grid");
        assert_eq!(
            s.connection
                .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert!(
            !s.update(
                0,
                [("app.tag_history_view_mode".into(), "grid".into())].into()
            )
            .unwrap()
            .changed
        );
        assert_eq!(
            s.connection
                .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        for key in ["app.history_view_mode", "app.tag_history_view_mode"] {
            for value in ["", "card", "GRID", " list ", "true"] {
                assert_eq!(
                    s.update(0, [(key.into(), value.into())].into())
                        .unwrap_err()
                        .code,
                    "invalid_value"
                );
            }
        }
        let change = s
            .update(0, [("app.history_view_mode".into(), "grid".into())].into())
            .unwrap();
        assert!(change.changed);
        assert_eq!(change.snapshot.revision, 1);
        assert_eq!(change.snapshot.values["app.tag_history_view_mode"], "grid");
        assert_eq!(
            s.update(
                0,
                [("app.tag_history_view_mode".into(), "list".into())].into()
            )
            .unwrap_err()
            .code,
            "revision_conflict"
        );
        let change = s
            .update(
                1,
                [("app.tag_history_view_mode".into(), "list".into())].into(),
            )
            .unwrap();
        assert_eq!(change.snapshot.values["app.history_view_mode"], "grid");
        assert_eq!(change.snapshot.values["app.tag_history_view_mode"], "list");
        s.connection.execute_batch("CREATE TRIGGER reject_layout BEFORE UPDATE ON settings BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(s
            .update(2, [("app.history_view_mode".into(), "list".into())].into())
            .is_err());
        assert_eq!(s.snapshot().unwrap(), change.snapshot);
    }
}
