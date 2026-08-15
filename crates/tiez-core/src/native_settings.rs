//! Allowlisted settings shared by native TieZ frontends.
//!
//! The native boundary deliberately exposes only daily-use, non-secret values.
//! Cloud, MQTT, relay, and credential-bearing settings never cross this API.

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum SettingKind {
    Bool,
    Integer { min: i64, max: i64 },
    Choice(&'static [&'static str]),
}

#[derive(Clone, Copy)]
struct SettingDefinition {
    key: &'static str,
    default_value: &'static str,
    kind: SettingKind,
}

const COLOR_MODES: &[&str] = &["system", "light", "dark"];

const NATIVE_SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        key: "app.color_mode",
        default_value: "system",
        kind: SettingKind::Choice(COLOR_MODES),
    },
    SettingDefinition {
        key: "app.compact_mode",
        default_value: "false",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.persistent",
        default_value: "false",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.persistent_limit_enabled",
        default_value: "true",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.persistent_limit",
        default_value: "500",
        kind: SettingKind::Integer {
            min: 0,
            max: 100_000,
        },
    },
    SettingDefinition {
        key: "app.deduplicate",
        default_value: "true",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.capture_files",
        default_value: "false",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.capture_rich_text",
        default_value: "false",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.rich_text_snapshot_preview",
        default_value: "true",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.privacy_protection",
        default_value: "true",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.hide_tray_icon",
        default_value: "false",
        kind: SettingKind::Bool,
    },
    SettingDefinition {
        key: "app.window_pinned",
        default_value: "false",
        kind: SettingKind::Bool,
    },
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeSettingsSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeSettingMutation {
    pub adapter: &'static str,
    pub key: String,
    pub value: String,
    pub generation: u64,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapturePreferences {
    pub deduplicate: bool,
    pub capture_files: bool,
    pub capture_rich_text: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSettingsErrorKind {
    InvalidDatabase,
    Storage,
    Validation,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSettingsError {
    kind: NativeSettingsErrorKind,
    message: String,
}

impl NativeSettingsError {
    fn new(kind: NativeSettingsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> NativeSettingsErrorKind {
        self.kind
    }
}

impl fmt::Display for NativeSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for NativeSettingsError {}

#[derive(Debug)]
enum NativeSettingsAdapter {
    Memory(BTreeMap<String, String>),
    Sqlite {
        database_path: PathBuf,
        read_only: bool,
    },
}

#[derive(Debug)]
pub struct NativeSettings {
    adapter: NativeSettingsAdapter,
    generation: u64,
}

impl NativeSettings {
    pub fn in_memory() -> Self {
        Self {
            adapter: NativeSettingsAdapter::Memory(default_values()),
            generation: 1,
        }
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, NativeSettingsError> {
        let database_path = database_path.into();
        let connection = open_connection(&database_path, read_only)?;
        connection
            .query_row("SELECT 1 FROM settings LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(|error| storage_error("failed to inspect settings table", error))?;
        Ok(Self {
            adapter: NativeSettingsAdapter::Sqlite {
                database_path,
                read_only,
            },
            generation: 1,
        })
    }

    pub fn snapshot(&self) -> Result<NativeSettingsSnapshot, NativeSettingsError> {
        let (adapter, read_only, values) = match &self.adapter {
            NativeSettingsAdapter::Memory(values) => ("memory", false, values.clone()),
            NativeSettingsAdapter::Sqlite {
                database_path,
                read_only,
            } => {
                let connection = open_connection(database_path, *read_only)?;
                let mut values = default_values();
                for definition in NATIVE_SETTINGS {
                    let value = connection
                        .query_row(
                            "SELECT value FROM settings WHERE key = ?1",
                            [definition.key],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(|error| {
                            storage_error(
                                &format!("failed to read native setting {}", definition.key),
                                error,
                            )
                        })?;
                    if let Some(value) = value {
                        let value = normalize_value(definition, &value)
                            .unwrap_or_else(|_| definition.default_value.to_owned());
                        values.insert(definition.key.to_owned(), value);
                    }
                }
                (
                    if *read_only {
                        "sqlite-read-only"
                    } else {
                        "sqlite"
                    },
                    *read_only,
                    values,
                )
            }
        };
        Ok(NativeSettingsSnapshot {
            adapter,
            read_only,
            generation: self.generation,
            values,
        })
    }

    pub fn update(
        &mut self,
        key: &str,
        value: &str,
    ) -> Result<NativeSettingMutation, NativeSettingsError> {
        let definition = setting_definition(key).ok_or_else(|| {
            NativeSettingsError::new(
                NativeSettingsErrorKind::Validation,
                format!("setting {key} is not exposed to native frontends"),
            )
        })?;
        let value = normalize_value(definition, value)?;
        let adapter = match &mut self.adapter {
            NativeSettingsAdapter::Memory(values) => {
                values.insert(key.to_owned(), value.clone());
                "memory"
            }
            NativeSettingsAdapter::Sqlite {
                database_path,
                read_only,
            } => {
                if *read_only {
                    return Err(NativeSettingsError::new(
                        NativeSettingsErrorKind::ReadOnly,
                        "setting updates are disabled for sqlite-read-only history",
                    ));
                }
                let connection = open_connection(database_path, false)?;
                connection
                    .execute(
                        "INSERT INTO settings (key, value) VALUES (?1, ?2)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        [key, value.as_str()],
                    )
                    .map_err(|error| {
                        storage_error(&format!("failed to write native setting {key}"), error)
                    })?;
                "sqlite"
            }
        };
        self.generation = self.generation.saturating_add(1);
        Ok(NativeSettingMutation {
            adapter,
            key: key.to_owned(),
            value: value.clone(),
            generation: self.generation,
            message: format!("Updated {key} to {value}"),
        })
    }

    pub fn capture_preferences(&self) -> Result<CapturePreferences, NativeSettingsError> {
        let snapshot = self.snapshot()?;
        Ok(CapturePreferences {
            deduplicate: bool_value(&snapshot.values, "app.deduplicate", true),
            capture_files: bool_value(&snapshot.values, "app.capture_files", false),
            capture_rich_text: bool_value(
                &snapshot.values,
                "app.capture_rich_text",
                false,
            ),
        })
    }
}

fn setting_definition(key: &str) -> Option<&'static SettingDefinition> {
    NATIVE_SETTINGS.iter().find(|definition| definition.key == key)
}

fn default_values() -> BTreeMap<String, String> {
    NATIVE_SETTINGS
        .iter()
        .map(|definition| {
            (
                definition.key.to_owned(),
                definition.default_value.to_owned(),
            )
        })
        .collect()
}

fn normalize_value(
    definition: &SettingDefinition,
    value: &str,
) -> Result<String, NativeSettingsError> {
    match definition.kind {
        SettingKind::Bool => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok("true".to_owned()),
            "false" | "0" => Ok("false".to_owned()),
            _ => Err(validation_error(
                definition.key,
                "must be true or false",
            )),
        },
        SettingKind::Integer { min, max } => {
            let parsed = value.trim().parse::<i64>().map_err(|_| {
                validation_error(definition.key, &format!("must be an integer from {min} to {max}"))
            })?;
            if !(min..=max).contains(&parsed) {
                return Err(validation_error(
                    definition.key,
                    &format!("must be an integer from {min} to {max}"),
                ));
            }
            Ok(parsed.to_string())
        }
        SettingKind::Choice(choices) => {
            let normalized = value.trim().to_ascii_lowercase();
            if choices.contains(&normalized.as_str()) {
                Ok(normalized)
            } else {
                Err(validation_error(
                    definition.key,
                    &format!("must be one of {}", choices.join(", ")),
                ))
            }
        }
    }
}

fn bool_value(values: &BTreeMap<String, String>, key: &str, default_value: bool) -> bool {
    values
        .get(key)
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(default_value)
}

fn validation_error(key: &str, detail: &str) -> NativeSettingsError {
    NativeSettingsError::new(
        NativeSettingsErrorKind::Validation,
        format!("setting {key} {detail}"),
    )
}

fn open_connection(
    database_path: &PathBuf,
    read_only: bool,
) -> Result<Connection, NativeSettingsError> {
    if !database_path.is_file() {
        return Err(NativeSettingsError::new(
            NativeSettingsErrorKind::InvalidDatabase,
            format!("database does not exist: {}", database_path.display()),
        ));
    }
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(database_path, flags)
        .map_err(|error| storage_error("failed to open native settings database", error))
}

fn storage_error(context: &str, error: impl fmt::Display) -> NativeSettingsError {
    NativeSettingsError::new(
        NativeSettingsErrorKind::Storage,
        format!("{context}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_database(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tiez-native-settings-{name}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    fn create_settings_database(path: &PathBuf) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.compact_mode', 'true'),
                    ('app.persistent_limit', 'not-a-number'),
                    ('mqtt_password', 'must-not-cross-native-boundary');",
            )
            .unwrap();
    }

    #[test]
    fn memory_settings_are_allowlisted_and_validated() {
        let mut settings = NativeSettings::in_memory();
        let snapshot = settings.snapshot().unwrap();
        assert_eq!(snapshot.values.get("app.compact_mode"), Some(&"false".to_owned()));
        assert!(!snapshot.values.contains_key("mqtt_password"));

        let mutation = settings.update("app.compact_mode", "1").unwrap();
        assert_eq!(mutation.value, "true");
        assert_eq!(mutation.generation, 2);
        assert_eq!(
            settings.update("mqtt_password", "secret").unwrap_err().kind(),
            NativeSettingsErrorKind::Validation
        );
        assert_eq!(
            settings.update("app.persistent_limit", "100001").unwrap_err().kind(),
            NativeSettingsErrorKind::Validation
        );
    }

    #[test]
    fn sqlite_settings_round_trip_without_exposing_secrets() {
        let path = temporary_database("round-trip");
        create_settings_database(&path);
        let mut settings = NativeSettings::open_sqlite(path.clone(), false).unwrap();

        let snapshot = settings.snapshot().unwrap();
        assert_eq!(snapshot.values.get("app.compact_mode"), Some(&"true".to_owned()));
        assert_eq!(snapshot.values.get("app.persistent_limit"), Some(&"500".to_owned()));
        assert!(!snapshot.values.contains_key("mqtt_password"));
        settings.update("app.capture_files", "true").unwrap();

        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM settings WHERE key = 'app.capture_files'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "true"
        );
        drop(connection);
        drop(settings);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sqlite_read_only_settings_reject_updates() {
        let path = temporary_database("read-only");
        create_settings_database(&path);
        let mut settings = NativeSettings::open_sqlite(path.clone(), true).unwrap();

        assert!(settings.snapshot().unwrap().read_only);
        assert_eq!(
            settings.update("app.compact_mode", "false").unwrap_err().kind(),
            NativeSettingsErrorKind::ReadOnly
        );

        drop(settings);
        fs::remove_file(path).unwrap();
    }
}
