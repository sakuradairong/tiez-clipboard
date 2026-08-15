//! Shared WebDAV cloud-sync configuration for native TieZ frontends.
//!
//! Passwords may be written or cleared through this API but are never included
//! in snapshots, mutations, probe results, or error messages. Network probes
//! perform a read-only `PROPFIND` and never create remote collections or upload
//! clipboard content.

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::{Method, StatusCode, Url};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::cloud_sync_protocol::CloudSyncContentPrefs;
use crate::cloud_sync_runner::CloudSyncRunnerConfig;

pub const DEFAULT_INTERVAL_SECS: u64 = 120;
pub const MIN_INTERVAL_SECS: u64 = 5;
pub const MAX_INTERVAL_SECS: u64 = 3600;
pub const DEFAULT_SNAPSHOT_INTERVAL_MIN: i64 = 720;
pub const MIN_SNAPSHOT_INTERVAL_MIN: i64 = 5;
pub const MAX_SNAPSHOT_INTERVAL_MIN: i64 = 1440;
pub const DEFAULT_WEBDAV_BASE_PATH: &str = "tiez-sync";

const PROBE_TIMEOUT_SECS: u64 = 15;
const KEY_ENABLED: &str = "cloud_sync_enabled";
const KEY_AUTO: &str = "cloud_sync_auto";
const KEY_PROVIDER: &str = "cloud_sync_provider";
const KEY_LEGACY_SERVER: &str = "cloud_sync_server";
const KEY_LEGACY_PASSWORD: &str = "cloud_sync_api_key";
const KEY_INTERVAL: &str = "cloud_sync_interval_sec";
const KEY_SNAPSHOT_INTERVAL: &str = "cloud_sync_snapshot_interval_min";
const KEY_WEBDAV_URL: &str = "cloud_sync_webdav_url";
const KEY_WEBDAV_USERNAME: &str = "cloud_sync_webdav_username";
const KEY_WEBDAV_PASSWORD: &str = "cloud_sync_webdav_password";
const KEY_WEBDAV_BASE_PATH: &str = "cloud_sync_webdav_base_path";
const KEY_CONTENT_PREFS: &str = "cloud_sync_content_prefs";

const STORED_KEYS: &[&str] = &[
    KEY_ENABLED,
    KEY_AUTO,
    KEY_PROVIDER,
    KEY_LEGACY_SERVER,
    KEY_LEGACY_PASSWORD,
    KEY_INTERVAL,
    KEY_SNAPSHOT_INTERVAL,
    KEY_WEBDAV_URL,
    KEY_WEBDAV_USERNAME,
    KEY_WEBDAV_PASSWORD,
    KEY_WEBDAV_BASE_PATH,
    KEY_CONTENT_PREFS,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudSyncContentPreferences {
    #[serde(default = "default_true")]
    pub text: bool,
    #[serde(default = "default_true")]
    pub image: bool,
    #[serde(rename = "file_path", default = "default_true")]
    pub file_path: bool,
    #[serde(default = "default_true")]
    pub emoji: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for CloudSyncContentPreferences {
    fn default() -> Self {
        Self {
            text: true,
            image: true,
            file_path: true,
            emoji: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CloudSyncSettingsSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub enabled: bool,
    pub auto_sync: bool,
    pub provider: &'static str,
    pub webdav_url: String,
    pub webdav_username: String,
    pub password_configured: bool,
    pub webdav_base_path: String,
    pub interval_secs: u64,
    pub snapshot_interval_min: i64,
    pub content_prefs: CloudSyncContentPreferences,
    pub secure_transport: bool,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudSyncSettingsUpdate {
    pub enabled: bool,
    pub auto_sync: bool,
    pub webdav_url: String,
    pub webdav_username: String,
    #[serde(default)]
    pub webdav_password: Option<String>,
    #[serde(default)]
    pub clear_password: bool,
    pub webdav_base_path: String,
    pub interval_secs: u64,
    pub snapshot_interval_min: i64,
    pub content_prefs: CloudSyncContentPreferences,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CloudSyncSettingsMutation {
    #[serde(flatten)]
    pub snapshot: CloudSyncSettingsSnapshot,
    pub password_changed: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CloudSyncProbeResult {
    pub reachable: bool,
    pub secure_transport: bool,
    pub status_code: Option<u16>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudSyncSettingsErrorKind {
    InvalidDatabase,
    Storage,
    Validation,
    ReadOnly,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudSyncSettingsError {
    kind: CloudSyncSettingsErrorKind,
    message: String,
}

impl CloudSyncSettingsError {
    fn new(kind: CloudSyncSettingsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CloudSyncSettingsErrorKind {
        self.kind
    }
}

impl fmt::Display for CloudSyncSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CloudSyncSettingsError {}

enum CloudSyncSettingsAdapter {
    Memory(BTreeMap<String, String>),
    Sqlite {
        database_path: PathBuf,
        read_only: bool,
    },
}

pub struct CloudSyncSettings {
    adapter: CloudSyncSettingsAdapter,
    generation: u64,
}

struct ResolvedCloudSyncSettings {
    enabled: bool,
    auto_sync: bool,
    webdav_url: String,
    webdav_username: String,
    webdav_password: String,
    webdav_base_path: String,
    interval_secs: u64,
    snapshot_interval_min: i64,
    content_prefs: CloudSyncContentPreferences,
}

impl CloudSyncSettings {
    pub fn in_memory() -> Self {
        Self {
            adapter: CloudSyncSettingsAdapter::Memory(default_values()),
            generation: 1,
        }
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, CloudSyncSettingsError> {
        let database_path = database_path.into();
        let connection = open_connection(&database_path, read_only)?;
        connection
            .query_row("SELECT 1 FROM settings LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(|error| storage_error("failed to inspect settings table", error))?;
        Ok(Self {
            adapter: CloudSyncSettingsAdapter::Sqlite {
                database_path,
                read_only,
            },
            generation: 1,
        })
    }

    pub fn snapshot(&self) -> Result<CloudSyncSettingsSnapshot, CloudSyncSettingsError> {
        let (adapter, read_only, values) = self.load_values()?;
        let resolved = resolve_values(&values);
        Ok(snapshot_from_resolved(
            adapter,
            read_only,
            self.generation,
            &resolved,
        ))
    }

    /// Build the secret-bearing runtime configuration for trusted Rust code.
    ///
    /// The returned type deliberately implements neither `Debug` nor
    /// `Serialize`; passwords must never cross the C ABI or appear in logs.
    pub fn runner_config(
        &self,
        device_id: impl Into<String>,
    ) -> Result<Option<CloudSyncRunnerConfig>, CloudSyncSettingsError> {
        let (_, read_only, values) = self.load_values()?;
        let resolved = resolve_values(&values);
        if !resolved.enabled {
            return Ok(None);
        }
        if read_only {
            return Err(CloudSyncSettingsError::new(
                CloudSyncSettingsErrorKind::ReadOnly,
                "cloud sync requires a writable database",
            ));
        }
        validate_webdav_url(&resolved.webdav_url, true)?;
        Ok(Some(CloudSyncRunnerConfig::new(
            device_id,
            resolved.webdav_url,
            resolved.webdav_username,
            resolved.webdav_password,
            resolved.webdav_base_path,
            resolved.interval_secs,
            resolved.snapshot_interval_min.saturating_mul(60),
            CloudSyncContentPrefs {
                text: resolved.content_prefs.text,
                image: resolved.content_prefs.image,
                file_path: resolved.content_prefs.file_path,
                emoji: resolved.content_prefs.emoji,
            },
        )))
    }

    pub fn update(
        &mut self,
        update: CloudSyncSettingsUpdate,
    ) -> Result<CloudSyncSettingsMutation, CloudSyncSettingsError> {
        let normalized = normalize_update(update)?;
        let password_changed = normalized.clear_password || normalized.webdav_password.is_some();
        let writes = update_writes(&normalized)?;

        let adapter = match &mut self.adapter {
            CloudSyncSettingsAdapter::Memory(values) => {
                for (key, value) in &writes {
                    values.insert((*key).to_owned(), value.clone());
                }
                "memory"
            }
            CloudSyncSettingsAdapter::Sqlite {
                database_path,
                read_only,
            } => {
                if *read_only {
                    return Err(CloudSyncSettingsError::new(
                        CloudSyncSettingsErrorKind::ReadOnly,
                        "cloud-sync settings are read-only for this database",
                    ));
                }
                let mut connection = open_connection(database_path, false)?;
                let transaction = connection.transaction().map_err(|error| {
                    storage_error("failed to begin cloud settings update", error)
                })?;
                for (key, value) in &writes {
                    transaction
                        .execute(
                            "INSERT INTO settings (key, value) VALUES (?1, ?2)
                             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                            [*key, value.as_str()],
                        )
                        .map_err(|error| {
                            storage_error(&format!("failed to write cloud setting {key}"), error)
                        })?;
                }
                transaction
                    .commit()
                    .map_err(|error| storage_error("failed to commit cloud settings", error))?;
                "sqlite"
            }
        };

        self.generation = self.generation.saturating_add(1);
        let (_, read_only, values) = self.load_values()?;
        let snapshot = snapshot_from_resolved(
            adapter,
            read_only,
            self.generation,
            &resolve_values(&values),
        );
        Ok(CloudSyncSettingsMutation {
            snapshot,
            password_changed,
            message: "Cloud sync settings updated".to_owned(),
        })
    }

    pub fn probe_webdav(&self) -> Result<CloudSyncProbeResult, CloudSyncSettingsError> {
        let (_, _, values) = self.load_values()?;
        let resolved = resolve_values(&values);
        let url = validate_webdav_url(&resolved.webdav_url, true)?;
        let secure_transport = url.scheme() == "https";
        let method = Method::from_bytes(b"PROPFIND").map_err(|error| {
            CloudSyncSettingsError::new(
                CloudSyncSettingsErrorKind::Network,
                format!("failed to prepare WebDAV probe: {error}"),
            )
        })?;
        let client = Client::builder()
            .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
            .redirect(Policy::none())
            .build()
            .map_err(|error| network_error("failed to build WebDAV client", error))?;
        let mut request = client
            .request(method, url)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
                 <d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/></d:prop></d:propfind>",
            );
        if !resolved.webdav_username.is_empty() {
            request = request.basic_auth(resolved.webdav_username, Some(resolved.webdav_password));
        }

        let response = request
            .send()
            .map_err(|error| network_error("WebDAV probe failed", error))?;
        let status = response.status();
        let reachable = status.is_success() || status == StatusCode::MULTI_STATUS;
        let message = if reachable {
            "WebDAV endpoint and credentials are reachable".to_owned()
        } else if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            "WebDAV endpoint rejected the configured credentials".to_owned()
        } else if status.is_redirection() {
            "WebDAV endpoint redirected the probe; save the final HTTPS URL instead".to_owned()
        } else {
            format!("WebDAV endpoint returned HTTP {}", status.as_u16())
        };
        Ok(CloudSyncProbeResult {
            reachable,
            secure_transport,
            status_code: Some(status.as_u16()),
            message,
        })
    }

    fn load_values(
        &self,
    ) -> Result<(&'static str, bool, BTreeMap<String, String>), CloudSyncSettingsError> {
        match &self.adapter {
            CloudSyncSettingsAdapter::Memory(values) => Ok(("memory", false, values.clone())),
            CloudSyncSettingsAdapter::Sqlite {
                database_path,
                read_only,
            } => {
                let connection = open_connection(database_path, *read_only)?;
                let mut values = BTreeMap::new();
                for key in STORED_KEYS {
                    let value = connection
                        .query_row("SELECT value FROM settings WHERE key = ?1", [*key], |row| {
                            row.get::<_, String>(0)
                        })
                        .optional()
                        .map_err(|error| {
                            storage_error(&format!("failed to read cloud setting {key}"), error)
                        })?;
                    if let Some(value) = value {
                        values.insert((*key).to_owned(), value);
                    }
                }
                Ok((
                    if *read_only {
                        "sqlite-read-only"
                    } else {
                        "sqlite"
                    },
                    *read_only,
                    values,
                ))
            }
        }
    }
}

fn default_values() -> BTreeMap<String, String> {
    BTreeMap::from([
        (KEY_ENABLED.to_owned(), "false".to_owned()),
        (KEY_AUTO.to_owned(), "true".to_owned()),
        (KEY_PROVIDER.to_owned(), "webdav".to_owned()),
        (KEY_LEGACY_SERVER.to_owned(), String::new()),
        (KEY_LEGACY_PASSWORD.to_owned(), String::new()),
        (KEY_INTERVAL.to_owned(), DEFAULT_INTERVAL_SECS.to_string()),
        (
            KEY_SNAPSHOT_INTERVAL.to_owned(),
            DEFAULT_SNAPSHOT_INTERVAL_MIN.to_string(),
        ),
        (KEY_WEBDAV_URL.to_owned(), String::new()),
        (KEY_WEBDAV_USERNAME.to_owned(), String::new()),
        (KEY_WEBDAV_PASSWORD.to_owned(), String::new()),
        (
            KEY_WEBDAV_BASE_PATH.to_owned(),
            DEFAULT_WEBDAV_BASE_PATH.to_owned(),
        ),
        (
            KEY_CONTENT_PREFS.to_owned(),
            serde_json::to_string(&CloudSyncContentPreferences::default())
                .expect("default cloud preferences serialize"),
        ),
    ])
}

fn resolve_values(values: &BTreeMap<String, String>) -> ResolvedCloudSyncSettings {
    let webdav_url = non_empty_value(values, KEY_WEBDAV_URL)
        .or_else(|| non_empty_value(values, KEY_LEGACY_SERVER))
        .unwrap_or_default();
    let webdav_password = non_empty_value(values, KEY_WEBDAV_PASSWORD)
        .or_else(|| non_empty_value(values, KEY_LEGACY_PASSWORD))
        .unwrap_or_default();
    ResolvedCloudSyncSettings {
        enabled: bool_value(values, KEY_ENABLED, false),
        auto_sync: bool_value(values, KEY_AUTO, true),
        webdav_url: webdav_url.trim().trim_end_matches('/').to_owned(),
        webdav_username: value(values, KEY_WEBDAV_USERNAME, "").trim().to_owned(),
        webdav_password,
        webdav_base_path: normalize_base_path(&value(
            values,
            KEY_WEBDAV_BASE_PATH,
            DEFAULT_WEBDAV_BASE_PATH,
        ))
        .unwrap_or_else(|_| DEFAULT_WEBDAV_BASE_PATH.to_owned()),
        interval_secs: integer_value(values, KEY_INTERVAL, DEFAULT_INTERVAL_SECS)
            .clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS),
        snapshot_interval_min: integer_value(
            values,
            KEY_SNAPSHOT_INTERVAL,
            DEFAULT_SNAPSHOT_INTERVAL_MIN as u64,
        )
        .clamp(
            MIN_SNAPSHOT_INTERVAL_MIN as u64,
            MAX_SNAPSHOT_INTERVAL_MIN as u64,
        ) as i64,
        content_prefs: serde_json::from_str(&value(values, KEY_CONTENT_PREFS, ""))
            .unwrap_or_default(),
    }
}

fn snapshot_from_resolved(
    adapter: &'static str,
    read_only: bool,
    generation: u64,
    resolved: &ResolvedCloudSyncSettings,
) -> CloudSyncSettingsSnapshot {
    CloudSyncSettingsSnapshot {
        adapter,
        read_only,
        generation,
        enabled: resolved.enabled,
        auto_sync: resolved.auto_sync,
        provider: "webdav",
        webdav_url: resolved.webdav_url.clone(),
        webdav_username: resolved.webdav_username.clone(),
        password_configured: !resolved.webdav_password.is_empty(),
        webdav_base_path: resolved.webdav_base_path.clone(),
        interval_secs: resolved.interval_secs,
        snapshot_interval_min: resolved.snapshot_interval_min,
        content_prefs: resolved.content_prefs.clone(),
        secure_transport: is_secure_transport(&resolved.webdav_url),
    }
}

fn normalize_update(
    mut update: CloudSyncSettingsUpdate,
) -> Result<CloudSyncSettingsUpdate, CloudSyncSettingsError> {
    if update.clear_password && update.webdav_password.is_some() {
        return Err(validation_error(
            "webdav_password",
            "cannot be replaced and cleared in the same update",
        ));
    }
    if let Some(password) = &update.webdav_password {
        if password.is_empty() {
            return Err(validation_error(
                "webdav_password",
                "must be omitted to preserve it or cleared explicitly",
            ));
        }
        if password.len() > 8192 {
            return Err(validation_error("webdav_password", "is too long"));
        }
    }
    update.webdav_url = update.webdav_url.trim().trim_end_matches('/').to_owned();
    if update.enabled && update.webdav_url.is_empty() {
        return Err(validation_error(
            "webdav_url",
            "is required when cloud sync is enabled",
        ));
    }
    if !update.webdav_url.is_empty() {
        validate_webdav_url(&update.webdav_url, false)?;
    }
    update.webdav_username = update.webdav_username.trim().to_owned();
    if update.webdav_username.len() > 1024 {
        return Err(validation_error("webdav_username", "is too long"));
    }
    update.webdav_base_path = normalize_base_path(&update.webdav_base_path)?;
    if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&update.interval_secs) {
        return Err(validation_error(
            "interval_secs",
            &format!("must be from {MIN_INTERVAL_SECS} to {MAX_INTERVAL_SECS}"),
        ));
    }
    if !(MIN_SNAPSHOT_INTERVAL_MIN..=MAX_SNAPSHOT_INTERVAL_MIN)
        .contains(&update.snapshot_interval_min)
    {
        return Err(validation_error(
            "snapshot_interval_min",
            &format!("must be from {MIN_SNAPSHOT_INTERVAL_MIN} to {MAX_SNAPSHOT_INTERVAL_MIN}"),
        ));
    }
    Ok(update)
}

fn update_writes(
    update: &CloudSyncSettingsUpdate,
) -> Result<Vec<(&'static str, String)>, CloudSyncSettingsError> {
    let mut writes = vec![
        (KEY_ENABLED, update.enabled.to_string()),
        (KEY_AUTO, update.auto_sync.to_string()),
        (KEY_PROVIDER, "webdav".to_owned()),
        (KEY_WEBDAV_URL, update.webdav_url.clone()),
        (KEY_WEBDAV_USERNAME, update.webdav_username.clone()),
        (KEY_WEBDAV_BASE_PATH, update.webdav_base_path.clone()),
        (KEY_INTERVAL, update.interval_secs.to_string()),
        (
            KEY_SNAPSHOT_INTERVAL,
            update.snapshot_interval_min.to_string(),
        ),
        (
            KEY_CONTENT_PREFS,
            serde_json::to_string(&update.content_prefs).map_err(|error| {
                CloudSyncSettingsError::new(
                    CloudSyncSettingsErrorKind::Validation,
                    format!("failed to serialize content preferences: {error}"),
                )
            })?,
        ),
    ];
    if update.clear_password {
        writes.push((KEY_WEBDAV_PASSWORD, String::new()));
        writes.push((KEY_LEGACY_PASSWORD, String::new()));
    } else if let Some(password) = &update.webdav_password {
        writes.push((KEY_WEBDAV_PASSWORD, password.clone()));
    }
    Ok(writes)
}

fn validate_webdav_url(raw: &str, required: bool) -> Result<Url, CloudSyncSettingsError> {
    if raw.trim().is_empty() {
        return Err(validation_error(
            "webdav_url",
            if required { "is required" } else { "is empty" },
        ));
    }
    if raw.len() > 4096 {
        return Err(validation_error("webdav_url", "is too long"));
    }
    let url =
        Url::parse(raw).map_err(|_| validation_error("webdav_url", "must be an absolute URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(validation_error(
            "webdav_url",
            "must use HTTP or HTTPS and include a host",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(validation_error(
            "webdav_url",
            "must not embed credentials; use the dedicated fields",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(validation_error(
            "webdav_url",
            "must not contain a query string or fragment",
        ));
    }
    if url.scheme() != "https" && !is_loopback_url(&url) {
        return Err(validation_error(
            "webdav_url",
            "must use HTTPS (HTTP is allowed only for local loopback testing)",
        ));
    }
    Ok(url)
}

fn normalize_base_path(raw: &str) -> Result<String, CloudSyncSettingsError> {
    let trimmed = raw.trim().trim_matches('/');
    let normalized = if trimmed.is_empty() {
        DEFAULT_WEBDAV_BASE_PATH
    } else {
        trimmed
    };
    if normalized.len() > 512 {
        return Err(validation_error("webdav_base_path", "is too long"));
    }
    if normalized.contains('\\')
        || normalized.chars().any(char::is_control)
        || normalized
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(validation_error(
            "webdav_base_path",
            "contains an unsafe path segment",
        ));
    }
    Ok(normalized.to_owned())
}

fn is_secure_transport(raw: &str) -> bool {
    Url::parse(raw)
        .map(|url| url.scheme() == "https")
        .unwrap_or(false)
}

fn is_loopback_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

fn value(values: &BTreeMap<String, String>, key: &str, fallback: &str) -> String {
    values
        .get(key)
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

fn non_empty_value(values: &BTreeMap<String, String>, key: &str) -> Option<String> {
    values.get(key).and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn bool_value(values: &BTreeMap<String, String>, key: &str, fallback: bool) -> bool {
    values
        .get(key)
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(fallback)
}

fn integer_value(values: &BTreeMap<String, String>, key: &str, fallback: u64) -> u64 {
    values
        .get(key)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn validation_error(field: &str, detail: &str) -> CloudSyncSettingsError {
    CloudSyncSettingsError::new(
        CloudSyncSettingsErrorKind::Validation,
        format!("cloud-sync field {field} {detail}"),
    )
}

fn open_connection(
    database_path: &PathBuf,
    read_only: bool,
) -> Result<Connection, CloudSyncSettingsError> {
    if !database_path.is_file() {
        return Err(CloudSyncSettingsError::new(
            CloudSyncSettingsErrorKind::InvalidDatabase,
            format!("database does not exist: {}", database_path.display()),
        ));
    }
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(database_path, flags)
        .map_err(|error| storage_error("failed to open cloud settings database", error))
}

fn storage_error(context: &str, error: impl fmt::Display) -> CloudSyncSettingsError {
    CloudSyncSettingsError::new(
        CloudSyncSettingsErrorKind::Storage,
        format!("{context}: {error}"),
    )
}

fn network_error(context: &str, error: impl fmt::Display) -> CloudSyncSettingsError {
    CloudSyncSettingsError::new(
        CloudSyncSettingsErrorKind::Network,
        format!("{context}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_database(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tiez-cloud-settings-{name}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    fn create_database(path: &PathBuf) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('cloud_sync_webdav_url', 'https://dav.example.test/root'),
                    ('cloud_sync_webdav_username', 'alice'),
                    ('cloud_sync_webdav_password', 'must-not-cross-boundary'),
                    ('cloud_sync_webdav_base_path', 'devices/tiez');",
            )
            .unwrap();
    }

    fn update_for(url: String) -> CloudSyncSettingsUpdate {
        CloudSyncSettingsUpdate {
            enabled: true,
            auto_sync: true,
            webdav_url: url,
            webdav_username: "alice".to_owned(),
            webdav_password: Some("secret".to_owned()),
            clear_password: false,
            webdav_base_path: "tiez-sync".to_owned(),
            interval_secs: 120,
            snapshot_interval_min: 720,
            content_prefs: CloudSyncContentPreferences::default(),
        }
    }

    #[test]
    fn snapshots_never_serialize_passwords() {
        let path = temporary_database("secret-boundary");
        create_database(&path);
        let settings = CloudSyncSettings::open_sqlite(path.clone(), false).unwrap();
        let snapshot = settings.snapshot().unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();

        assert!(snapshot.password_configured);
        assert!(!json.contains("must-not-cross-boundary"));
        assert!(!json.contains("webdav_password"));

        drop(settings);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn trusted_runner_config_keeps_password_inside_rust() {
        let mut settings = CloudSyncSettings::in_memory();
        let mutation = settings
            .update(update_for("https://dav.example.test/runtime".to_owned()))
            .unwrap();
        let config = settings.runner_config("aaaaaaaa").unwrap().unwrap();

        assert_eq!(config.device_id, "aaaaaaaa");
        assert_eq!(config.webdav_password, "secret");
        assert_eq!(config.snapshot_interval_secs, 43_200);
        let boundary_json = serde_json::to_string(&mutation.snapshot).unwrap();
        assert!(!boundary_json.contains("secret"));
        assert!(!boundary_json.contains("webdav_password"));
    }

    #[test]
    fn sqlite_updates_are_transactional_and_passwords_can_be_cleared() {
        let path = temporary_database("round-trip");
        create_database(&path);
        let mut settings = CloudSyncSettings::open_sqlite(path.clone(), false).unwrap();
        let mutation = settings
            .update(CloudSyncSettingsUpdate {
                webdav_password: None,
                ..update_for("https://dav.example.test/new-root".to_owned())
            })
            .unwrap();
        assert!(mutation.snapshot.password_configured);
        assert_eq!(
            mutation.snapshot.webdav_url,
            "https://dav.example.test/new-root"
        );

        let cleared = settings
            .update(CloudSyncSettingsUpdate {
                webdav_password: None,
                clear_password: true,
                ..update_for("https://dav.example.test/new-root".to_owned())
            })
            .unwrap();
        assert!(!cleared.snapshot.password_configured);

        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM settings WHERE key = 'cloud_sync_webdav_password'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            ""
        );
        drop(connection);
        drop(settings);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn validation_rejects_remote_http_embedded_credentials_and_unsafe_paths() {
        let mut settings = CloudSyncSettings::in_memory();
        assert_eq!(
            settings
                .update(update_for("http://dav.example.test/root".to_owned()))
                .unwrap_err()
                .kind(),
            CloudSyncSettingsErrorKind::Validation
        );
        assert_eq!(
            settings
                .update(update_for(
                    "https://alice:secret@dav.example.test/root".to_owned()
                ))
                .unwrap_err()
                .kind(),
            CloudSyncSettingsErrorKind::Validation
        );
        assert_eq!(
            settings
                .update(CloudSyncSettingsUpdate {
                    webdav_base_path: "../escape".to_owned(),
                    ..update_for("https://dav.example.test/root".to_owned())
                })
                .unwrap_err()
                .kind(),
            CloudSyncSettingsErrorKind::Validation
        );
    }

    #[test]
    fn probe_uses_read_only_propfind_and_basic_auth() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0_u8; 8192];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("PROPFIND /dav HTTP/1.1"));
            let request_lower = request.to_ascii_lowercase();
            assert!(request_lower.contains("depth: 0"));
            assert!(request_lower.contains("authorization: basic ywxpy2u6c2vjcmv0"));
            stream
                .write_all(
                    b"HTTP/1.1 207 Multi-Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let mut settings = CloudSyncSettings::in_memory();
        settings
            .update(update_for(format!("http://{address}/dav")))
            .unwrap();
        let result = settings.probe_webdav().unwrap();
        assert!(result.reachable);
        assert!(!result.secure_transport);
        assert_eq!(result.status_code, Some(207));
        server.join().unwrap();
    }

    #[test]
    fn read_only_databases_allow_probe_configuration_reads_but_reject_updates() {
        let path = temporary_database("read-only");
        create_database(&path);
        let mut settings = CloudSyncSettings::open_sqlite(path.clone(), true).unwrap();
        assert!(settings.snapshot().unwrap().read_only);
        assert_eq!(
            settings
                .update(update_for("https://dav.example.test/root".to_owned()))
                .unwrap_err()
                .kind(),
            CloudSyncSettingsErrorKind::ReadOnly
        );
        drop(settings);
        fs::remove_file(path).unwrap();
    }
}
