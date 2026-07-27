use crate::app::commands::file_cmd::{image_ext_from_mime, save_emoji_favorite_bytes_to_dir};
use crate::database::{
    calc_legacy_text_hash, has_sensitive_tag, uses_text_content_hash, DbState, ENCRYPT_PREFIX,
};
use crate::domain::models::ClipboardEntry;
use crate::error::{AppError, AppResult};
use crate::infrastructure::encryption;
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use base64::Engine;
use regex::Regex;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::sleep;
use urlencoding::decode;

const DEFAULT_INTERVAL_SECS: u64 = 120;
const MIN_INTERVAL_SECS: u64 = 5;
const MAX_INTERVAL_SECS: u64 = 3600;
const DEFAULT_SNAPSHOT_INTERVAL_MIN: i64 = 720;
const MIN_SNAPSHOT_INTERVAL_MIN: i64 = 5;
const MAX_SNAPSHOT_INTERVAL_MIN: i64 = 1440;
const SYNC_FETCH_PAGE_SIZE: i32 = 1000;
const DEFAULT_WEBDAV_BASE_PATH: &str = "tiez-sync";
const MAX_REMOTE_SNAPSHOTS: usize = 24;
const MAX_INLINE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const RICH_IMAGE_FALLBACK_PREFIX: &str = "<!--TIEZ_RICH_IMAGE:";
const RICH_IMAGE_FALLBACK_SUFFIX: &str = "-->";
const WEBDAV_OP_BATCH_SIZE: usize = 400;
const EMOJI_FAVORITES_SETTING_KEY: &str = "app.emoji_favorites";
const CLOUD_SYNC_WEBDAV_LOCAL_SEQ_KEY: &str = "cloud_sync_webdav_local_seq";
const CLOUD_SYNC_WEBDAV_OP_CURSOR_MAP_KEY: &str = "cloud_sync_webdav_op_cursor_map";
const CLOUD_SYNC_WEBDAV_BLOB_CACHE_KEY: &str = "cloud_sync_webdav_blob_cache";
const CLOUD_SYNC_WEBDAV_LAST_SNAPSHOT_PUSH_AT_KEY: &str = "cloud_sync_webdav_last_snapshot_push_at";
const CLOUD_SYNC_WEBDAV_LAST_SNAPSHOT_PULL_AT_KEY: &str = "cloud_sync_webdav_last_snapshot_pull_at";
const CLOUD_SYNC_WEBDAV_LAST_HEAD_REBUILD_AT_KEY: &str = "cloud_sync_webdav_last_head_rebuild_at";
const BLOB_KIND_IMAGE: &str = "image";
const BLOB_KIND_CONTENT: &str = "content";
const BLOB_KIND_HTML: &str = "html";
const BLOB_THRESHOLD_CONTENT: usize = 12 * 1024;
const BLOB_THRESHOLD_HTML: usize = 24 * 1024;
const WEBDAV_REQUEST_TIMEOUT_SECS: u64 = 45;
const WEBDAV_MAX_RETRIES: usize = 3;
const WEBDAV_JSON_READ_RETRIES: usize = 3;
const WEBDAV_RETRY_BASE_DELAY_MS: u64 = 600;
const WEBDAV_HEAD_REBUILD_INTERVAL_SECS: i64 = 5 * 60;
const WEBDAV_HEAD_FILENAME: &str = "head.json";
const WEBDAV_BLOB_CACHE_MAX_ENTRIES: usize = 5000;
const HASH_VERSION_LEGACY: i64 = 1;
const HASH_VERSION_WHITESPACE: i64 = 2;

fn default_hash_version() -> i64 {
    HASH_VERSION_LEGACY
}

static CLOUD_SYNC_TASK_ACTIVE: AtomicBool = AtomicBool::new(false);
static CLOUD_SYNC_REQUESTED: AtomicBool = AtomicBool::new(false);
static CLOUD_SYNC_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
static CLOUD_SYNC_LAST_SYNC_AT: AtomicI64 = AtomicI64::new(0);
static LAST_PUSHED_EMOJI_HASH: AtomicI64 = AtomicI64::new(0);
static CLOUD_SYNC_BACKOFF_UNTIL: AtomicI64 = AtomicI64::new(0);

// 用于记录在本次运行中，哪些 WebDAV 目录已经确认存在，避免重复发网络请求
static WEBDAV_KNOWN_DIRS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudSyncProvider {
    #[allow(dead_code)]
    Http,
    WebDav,
}

impl CloudSyncProvider {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::WebDav => "webdav",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSyncStatus {
    pub state: String, // disabled | idle | syncing | error
    pub running: bool,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
    pub uploaded_items: usize,
    pub received_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CloudSyncContentPrefs {
    #[serde(default = "default_cloud_sync_pref_true")]
    text: bool,
    #[serde(default = "default_cloud_sync_pref_true")]
    image: bool,
    #[serde(rename = "file_path", default = "default_cloud_sync_pref_true")]
    file_path: bool,
    #[serde(default = "default_cloud_sync_pref_true")]
    emoji: bool,
}

const fn default_cloud_sync_pref_true() -> bool {
    true
}

impl Default for CloudSyncContentPrefs {
    fn default() -> Self {
        Self {
            text: true,
            image: true,
            file_path: true,
            emoji: true,
        }
    }
}

impl CloudSyncContentPrefs {
    fn includes_content_type(&self, content_type: &str) -> bool {
        if !is_cloud_clipboard_content_type(content_type) {
            return false;
        }
        match content_type {
            "image" => self.image,
            "file" | "video" => self.file_path,
            "emoji_sync" => self.emoji,
            "text" | "code" | "url" | "rich_text" => self.text,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct CloudSyncConfig {
    enabled: bool,
    auto_sync: bool,
    provider: CloudSyncProvider,
    base_url: String,
    api_key: String,
    device_id: String,
    interval_secs: u64,
    snapshot_interval_secs: i64,
    cursor: i64,
    webdav_url: String,
    webdav_username: String,
    webdav_password: String,
    webdav_base_path: String,
    content_prefs: CloudSyncContentPrefs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CloudSyncItem {
    pub content_type: String,
    pub content: String,
    #[serde(default)]
    pub content_hash: i64,
    #[serde(default = "default_hash_version")]
    pub hash_version: i64,
    #[serde(default)]
    pub deleted_at: i64,
    #[serde(default)]
    pub html_content: Option<String>,
    #[serde(default)]
    pub content_blob_hash: Option<String>,
    #[serde(default)]
    pub html_blob_hash: Option<String>,
    pub source_app: String,
    pub timestamp: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub updated_by: String,
    pub preview: String,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub use_count: i32,
    #[serde(default)]
    pub pinned_order: i64,
}

type LocalSyncEntryState = (
    i64,
    bool,
    i64,
    String,
    String,
    i32,
    String,
    Option<String>,
    bool,
    i64,
    String,
);

#[derive(Debug, Serialize)]
struct CloudSyncRequest {
    device_id: String,
    cursor: i64,
    entries: Vec<CloudSyncItem>,
}

#[derive(Debug, Deserialize)]
struct CloudSyncResponse {
    #[serde(default)]
    cursor: Option<i64>,
    #[serde(default)]
    entries: Vec<CloudSyncItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebDavDeviceSnapshot {
    device_id: String,
    updated_at: i64,
    #[serde(default)]
    latest_op_seq: i64,
    entries: Vec<CloudSyncItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebDavSettingsSnapshot {
    device_id: String,
    updated_at: i64,
    settings: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct WebDavPaths {
    devices_path: String,
    settings_path: String,
    ops_path: String,
    head_path: String,
    blobs_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebDavOpsBatch {
    device_id: String,
    seq: i64,
    updated_at: i64,
    entries: Vec<CloudSyncItem>,
}

#[derive(Debug, Clone)]
struct WebDavOpRef {
    device_id: String,
    seq: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WebDavDeviceHead {
    #[serde(default)]
    latest_op_seq: i64,
    #[serde(default)]
    snapshot_updated_at: i64,
    #[serde(default)]
    snapshot_op_seq: i64,
    #[serde(default)]
    settings_updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WebDavSyncHead {
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    devices: BTreeMap<String, WebDavDeviceHead>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn status_store() -> &'static Mutex<CloudSyncStatus> {
    static STORE: OnceLock<Mutex<CloudSyncStatus>> = OnceLock::new();
    STORE.get_or_init(|| {
        Mutex::new(CloudSyncStatus {
            state: "disabled".to_string(),
            running: false,
            last_sync_at: None,
            last_error: None,
            uploaded_items: 0,
            received_items: 0,
        })
    })
}

fn sync_run_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn disabled_status() -> CloudSyncStatus {
    CloudSyncStatus {
        state: "disabled".to_string(),
        running: false,
        last_sync_at: None,
        last_error: None,
        uploaded_items: 0,
        received_items: 0,
    }
}

fn cloud_sync_cancel_requested() -> bool {
    CLOUD_SYNC_CANCEL_REQUESTED.load(Ordering::Relaxed)
}

fn emit_status(app: Option<&AppHandle>, mut next: CloudSyncStatus) {
    if next.last_sync_at.is_none() {
        let ts = CLOUD_SYNC_LAST_SYNC_AT.load(Ordering::Relaxed);
        if ts > 0 {
            next.last_sync_at = Some(ts);
        }
    }
    if let Ok(mut guard) = status_store().lock() {
        *guard = next.clone();
    }
    if let Some(handle) = app {
        let _ = handle.emit("cloud-sync-status", next);
    }
}

fn parse_interval_secs(raw: Option<String>) -> u64 {
    let parsed = raw
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    parsed.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)
}

fn parse_snapshot_interval_secs(raw: Option<String>) -> i64 {
    let parsed_min = raw
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_SNAPSHOT_INTERVAL_MIN)
        .clamp(MIN_SNAPSHOT_INTERVAL_MIN, MAX_SNAPSHOT_INTERVAL_MIN);
    parsed_min.saturating_mul(60)
}

fn normalize_webdav_base_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        DEFAULT_WEBDAV_BASE_PATH.to_string()
    } else {
        trimmed.to_string()
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn get_blob_path(base_blobs: &str, kind: &str, hash: &str) -> String {
    let prefix = if hash.len() >= 2 { &hash[0..2] } else { "xx" };
    format!("{}/{}/{}_{}.blob", base_blobs, prefix, kind, hash)
}

fn blob_cache_storage_key(cfg: &CloudSyncConfig, relative_path: &str) -> String {
    format!(
        "{}|{}|{}",
        cfg.webdav_url.trim_end_matches('/'),
        normalize_webdav_base_path(&cfg.webdav_base_path),
        relative_path
    )
}

fn get_config(app: &AppHandle) -> Option<CloudSyncConfig> {
    let db_state = app.try_state::<DbState>()?;
    let enabled = db_state
        .settings_repo
        .get("cloud_sync_enabled")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    let auto_sync = db_state
        .settings_repo
        .get("cloud_sync_auto")
        .ok()
        .flatten()
        .map(|v| v != "false")
        .unwrap_or(true);

    // HTTP provider is intentionally disabled for now.
    // TODO: Restore provider switching after a real HTTP sync service is available.
    let provider = CloudSyncProvider::WebDav;

    let base_url = db_state
        .settings_repo
        .get("cloud_sync_server")
        .ok()
        .flatten()
        .unwrap_or_default()
        .trim()
        .to_string();

    let api_key = db_state
        .settings_repo
        .get("cloud_sync_api_key")
        .ok()
        .flatten()
        .unwrap_or_default();

    let cursor = db_state
        .settings_repo
        .get("cloud_sync_cursor")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    let interval_secs = parse_interval_secs(
        db_state
            .settings_repo
            .get("cloud_sync_interval_sec")
            .ok()
            .flatten(),
    );
    let snapshot_interval_secs = parse_snapshot_interval_secs(
        db_state
            .settings_repo
            .get("cloud_sync_snapshot_interval_min")
            .ok()
            .flatten(),
    );

    let stored_device_id = db_state.settings_repo.get("app.anon_id").ok().flatten();
    let device_id = stored_device_id
        .as_deref()
        .and_then(crate::app::system::normalize_anon_id)
        .unwrap_or_else(
            || crate::app::system::build_anon_id(&crate::app::system::get_machine_id()),
        );
    let should_persist_device_id = stored_device_id
        .as_deref()
        .map(|value| value.trim() != device_id)
        .unwrap_or(true);
    let did_migrate_device_id = stored_device_id
        .as_deref()
        .map(|value| value.trim() != device_id)
        .unwrap_or(false);

    if should_persist_device_id {
        let _ = db_state.settings_repo.set("app.anon_id", &device_id);
    }
    if did_migrate_device_id {
        if let Ok(conn) = db_state.conn.lock() {
            let _ = conn.execute("DELETE FROM cloud_sync_local_index", []);
        }
    }

    let webdav_url = db_state
        .settings_repo
        .get("cloud_sync_webdav_url")
        .ok()
        .flatten()
        .unwrap_or_default()
        .trim()
        .to_string();
    let webdav_username = db_state
        .settings_repo
        .get("cloud_sync_webdav_username")
        .ok()
        .flatten()
        .unwrap_or_default();
    let webdav_password = db_state
        .settings_repo
        .get("cloud_sync_webdav_password")
        .ok()
        .flatten()
        .unwrap_or_default();
    let webdav_base_path = normalize_webdav_base_path(
        &db_state
            .settings_repo
            .get("cloud_sync_webdav_base_path")
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_WEBDAV_BASE_PATH.to_string()),
    );

    let content_prefs = db_state
        .settings_repo
        .get("cloud_sync_content_prefs")
        .ok()
        .flatten()
        .map(|raw| serde_json::from_str::<CloudSyncContentPrefs>(&raw).unwrap_or_default())
        .unwrap_or_default();

    Some(CloudSyncConfig {
        enabled,
        auto_sync,
        provider,
        base_url: base_url.clone(),
        api_key: api_key.clone(),
        device_id,
        interval_secs,
        snapshot_interval_secs,
        cursor,
        webdav_url: if webdav_url.is_empty() {
            base_url.clone()
        } else {
            webdav_url
        },
        webdav_username,
        webdav_password: if webdav_password.trim().is_empty() {
            api_key
        } else {
            webdav_password
        },
        webdav_base_path,
        content_prefs,
    })
}

fn is_cloud_clipboard_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "text" | "code" | "url" | "rich_text" | "image" | "file" | "video" | "emoji_sync"
    )
}

fn is_setting_sync_eligible(key: &str) -> bool {
    !matches!(
        key,
        "app.anon_id"
            | "clipboard_relay_shared_key"
            | "app.emoji_favorites"
            | "app.last_ping_date"
            | "app.window_width"
            | "app.window_height"
            | "app.tag_manager_size"
            | "cloud_sync_enabled"
            | "cloud_sync_auto"
            | "cloud_sync_provider"
            | "cloud_sync_server"
            | "cloud_sync_api_key"
            | "cloud_sync_interval_sec"
            | "cloud_sync_snapshot_interval_min"
            | "cloud_sync_cursor"
            | "cloud_sync_webdav_url"
            | "cloud_sync_webdav_username"
            | "cloud_sync_webdav_password"
            | "cloud_sync_webdav_base_path"
            | "cloud_sync_content_prefs"
            | "cloud_sync_webdav_local_seq"
            | "cloud_sync_webdav_op_cursor_map"
            | "cloud_sync_webdav_blob_cache"
            | "cloud_sync_webdav_last_snapshot_push_at"
            | "cloud_sync_webdav_last_snapshot_pull_at"
            | "cloud_sync_webdav_last_head_rebuild_at"
            | "cloud_sync_settings_applied_at"
    )
}

fn to_data_url_from_path(path: &str) -> Option<String> {
    let file_path = Path::new(path);
    if !file_path.exists() || !file_path.is_file() {
        return None;
    }

    let bytes = std::fs::read(file_path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return None;
    }

    let mime = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    let payload = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{}", mime, payload))
}

fn rewrite_rich_fallback_payload_to_data_url(html: &str) -> String {
    let Some(start) = html.rfind(RICH_IMAGE_FALLBACK_PREFIX) else {
        return html.to_string();
    };
    let marker_start = start + RICH_IMAGE_FALLBACK_PREFIX.len();
    let Some(end_rel) = html[marker_start..].find(RICH_IMAGE_FALLBACK_SUFFIX) else {
        return html.to_string();
    };

    let marker_end = marker_start + end_rel;
    let payload = html[marker_start..marker_end].trim();
    if payload.is_empty()
        || payload.starts_with("data:image/")
        || payload.starts_with("http://asset.localhost/")
        || payload.starts_with("https://asset.localhost/")
    {
        return html.to_string();
    }

    let Some(data_url) = to_data_url_from_path(payload) else {
        return html.to_string();
    };

    format!(
        "{}{}{}",
        &html[..marker_start],
        data_url,
        &html[marker_end..]
    )
}

fn rich_html_resource_path_to_data_url(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty()
        || value.starts_with("data:")
        || value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("//")
        || value.starts_with("asset:")
        || value.starts_with("tauri:")
        || value.starts_with("blob:")
    {
        return None;
    }

    let path_raw = if value.starts_with("file://") {
        value.trim_start_matches("file://")
    } else {
        value
    };

    let path_without_drive_prefix =
        if path_raw.starts_with('/') && path_raw.chars().nth(2) == Some(':') {
            &path_raw[1..]
        } else {
            path_raw
        };

    let decoded_path = decode(path_without_drive_prefix)
        .map(|p| p.into_owned())
        .unwrap_or_else(|_| path_without_drive_prefix.to_string());
    let clean_path = decoded_path
        .split('?')
        .next()
        .unwrap_or(&decoded_path)
        .split('#')
        .next()
        .unwrap_or(&decoded_path)
        .trim();

    if clean_path.is_empty() {
        return None;
    }

    let is_absolute = clean_path.starts_with('/')
        || (clean_path.len() >= 3
            && clean_path.as_bytes()[1] == b':'
            && (clean_path.as_bytes()[2] == b'\\' || clean_path.as_bytes()[2] == b'/'));
    if !is_absolute {
        return None;
    }

    to_data_url_from_path(clean_path)
}

fn rewrite_rich_html_image_sources_to_data_url(html: &str) -> String {
    static IMG_SRC_RE: OnceLock<Regex> = OnceLock::new();
    let re = IMG_SRC_RE
        .get_or_init(|| Regex::new(r#"(?is)(<img\b[^>]*\bsrc=["'])([^"']+)(["'][^>]*>)"#).unwrap());

    re.replace_all(html, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let src = &caps[2];
        let suffix = &caps[3];

        if let Some(data_url) = rich_html_resource_path_to_data_url(src) {
            format!("{}{}{}", prefix, data_url, suffix)
        } else {
            caps[0].to_string()
        }
    })
    .into_owned()
}

fn rewrite_rich_html_resources_for_sync(html: &str) -> String {
    let with_inline_images = rewrite_rich_html_image_sources_to_data_url(html);
    rewrite_rich_fallback_payload_to_data_url(&with_inline_images)
}

fn encode_emoji_favorites_setting(raw: &str) -> Option<String> {
    let paths: Vec<String> = serde_json::from_str(raw).ok()?;
    let encoded: Vec<String> = paths
        .into_iter()
        .filter_map(|path| to_data_url_from_path(path.trim()))
        .collect();
    serde_json::to_string(&encoded).ok()
}

fn emoji_sync_payload_hash(sync_payload: &str) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    sync_payload.hash(&mut hasher);
    hasher.finish() as i64
}

fn merged_emoji_suppression_hash(
    remote_paths: &[String],
    merged_paths: &[String],
    merged_sync_payload: &str,
) -> Option<i64> {
    // Both path lists are materialized, sorted, and deduplicated. Equality is
    // therefore proof that the exact resulting local payload came from remote.
    (remote_paths == merged_paths).then(|| emoji_sync_payload_hash(merged_sync_payload))
}

fn materialize_emoji_favorite_paths(app: &AppHandle, raw: &str) -> AppResult<Vec<String>> {
    let items: Vec<String> = serde_json::from_str(raw)
        .map_err(|e| AppError::Validation(format!("invalid emoji favorites payload: {}", e)))?;
    let data_dir = get_app_data_dir(app)
        .ok_or_else(|| AppError::Internal("App data dir unavailable".to_string()))?;
    let mut saved_paths: Vec<String> = Vec::new();

    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with("data:") {
            let path = Path::new(trimmed);
            if path.is_file() {
                saved_paths.push(trimmed.to_string());
            }
            continue;
        }

        let (mime, bytes) = decode_data_url(trimmed)?;
        if bytes.is_empty() || bytes.len() > MAX_INLINE_IMAGE_BYTES {
            continue;
        }
        let ext = image_ext_from_mime(mime).ok_or_else(|| {
            AppError::Validation(format!("unsupported emoji mime type: {}", mime))
        })?;
        let path = save_emoji_favorite_bytes_to_dir(&data_dir, &bytes, ext)?;
        saved_paths.push(path);
    }

    saved_paths.sort();
    saved_paths.dedup();
    Ok(saved_paths)
}

fn decode_data_url(data_url: &str) -> AppResult<(&str, Vec<u8>)> {
    let Some(header_and_payload) = data_url.strip_prefix("data:") else {
        return Err(AppError::Validation("invalid data url".to_string()));
    };
    let Some((meta, payload)) = header_and_payload.split_once(',') else {
        return Err(AppError::Validation("invalid data url payload".to_string()));
    };
    if !meta.contains(";base64") {
        return Err(AppError::Validation(
            "unsupported data url encoding".to_string(),
        ));
    }
    let mime = meta.split(';').next().unwrap_or("").trim();
    if mime.is_empty() {
        return Err(AppError::Validation("missing mime type".to_string()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| AppError::Validation(format!("invalid base64 payload: {}", e)))?;
    Ok((mime, bytes))
}

fn image_mime_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::Gif => Some("image/gif"),
        image::ImageFormat::WebP => Some("image/webp"),
        image::ImageFormat::Bmp => Some("image/bmp"),
        _ => None,
    }
}

fn image_data_url_from_blob_bytes(bytes: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        let trimmed = text.trim();
        if trimmed.starts_with("data:image/") {
            return Some(trimmed.to_string());
        }
    }

    let mime = image_mime_from_bytes(bytes)?;
    let payload = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{}", mime, payload))
}

fn decode_emoji_favorites_setting(app: &AppHandle, raw: &str) -> AppResult<String> {
    let saved_paths = materialize_emoji_favorite_paths(app, raw)?;
    serde_json::to_string(&saved_paths)
        .map_err(|e| AppError::Internal(format!("serialize emoji favorites failed: {}", e)))
}

fn normalize_item_for_sync(mut item: CloudSyncItem) -> Option<CloudSyncItem> {
    if item.deleted_at > 0 {
        return Some(item);
    }

    if item.content_type == "image" && !item.content.starts_with("data:image/") {
        item.content = to_data_url_from_path(&item.content)?;
    }

    if item.content_type == "rich_text" {
        if let Some(html) = item.html_content.as_ref() {
            item.html_content = Some(rewrite_rich_html_resources_for_sync(html));
        }
    }

    Some(item)
}

fn uses_text_sync_hash(content_type: &str) -> bool {
    uses_text_content_hash(content_type)
}

fn compute_sync_content_hash(content_type: &str, content: &str) -> i64 {
    match content_type {
        "image" => crate::database::calc_image_hash(content).unwrap_or(0),
        content_type if uses_text_sync_hash(content_type) => {
            crate::database::calc_text_hash(content) as i64
        }
        _ => 0,
    }
}

fn compute_legacy_sync_content_hash(content_type: &str, content: &str) -> i64 {
    if uses_text_sync_hash(content_type) {
        calc_legacy_text_hash(content) as i64
    } else {
        compute_sync_content_hash(content_type, content)
    }
}

fn canonicalize_incoming_live_hash(item: &mut CloudSyncItem) {
    if item.deleted_at > 0 {
        return;
    }

    if uses_text_sync_hash(&item.content_type) {
        // A buggy/pre-version peer may have uploaded locally encrypted bytes. Do
        // not relabel those bytes as a v2 plaintext hash unless they are readable.
        let Some(plain_content) = try_decode_persisted_sync_text(&item.content) else {
            return;
        };
        item.content = plain_content;
        item.content_hash = compute_sync_content_hash(&item.content_type, &item.content);
        item.hash_version = HASH_VERSION_WHITESPACE;
    } else if item.content_type == "image"
        && item.hash_version <= HASH_VERSION_LEGACY
        && !item.content.trim().is_empty()
    {
        // Image identity did not change in v2. Blob-backed images reach this point
        // after enrichment, so prefer a hash recomputed from the materialized image.
        // If the image is no longer readable, a supplied legacy hash is still the
        // same identity and can safely participate in v2 lookup.
        let recomputed_hash = compute_sync_content_hash(&item.content_type, &item.content);
        if recomputed_hash != 0 {
            item.content_hash = recomputed_hash;
        }
        if item.content_hash != 0 {
            item.hash_version = HASH_VERSION_WHITESPACE;
        }
    }
}

fn resolved_content_hash(item: &CloudSyncItem) -> i64 {
    if item.content_hash != 0 {
        item.content_hash
    } else {
        compute_sync_content_hash(&item.content_type, &item.content)
    }
}

fn item_updated_at(item: &CloudSyncItem) -> i64 {
    if item.updated_at > 0 {
        item.updated_at
    } else {
        item.timestamp
    }
}

fn item_revision(item: &CloudSyncItem) -> i64 {
    item.deleted_at.max(item_updated_at(item))
}

fn sync_key_for_item(item: &CloudSyncItem) -> Option<String> {
    let hash = resolved_content_hash(item);
    if hash == 0 {
        return None;
    }
    Some(format!(
        "{}:{}:{}",
        item.content_type, item.hash_version, hash
    ))
}

fn sync_digest_for_item(item: &CloudSyncItem) -> String {
    let tags_json = serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".to_string());
    let html_hash = item
        .html_content
        .as_ref()
        .map(|v| crate::database::calc_text_hash(v))
        .unwrap_or(0);
    let preview_hash = crate::database::calc_text_hash(&item.preview);
    let source_hash = crate::database::calc_text_hash(&item.source_app);
    let meta = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        resolved_content_hash(item),
        item.hash_version,
        item.timestamp,
        item.updated_at,
        item.updated_by,
        item.deleted_at,
        item.is_pinned,
        item.pinned_order,
        item.use_count,
        html_hash,
        preview_hash,
        source_hash,
        crate::database::calc_text_hash(&tags_json)
    );
    crate::database::calc_text_hash(&meta).to_string()
}

async fn process_items_blobs_before_push(
    client: &reqwest::Client,
    cfg: &CloudSyncConfig,
    blobs_path: &str,
    blob_cache: &mut HashMap<String, i64>,
    items: &mut [CloudSyncItem],
) -> AppResult<()> {
    for item in items {
        if item.deleted_at > 0 {
            continue;
        }

        if item.content_type == "image" {
            if !item.content.starts_with("data:image/") {
                item.content = to_data_url_from_path(&item.content).ok_or_else(|| {
                    AppError::Internal("convert image path to data url failed".to_string())
                })?;
            }
            if !item.content.is_empty() {
                let relative_hash = sha256_hex(item.content.as_bytes());
                let relative = get_blob_path(blobs_path, BLOB_KIND_IMAGE, &relative_hash);
                let cache_key = blob_cache_storage_key(cfg, &relative);
                if !blob_cache.contains_key(&cache_key) {
                    upload_webdav_blob(
                        client,
                        cfg,
                        blobs_path,
                        BLOB_KIND_IMAGE,
                        item.content.as_bytes(),
                    )
                    .await?;
                }
                blob_cache.insert(cache_key, now_ms());
                let hash = relative_hash;
                item.content_blob_hash = Some(hash);
                item.content = String::new();
            }
        } else {
            let bytes = item.content.as_bytes();
            if bytes.len() > BLOB_THRESHOLD_CONTENT {
                let relative_hash = sha256_hex(bytes);
                let relative = get_blob_path(blobs_path, BLOB_KIND_CONTENT, &relative_hash);
                let cache_key = blob_cache_storage_key(cfg, &relative);
                if !blob_cache.contains_key(&cache_key) {
                    upload_webdav_blob(client, cfg, blobs_path, BLOB_KIND_CONTENT, bytes).await?;
                }
                blob_cache.insert(cache_key, now_ms());
                let hash = relative_hash;
                item.content_blob_hash = Some(hash);
                item.content = String::new();
            }
            if let Some(html) = item.html_content.as_ref() {
                let hbytes = html.as_bytes();
                if hbytes.len() > BLOB_THRESHOLD_HTML {
                    let relative_hash = sha256_hex(hbytes);
                    let relative = get_blob_path(blobs_path, BLOB_KIND_HTML, &relative_hash);
                    let cache_key = blob_cache_storage_key(cfg, &relative);
                    if !blob_cache.contains_key(&cache_key) {
                        upload_webdav_blob(client, cfg, blobs_path, BLOB_KIND_HTML, hbytes).await?;
                    }
                    blob_cache.insert(cache_key, now_ms());
                    let hash = relative_hash;
                    item.html_blob_hash = Some(hash);
                    item.html_content = None;
                }
            }
        }
    }
    Ok(())
}

async fn enrich_item_blobs_after_pull(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    cfg: &CloudSyncConfig,
    blobs_path: &str,
    items: &mut [CloudSyncItem],
) -> AppResult<()> {
    for item in items {
        if let Some(hash) = item.content_blob_hash.as_ref() {
            let kind = if item.content_type == "image" {
                BLOB_KIND_IMAGE
            } else {
                BLOB_KIND_CONTENT
            };
            let bytes = download_webdav_blob(client, cfg, blobs_path, kind, hash).await?;
            if item.content_type == "image" {
                let data_url = image_data_url_from_blob_bytes(&bytes).ok_or_else(|| {
                    AppError::Validation(format!("unsupported image blob payload: {}", hash))
                })?;
                if let Some(data_dir) = get_app_data_dir(app) {
                    if let Some(path) = crate::database::save_image_to_file(&data_url, &data_dir) {
                        item.content = path;
                    } else {
                        item.content = data_url;
                    }
                } else {
                    item.content = data_url;
                }
            } else {
                item.content = String::from_utf8(bytes).unwrap_or_default();
            }
        }

        if let Some(hash) = item.html_blob_hash.as_ref() {
            let bytes = download_webdav_blob(client, cfg, blobs_path, BLOB_KIND_HTML, hash).await?;
            item.html_content = Some(String::from_utf8(bytes).unwrap_or_default());
        }
    }
    Ok(())
}

fn collapse_items_by_sync_key(items: &[CloudSyncItem]) -> BTreeMap<String, CloudSyncItem> {
    let mut map: BTreeMap<String, CloudSyncItem> = BTreeMap::new();
    for item in items {
        let Some(key) = sync_key_for_item(item) else {
            continue;
        };
        let mut normalized = item.clone();
        normalized.content_hash = resolved_content_hash(item);

        let replace = map
            .get(&key)
            .map(|old| {
                (item_revision(&normalized), normalized.updated_by.as_str())
                    >= (item_revision(old), old.updated_by.as_str())
            })
            .unwrap_or(true);
        if replace {
            map.insert(key, normalized);
        }
    }
    map
}

fn load_local_sync_index(app: &AppHandle) -> AppResult<HashMap<String, String>> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let conn = db_state
        .conn
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut stmt = conn
        .prepare("SELECT sync_key, digest FROM cloud_sync_local_index")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut index = HashMap::new();
    for row in rows {
        let (k, v) = row.map_err(|e| AppError::Internal(e.to_string()))?;
        index.insert(k, v);
    }
    Ok(index)
}

fn replace_local_sync_index(
    app: &AppHandle,
    collapsed: &BTreeMap<String, CloudSyncItem>,
) -> AppResult<()> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let mut conn = db_state
        .conn
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let tx = conn
        .transaction()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    tx.execute("DELETE FROM cloud_sync_local_index", [])
        .map_err(|e| AppError::Internal(e.to_string()))?;
    for (sync_key, item) in collapsed {
        let digest = sync_digest_for_item(item);
        tx.execute(
            "INSERT INTO cloud_sync_local_index (sync_key, digest) VALUES (?1, ?2)",
            rusqlite::params![sync_key, digest],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }
    tx.commit().map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

fn try_decode_persisted_sync_text(value: &str) -> Option<String> {
    if value.starts_with(ENCRYPT_PREFIX) {
        encryption::decrypt_value(value).filter(|plain| plain != value)
    } else {
        Some(value.to_string())
    }
}

fn decode_persisted_sync_text(value: String) -> String {
    try_decode_persisted_sync_text(&value).unwrap_or(value)
}

fn encode_persisted_sync_text(value: &str, sensitive: bool) -> AppResult<String> {
    if !sensitive {
        return Ok(value.to_string());
    }
    #[cfg(not(feature = "portable"))]
    {
        encryption::encrypt_value(value).ok_or_else(|| {
            AppError::Internal("failed to encrypt sensitive synced content".to_string())
        })
    }
    #[cfg(feature = "portable")]
    {
        Ok(value.to_string())
    }
}

fn load_persisted_sync_item(conn: &rusqlite::Connection, id: i64) -> AppResult<CloudSyncItem> {
    conn.query_row(
        "SELECT content_type, content, content_hash, html_content, source_app, timestamp,
                sync_updated_at, sync_updated_by, preview, is_pinned, tags, use_count, pinned_order,
                content_hash_version
         FROM clipboard_history WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            let tags_json: String = row.get(10)?;
            Ok(CloudSyncItem {
                content_type: row.get(0)?,
                content: decode_persisted_sync_text(row.get(1)?),
                content_hash: row.get(2)?,
                hash_version: row.get(13)?,
                deleted_at: 0,
                html_content: row
                    .get::<_, Option<String>>(3)?
                    .map(decode_persisted_sync_text),
                content_blob_hash: None,
                html_blob_hash: None,
                source_app: row.get(4)?,
                timestamp: row.get(5)?,
                updated_at: row.get(6)?,
                updated_by: row.get(7)?,
                preview: decode_persisted_sync_text(row.get(8)?),
                is_pinned: row.get(9)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                use_count: row.get(11)?,
                pinned_order: row.get(12)?,
            })
        },
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

fn persisted_tombstone_sync_item(
    content_type: &str,
    content_hash: i64,
    hash_version: i64,
    deleted_at: i64,
) -> CloudSyncItem {
    CloudSyncItem {
        content_type: content_type.to_string(),
        content: String::new(),
        content_hash,
        hash_version,
        deleted_at,
        html_content: None,
        content_blob_hash: None,
        html_blob_hash: None,
        source_app: "sync".to_string(),
        timestamp: deleted_at,
        updated_at: deleted_at,
        updated_by: String::new(),
        preview: String::new(),
        is_pinned: false,
        tags: Vec::new(),
        use_count: 0,
        pinned_order: 0,
    }
}

fn record_remote_sync_digest(conn: &rusqlite::Connection, item: &CloudSyncItem) -> AppResult<()> {
    let mut normalized = item.clone();
    normalized.content_hash = resolved_content_hash(item);
    if normalized.updated_at <= 0 {
        normalized.updated_at = normalized.timestamp;
    }
    let Some(sync_key) = sync_key_for_item(&normalized) else {
        return Ok(());
    };
    let digest = sync_digest_for_item(&normalized);
    conn.execute(
        "INSERT INTO cloud_sync_local_index (sync_key, digest) VALUES (?1, ?2)
         ON CONFLICT(sync_key) DO UPDATE SET digest = excluded.digest",
        rusqlite::params![sync_key, digest],
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

fn collect_local_incremental_items(
    app: &AppHandle,
    local_items: &[CloudSyncItem],
) -> AppResult<(Vec<CloudSyncItem>, BTreeMap<String, CloudSyncItem>)> {
    let collapsed = collapse_items_by_sync_key(local_items);
    let prev_index = load_local_sync_index(app)?;

    let mut deltas = Vec::new();
    for (sync_key, item) in &collapsed {
        let digest = sync_digest_for_item(item);
        let changed = prev_index
            .get(sync_key)
            .map(|old| old != &digest)
            .unwrap_or(true);
        if changed {
            deltas.push(item.clone());
        }
    }

    deltas.sort_by_key(item_revision);
    Ok((deltas, collapsed))
}

fn get_setting_i64(app: &AppHandle, key: &str, default: i64) -> i64 {
    app.try_state::<DbState>()
        .and_then(|db| db.settings_repo.get(key).ok().flatten())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

fn set_setting_i64_result(app: &AppHandle, key: &str, value: i64) -> AppResult<()> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    db_state
        .settings_repo
        .set(key, &value.to_string())
        .map_err(AppError::from)
}

fn set_setting_i64(app: &AppHandle, key: &str, value: i64) {
    let _ = set_setting_i64_result(app, key, value);
}

fn get_local_webdav_op_seq(app: &AppHandle) -> i64 {
    get_setting_i64(app, CLOUD_SYNC_WEBDAV_LOCAL_SEQ_KEY, 0)
}

fn set_local_webdav_op_seq(app: &AppHandle, seq: i64) -> AppResult<()> {
    set_setting_i64_result(app, CLOUD_SYNC_WEBDAV_LOCAL_SEQ_KEY, seq)
}

fn load_webdav_op_cursor_map(app: &AppHandle) -> HashMap<String, i64> {
    let raw = app
        .try_state::<DbState>()
        .and_then(|db| {
            db.settings_repo
                .get(CLOUD_SYNC_WEBDAV_OP_CURSOR_MAP_KEY)
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    if raw.trim().is_empty() {
        return HashMap::new();
    }
    serde_json::from_str::<HashMap<String, i64>>(&raw).unwrap_or_default()
}

fn save_webdav_op_cursor_map(app: &AppHandle, map: &HashMap<String, i64>) {
    if let Some(db_state) = app.try_state::<DbState>() {
        let payload = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string());
        let _ = db_state
            .settings_repo
            .set(CLOUD_SYNC_WEBDAV_OP_CURSOR_MAP_KEY, &payload);
    }
}

fn load_webdav_blob_cache(app: &AppHandle) -> HashMap<String, i64> {
    let raw = app
        .try_state::<DbState>()
        .and_then(|db| {
            db.settings_repo
                .get(CLOUD_SYNC_WEBDAV_BLOB_CACHE_KEY)
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    if raw.trim().is_empty() {
        return HashMap::new();
    }
    serde_json::from_str::<HashMap<String, i64>>(&raw).unwrap_or_default()
}

fn save_webdav_blob_cache(app: &AppHandle, cache: &HashMap<String, i64>) {
    if let Some(db_state) = app.try_state::<DbState>() {
        let mut entries: Vec<(String, i64)> = cache.iter().map(|(k, v)| (k.clone(), *v)).collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        entries.truncate(WEBDAV_BLOB_CACHE_MAX_ENTRIES);
        let payload = serde_json::to_string(&entries.into_iter().collect::<HashMap<_, _>>())
            .unwrap_or_else(|_| "{}".to_string());
        let _ = db_state
            .settings_repo
            .set(CLOUD_SYNC_WEBDAV_BLOB_CACHE_KEY, &payload);
    }
}

fn get_app_data_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let state = app.try_state::<crate::app_state::AppDataDir>()?;
    let guard = state.0.lock().ok()?;
    Some(guard.clone())
}

fn collect_local_syncable_items(
    app: &AppHandle,
    prefs: &CloudSyncContentPrefs,
) -> AppResult<Vec<CloudSyncItem>> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;

    let mut entries: Vec<ClipboardEntry> = Vec::new();
    let mut offset: i32 = 0;

    loop {
        let batch = db_state
            .repo
            .get_history(SYNC_FETCH_PAGE_SIZE, offset, None)
            .map_err(AppError::Internal)?;

        if batch.is_empty() {
            break;
        }

        let fetched = batch.len() as i32;
        entries.extend(batch.into_iter().filter(|e| {
            is_cloud_clipboard_content_type(&e.content_type)
                && prefs.includes_content_type(&e.content_type)
        }));
        offset = offset.saturating_add(fetched);
        if fetched < SYNC_FETCH_PAGE_SIZE {
            break;
        }
    }

    let sync_versions: HashMap<i64, (i64, String, i64, i64)> = {
        let conn = db_state
            .conn
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, sync_updated_at, sync_updated_by, content_hash, content_hash_version
                 FROM clipboard_history",
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut versions = HashMap::new();
        for row in rows {
            let (id, updated_at, updated_by, content_hash, hash_version) =
                row.map_err(|e| AppError::Internal(e.to_string()))?;
            versions.insert(id, (updated_at, updated_by, content_hash, hash_version));
        }
        versions
    };

    let mut items: Vec<CloudSyncItem> = entries
        .into_iter()
        .filter_map(|e| {
            let (updated_at, updated_by, persisted_hash, persisted_hash_version) = sync_versions
                .get(&e.id)
                .cloned()
                .unwrap_or_else(|| (e.timestamp, String::new(), 0, HASH_VERSION_WHITESPACE));
            let normalized = normalize_item_for_sync(CloudSyncItem {
                content_type: e.content_type,
                content: e.content,
                content_hash: 0,
                hash_version: HASH_VERSION_WHITESPACE,
                deleted_at: 0,
                html_content: e.html_content,
                content_blob_hash: None,
                html_blob_hash: None,
                source_app: e.source_app,
                timestamp: e.timestamp,
                updated_at,
                updated_by,
                preview: e.preview,
                is_pinned: e.is_pinned,
                tags: e.tags,
                use_count: e.use_count,
                pinned_order: e.pinned_order,
            })?;
            let mut item = normalized;
            item.content_hash = if persisted_hash != 0 {
                persisted_hash
            } else {
                compute_sync_content_hash(&item.content_type, &item.content)
            };
            item.hash_version = persisted_hash_version;
            Some(item)
        })
        .collect();

    let mut tombstones = collect_local_tombstones(app, prefs)?;
    items.append(&mut tombstones);
    items.sort_by_key(item_revision);
    Ok(items)
}

fn collect_local_changes(
    app: &AppHandle,
    cursor: i64,
    prefs: &CloudSyncContentPrefs,
) -> AppResult<Vec<CloudSyncItem>> {
    let mut items = collect_local_syncable_items(app, prefs)?;
    items.retain(|e| e.timestamp > cursor);
    Ok(items)
}

fn collect_local_tombstones(
    app: &AppHandle,
    prefs: &CloudSyncContentPrefs,
) -> AppResult<Vec<CloudSyncItem>> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let conn = db_state
        .conn
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT content_type, content_hash, hash_version, deleted_at
             FROM cloud_sync_tombstones
             ORDER BY deleted_at ASC",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CloudSyncItem {
                content_type: row.get(0)?,
                content: String::new(),
                content_hash: row.get(1)?,
                hash_version: row.get(2)?,
                deleted_at: row.get(3)?,
                html_content: None,
                content_blob_hash: None,
                html_blob_hash: None,
                source_app: "sync".to_string(),
                timestamp: row.get(3)?,
                updated_at: row.get(3)?,
                updated_by: String::new(),
                preview: String::new(),
                is_pinned: false,
                tags: Vec::new(),
                use_count: 0,
                pinned_order: 0,
            })
        })
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| AppError::Internal(e.to_string()))?);
    }
    items.retain(|item| {
        is_cloud_clipboard_content_type(&item.content_type)
            && prefs.includes_content_type(&item.content_type)
    });
    Ok(items)
}

fn with_sync_entry_savepoint<T, F>(conn: &rusqlite::Connection, operation: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T>,
{
    const SAVEPOINT: &str = "tiez_sync_entry_update";
    conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT};"))
        .map_err(|e| AppError::Internal(e.to_string()))?;

    match operation() {
        Ok(value) => match conn.execute_batch(&format!("RELEASE SAVEPOINT {SAVEPOINT};")) {
            Ok(()) => Ok(value),
            Err(release_error) => {
                let rollback = conn.execute_batch(&format!(
                    "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT};"
                ));
                match rollback {
                    Ok(()) => Err(AppError::Internal(release_error.to_string())),
                    Err(rollback_error) => Err(AppError::Internal(format!(
                        "{release_error}; savepoint rollback also failed: {rollback_error}"
                    ))),
                }
            }
        },
        Err(operation_error) => {
            let rollback = conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT};"
            ));
            match rollback {
                Ok(()) => Err(operation_error),
                Err(rollback_error) => Err(AppError::Internal(format!(
                    "{operation_error}; savepoint rollback also failed: {rollback_error}"
                ))),
            }
        }
    }
}

fn update_existing_entry_from_sync(
    conn: &rusqlite::Connection,
    id: i64,
    item: &CloudSyncItem,
    effective_timestamp: i64,
) -> AppResult<(bool, bool)> {
    let (local_content_raw, local_html_raw, local_content_hash, local_hash_version): (
        String,
        Option<String>,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT content, html_content, content_hash, content_hash_version
             FROM clipboard_history WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let local_content = decode_persisted_sync_text(local_content_raw);
    let local_html = local_html_raw.map(decode_persisted_sync_text);

    let (
        local_timestamp,
        local_is_pinned,
        local_pinned_order,
        local_preview,
        local_source_app,
        local_use_count,
        local_tags_json,
        local_source_app_path,
        local_is_external,
        local_updated_at,
        local_updated_by,
    ): LocalSyncEntryState = conn
        .query_row(
            "SELECT timestamp, is_pinned, pinned_order, preview, source_app, use_count, tags,
                    source_app_path, is_external, sync_updated_at, sync_updated_by
             FROM clipboard_history WHERE id = ?",
            rusqlite::params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7).unwrap_or(None),
                    row.get(8).unwrap_or(false),
                    row.get(9).or_else(|_| row.get(0))?,
                    row.get(10).unwrap_or_default(),
                ))
            },
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let remote_tags_json = serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".to_string());
    let remote_updated_at = item_updated_at(item);
    let remote_version = (remote_updated_at, item.updated_by.as_str());
    let local_version = (local_updated_at, local_updated_by.as_str());
    let local_preview_plain = decode_persisted_sync_text(local_preview.clone());
    let remote_metadata_key = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        effective_timestamp,
        item.is_pinned,
        item.pinned_order,
        item.preview,
        item.source_app,
        item.use_count,
        remote_tags_json,
        item.content,
        item.html_content.as_deref().unwrap_or_default()
    );
    let local_metadata_key = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        local_timestamp,
        local_is_pinned,
        local_pinned_order,
        local_preview_plain,
        local_source_app,
        local_use_count,
        local_tags_json,
        local_content,
        local_html.as_deref().unwrap_or_default()
    );
    let remote_wins = remote_version > local_version
        || (remote_version == local_version && remote_metadata_key > local_metadata_key);
    let remote_accepted = remote_version > local_version
        || (remote_version == local_version && remote_metadata_key >= local_metadata_key);

    let mut changed = false;
    let mut timestamp = local_timestamp;
    let mut is_pinned = local_is_pinned;
    let mut pinned_order = local_pinned_order;
    let mut preview = local_preview;
    let mut source_app = local_source_app;
    let mut use_count = local_use_count;
    let mut tags_json = local_tags_json.clone();
    let mut source_app_path = local_source_app_path;
    let mut is_external = local_is_external;
    let mut updated_at = local_updated_at;
    let mut updated_by = local_updated_by;
    let mut content = None;
    let mut html_content = None;
    let mut content_hash = local_content_hash;
    let mut hash_version = local_hash_version;

    if remote_wins {
        let sensitive = has_sensitive_tag(&item.tags);
        timestamp = effective_timestamp;
        is_pinned = item.is_pinned;
        pinned_order = item.pinned_order;
        let winning_preview = if item.preview.is_empty() {
            local_preview_plain.as_str()
        } else {
            item.preview.as_str()
        };
        preview = encode_persisted_sync_text(winning_preview, sensitive)?;
        content = Some(encode_persisted_sync_text(&item.content, sensitive)?);
        html_content = Some(
            item.html_content
                .as_deref()
                .map(|html| encode_persisted_sync_text(html, sensitive))
                .transpose()?,
        );
        content_hash = resolved_content_hash(item);
        hash_version = item.hash_version;
        if item.source_app != "sync" && !item.source_app.is_empty() {
            source_app = item.source_app.clone();
            source_app_path = None;
        }
        tags_json = remote_tags_json;
        is_external = item.content_type == "image"
            || item.content_type == "file"
            || item.content_type == "video";
        updated_at = remote_updated_at;
        updated_by = item.updated_by.clone();
        changed = true;
    }
    if item.use_count > use_count {
        use_count = item.use_count;
        changed = true;
    }

    if changed {
        with_sync_entry_savepoint(conn, || {
            conn.execute(
                "UPDATE clipboard_history SET
                    timestamp = ?1,
                    is_pinned = ?2,
                    pinned_order = ?3,
                    preview = ?4,
                    source_app = ?5,
                    use_count = ?6,
                    tags = ?7,
                    source_app_path = ?8,
                    is_external = ?9,
                    sync_updated_at = ?10,
                    sync_updated_by = ?11,
                    content = COALESCE(?12, content),
                    html_content = CASE WHEN ?12 IS NULL THEN html_content ELSE ?13 END,
                    content_hash = ?14,
                    content_hash_version = ?15
                 WHERE id = ?16",
                rusqlite::params![
                    timestamp,
                    is_pinned,
                    pinned_order,
                    preview,
                    source_app,
                    use_count,
                    tags_json,
                    source_app_path,
                    if is_external { 1 } else { 0 },
                    updated_at,
                    updated_by,
                    content,
                    html_content,
                    content_hash,
                    hash_version,
                    id
                ],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;

            if remote_wins {
                conn.execute(
                    "DELETE FROM entry_tags WHERE entry_id = ?",
                    rusqlite::params![id],
                )
                .map_err(|e| AppError::Internal(e.to_string()))?;
                for tag in &item.tags {
                    conn.execute(
                        "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?, ?)",
                        rusqlite::params![id, tag],
                    )
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                }
            }
            Ok(())
        })?;
    }

    Ok((changed, remote_accepted))
}

fn legacy_hash_matches_content(content_type: &str, content: &str, hash: i64) -> bool {
    compute_legacy_sync_content_hash(content_type, content) == hash
}

fn remote_tombstone_matches_stored_entry(
    content_type: &str,
    remote_hash: i64,
    remote_hash_version: i64,
    stored_content: &str,
    stored_hash: i64,
    stored_hash_version: i64,
) -> bool {
    if remote_hash_version <= HASH_VERSION_LEGACY {
        // Version-1/unversioned tombstones are ambiguous: historical peers
        // produced both trim-equivalent hashes and exact pre-version hashes.
        stored_hash == remote_hash
            || try_decode_persisted_sync_text(stored_content)
                .is_some_and(|plain| legacy_hash_matches_content(content_type, &plain, remote_hash))
    } else {
        // Never apply a v2 deletion to a legacy row merely because the numeric
        // hash happens to collide; both sides must use whitespace semantics.
        stored_hash_version >= HASH_VERSION_WHITESPACE && stored_hash == remote_hash
    }
}

fn matching_tombstone_deleted_at(
    conn: &rusqlite::Connection,
    item: &CloudSyncItem,
) -> AppResult<i64> {
    let exact_hash = resolved_content_hash(item);
    let legacy_hash = compute_legacy_sync_content_hash(&item.content_type, &item.content);
    conn.query_row(
        "SELECT COALESCE(MAX(deleted_at), 0) FROM cloud_sync_tombstones
         WHERE content_type = ?1
           AND ((hash_version <= 1 AND content_hash IN (?2, ?3))
             OR (?4 >= 2 AND hash_version >= 2 AND content_hash = ?3))",
        rusqlite::params![
            item.content_type,
            legacy_hash,
            exact_hash,
            item.hash_version
        ],
        |row| row.get(0),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

fn clear_matching_tombstones(conn: &rusqlite::Connection, item: &CloudSyncItem) -> AppResult<()> {
    let exact_hash = resolved_content_hash(item);
    let legacy_hash = compute_legacy_sync_content_hash(&item.content_type, &item.content);
    conn.execute(
        "DELETE FROM cloud_sync_tombstones
         WHERE content_type = ?1 AND deleted_at <= ?4
           AND ((hash_version <= 1 AND content_hash IN (?2, ?3))
             OR (?5 >= 2 AND hash_version >= 2 AND content_hash = ?3))",
        rusqlite::params![
            item.content_type,
            legacy_hash,
            exact_hash,
            item_updated_at(item),
            item.hash_version
        ],
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

fn apply_remote_tombstone_with_conn(
    repo: &crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository,
    conn: &rusqlite::Connection,
    item: &CloudSyncItem,
    data_dir: Option<&Path>,
) -> AppResult<usize> {
    let remote_hash = resolved_content_hash(item);
    if remote_hash == 0 || item.deleted_at <= 0 {
        return Ok(0);
    }

    let may_cleanup_after_savepoint = conn.is_autocommit();
    let (applied, cleanup_paths) = with_sync_entry_savepoint(conn, || {
        let tombstone_ts = item.deleted_at;
        let hash_version = item.hash_version.max(HASH_VERSION_LEGACY);
        conn.execute(
            "INSERT INTO cloud_sync_tombstones
                (content_type, content_hash, hash_version, deleted_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(content_type, content_hash, hash_version)
             DO UPDATE SET deleted_at = MAX(cloud_sync_tombstones.deleted_at, excluded.deleted_at)",
            rusqlite::params![item.content_type, remote_hash, hash_version, tombstone_ts],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
        let persisted_tombstone_ts = conn
            .query_row(
                "SELECT deleted_at FROM cloud_sync_tombstones
                 WHERE content_type = ?1 AND content_hash = ?2 AND hash_version = ?3",
                rusqlite::params![item.content_type, remote_hash, hash_version],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let entries = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, sync_updated_at, sync_updated_by, content, content_hash,
                            content_hash_version, html_content, is_external
                     FROM clipboard_history WHERE content_type = ?1",
                )
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params![item.content_type], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, i32>(7)? == 1,
                    ))
                })
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let mut entries = Vec::new();
            for row in rows {
                let entry = row.map_err(|e| AppError::Internal(e.to_string()))?;
                if remote_tombstone_matches_stored_entry(
                    &item.content_type,
                    remote_hash,
                    hash_version,
                    &entry.3,
                    entry.4,
                    entry.5,
                ) {
                    entries.push(entry);
                }
            }
            entries
        };

        let mut applied = 0;
        let mut deletion_accepted = true;
        let mut cleanup_paths = HashSet::new();
        for (id, local_updated_at, local_updated_by, content, _, _, html, is_external) in entries {
            if (tombstone_ts, item.updated_by.as_str())
                < (local_updated_at, local_updated_by.as_str())
            {
                deletion_accepted = false;
                continue;
            }

            if let Some(dir) = data_dir {
                cleanup_paths.extend(repo.collect_attachment_paths_for_cleanup(
                    &content,
                    html.as_deref(),
                    is_external,
                    &dir.join("attachments"),
                ));
            }
            // A remote deletion already has its authoritative tombstone. Remove only
            // the local row metadata so no newer local-time/v2 echo is manufactured.
            // The surrounding sync savepoint owns both metadata statements.
            repo.delete_metadata_with_conn(conn, id)
                .map_err(AppError::Internal)?;
            applied += 1;
        }

        if deletion_accepted && persisted_tombstone_ts == tombstone_ts {
            let persisted = persisted_tombstone_sync_item(
                &item.content_type,
                remote_hash,
                hash_version,
                persisted_tombstone_ts,
            );
            record_remote_sync_digest(conn, &persisted)?;
        }

        Ok((applied, cleanup_paths))
    })?;

    if may_cleanup_after_savepoint {
        if let Some(dir) = data_dir {
            repo.cleanup_unreferenced_attachment_paths_with_conn(
                conn,
                cleanup_paths,
                &dir.join("attachments"),
            );
        }
    }

    Ok(applied)
}

fn find_existing_entry_for_sync(
    conn: &rusqlite::Connection,
    item: &CloudSyncItem,
) -> AppResult<Option<i64>> {
    let remote_hash = resolved_content_hash(item);
    if remote_hash != 0 {
        return Ok(conn
            .query_row(
                "SELECT id FROM clipboard_history
                 WHERE content_type = ?1 AND content_hash = ?2
                   AND ((?3 >= 2 AND content_hash_version >= 2)
                     OR (?3 <= 1 AND content_hash_version <= 1))
                 ORDER BY id DESC LIMIT 1",
                rusqlite::params![item.content_type, remote_hash, item.hash_version],
                |row| row.get::<_, i64>(0),
            )
            .ok());
    }

    // Hashless payloads cannot participate in versioned lookup. Exact stored
    // content is the only safe fallback (principally for unsupported images).
    Ok(conn
        .query_row(
            "SELECT id FROM clipboard_history
             WHERE content_type = ?1 AND content = ?2
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![item.content_type, item.content],
            |row| row.get::<_, i64>(0),
        )
        .ok())
}

fn apply_remote_emoji_item<F>(
    item: &CloudSyncItem,
    prefs: &CloudSyncContentPrefs,
    merge: F,
) -> AppResult<bool>
where
    F: FnOnce(&str) -> AppResult<()>,
{
    if item.content_type != "emoji_sync" || !prefs.emoji {
        return Ok(false);
    }
    merge(&item.content)?;
    Ok(true)
}

fn apply_remote_live_item_with_conn(
    repo: &crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository,
    conn: &rusqlite::Connection,
    item: &CloudSyncItem,
    effective_timestamp: i64,
    data_dir: Option<&Path>,
) -> AppResult<usize> {
    with_sync_entry_savepoint(conn, || {
        let remote_hash = resolved_content_hash(item);
        if remote_hash != 0 && matching_tombstone_deleted_at(conn, item)? >= item_updated_at(item) {
            return Ok(0);
        }

        if let Some(id) = find_existing_entry_for_sync(conn, item)? {
            let (changed, remote_accepted) =
                update_existing_entry_from_sync(conn, id, item, effective_timestamp)?;
            if remote_hash != 0 {
                clear_matching_tombstones(conn, item)?;
            }
            if remote_accepted {
                let persisted = load_persisted_sync_item(conn, id)?;
                record_remote_sync_digest(conn, &persisted)?;
            }
            return Ok(usize::from(changed));
        }

        let preview = if item.preview.trim().is_empty() {
            if item.content_type == "image" {
                "[Image Content]".to_string()
            } else {
                item.content.chars().take(200).collect::<String>()
            }
        } else {
            item.preview.clone()
        };

        let entry = ClipboardEntry {
            id: 0,
            content_type: item.content_type.clone(),
            content: item.content.clone(),
            html_content: item.html_content.clone(),
            source_app: item.source_app.clone(),
            source_app_path: None,
            timestamp: effective_timestamp,
            preview,
            is_pinned: item.is_pinned,
            tags: item.tags.clone(),
            use_count: item.use_count,
            is_external: item.content_type == "image"
                || item.content_type == "file"
                || item.content_type == "video",
            pinned_order: item.pinned_order,
            file_preview_exists: true,
        };

        let inserted_id = repo
            .save_with_conn(conn, &entry, data_dir)
            .map_err(AppError::Internal)?;
        let affected = conn
            .execute(
                "UPDATE clipboard_history
                 SET sync_updated_at = ?1,
                     sync_updated_by = ?2,
                     content_hash = CASE WHEN ?3 != 0 THEN ?3 ELSE content_hash END,
                     content_hash_version = ?4
                 WHERE id = ?5",
                rusqlite::params![
                    item_updated_at(item),
                    item.updated_by,
                    remote_hash,
                    item.hash_version,
                    inserted_id
                ],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if affected != 1 {
            return Err(AppError::Internal(format!(
                "new synced clipboard entry {inserted_id} disappeared before metadata update"
            )));
        }
        if remote_hash != 0 {
            clear_matching_tombstones(conn, item)?;
        }
        let persisted = load_persisted_sync_item(conn, inserted_id)?;
        record_remote_sync_digest(conn, &persisted)?;
        Ok(1)
    })
}

fn apply_remote_changes(
    app: &AppHandle,
    remote_items: &[CloudSyncItem],
    prefs: &CloudSyncContentPrefs,
) -> AppResult<usize> {
    if remote_items.is_empty() {
        return Ok(0);
    }

    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let mut applied = 0usize;
    let app_data_dir = get_app_data_dir(app);
    for remote_item in remote_items {
        let mut normalized_item = remote_item.clone();
        canonicalize_incoming_live_hash(&mut normalized_item);
        let item = &normalized_item;
        if item.content_type == "emoji_sync" {
            if apply_remote_emoji_item(item, prefs, |content| merge_remote_emojis(app, content))? {
                applied += 1;
            }
            continue;
        }

        if !is_cloud_clipboard_content_type(&item.content_type) {
            continue;
        }
        if !prefs.includes_content_type(&item.content_type) {
            continue;
        }

        let conn = db_state
            .conn
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let effective_timestamp = if item.timestamp > 0 {
            item.timestamp
        } else {
            now_ms()
        };

        if item.deleted_at > 0 {
            applied += apply_remote_tombstone_with_conn(
                &db_state.repo,
                &conn,
                item,
                app_data_dir.as_deref(),
            )?;
            continue;
        }

        if item.content.trim().is_empty() {
            continue;
        }

        applied += apply_remote_live_item_with_conn(
            &db_state.repo,
            &conn,
            item,
            effective_timestamp,
            app_data_dir.as_deref(),
        )?;
    }

    Ok(applied)
}

fn cloud_sync_target_ready(cfg: &CloudSyncConfig) -> bool {
    match cfg.provider {
        CloudSyncProvider::Http => !cfg.base_url.trim().is_empty(),
        CloudSyncProvider::WebDav => !cfg.webdav_url.trim().is_empty(),
    }
}

fn cloud_sync_target_not_ready_message(cfg: &CloudSyncConfig) -> String {
    match cfg.provider {
        CloudSyncProvider::Http => "cloud_sync_server is empty".to_string(),
        CloudSyncProvider::WebDav => "cloud_sync_webdav_url is empty".to_string(),
    }
}

fn build_http_client() -> AppResult<Client> {
    Client::builder()
        .timeout(Duration::from_secs(WEBDAV_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::Network(e.to_string()))
}

fn webdav_retry_delay(attempt: usize) -> Duration {
    let factor = 1u64 << attempt.min(4);
    Duration::from_millis(WEBDAV_RETRY_BASE_DELAY_MS.saturating_mul(factor))
}

fn is_retryable_webdav_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn check_webdav_status_for_backoff(status: StatusCode) {
    if matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
    ) {
        // 进入 5 分钟冷却期，避免激怒坚果云导致封禁时间被无限延长
        let cooldown = now_ms() + 300 * 1000;
        CLOUD_SYNC_BACKOFF_UNTIL.store(cooldown, Ordering::Relaxed);
    }
}

async fn webdav_send_with_retry<F>(mut build_request: F) -> AppResult<Response>
where
    F: FnMut() -> RequestBuilder,
{
    let mut last_error = None;

    for attempt in 0..=WEBDAV_MAX_RETRIES {
        match build_request().send().await {
            Ok(resp) => {
                check_webdav_status_for_backoff(resp.status());
                if is_retryable_webdav_status(resp.status()) && attempt < WEBDAV_MAX_RETRIES {
                    last_error = Some(format!("transient WebDAV status {}", resp.status()));
                    sleep(webdav_retry_delay(attempt)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(err) => {
                last_error = Some(err.to_string());
                if attempt < WEBDAV_MAX_RETRIES {
                    sleep(webdav_retry_delay(attempt)).await;
                    continue;
                }
            }
        }
    }

    Err(AppError::Network(
        last_error.unwrap_or_else(|| "webdav request failed".to_string()),
    ))
}

fn webdav_with_auth(req: RequestBuilder, cfg: &CloudSyncConfig) -> RequestBuilder {
    if cfg.webdav_username.trim().is_empty() {
        req
    } else {
        req.basic_auth(cfg.webdav_username.trim(), Some(cfg.webdav_password.trim()))
    }
}

fn encode_webdav_relative_path(relative_path: &str, collection: bool) -> String {
    let mut encoded = relative_path
        .replace('\\', "/")
        .split('/')
        .filter_map(|segment| {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(urlencoding::encode(trimmed).into_owned())
            }
        })
        .collect::<Vec<_>>()
        .join("/");

    if collection && !encoded.is_empty() {
        encoded.push('/');
    }

    encoded
}

fn webdav_resource_url_for(cfg: &CloudSyncConfig, relative_path: &str) -> String {
    let encoded = encode_webdav_relative_path(relative_path, false);
    if encoded.is_empty() {
        cfg.webdav_url.trim_end_matches('/').to_string()
    } else {
        format!("{}/{}", cfg.webdav_url.trim_end_matches('/'), encoded)
    }
}

fn webdav_collection_url_for(cfg: &CloudSyncConfig, relative_path: &str) -> String {
    let encoded = encode_webdav_relative_path(relative_path, true);
    if encoded.is_empty() {
        format!("{}/", cfg.webdav_url.trim_end_matches('/'))
    } else {
        format!("{}/{}", cfg.webdav_url.trim_end_matches('/'), encoded)
    }
}

fn webdav_url_for(cfg: &CloudSyncConfig, relative_path: &str) -> String {
    webdav_resource_url_for(cfg, relative_path)
}

async fn webdav_collection_exists(
    client: &Client,
    cfg: &CloudSyncConfig,
    relative_path: &str,
) -> AppResult<bool> {
    let method = Method::from_bytes(b"PROPFIND")
        .map_err(|e| AppError::Internal(format!("invalid PROPFIND method: {}", e)))?;
    let url = webdav_collection_url_for(cfg, relative_path);
    let resp = webdav_send_with_retry(|| {
        webdav_with_auth(
            client
                .request(method.clone(), &url)
                .header("Depth", "0")
                .header("Content-Type", "application/xml; charset=utf-8"),
            cfg,
        )
    })
    .await?;

    Ok(resp.status().is_success() || resp.status().as_u16() == 207)
}

async fn mkcol_if_needed(
    client: &Client,
    cfg: &CloudSyncConfig,
    relative_path: &str,
) -> AppResult<()> {
    // 1. 生成唯一的缓存 Key（URL + 相对路径）
    let cache_key = format!("{}:{}", cfg.webdav_url, relative_path);
    // 2. 检查缓存中是否已经记录过该目录
    {
        let cache = WEBDAV_KNOWN_DIRS.get_or_init(|| Mutex::new(HashSet::new()));
        if cache.lock().unwrap().contains(&cache_key) {
            // 如果已在缓存中，直接返回成功，不产生任何网络请求
            return Ok(());
        }
    }

    let method = Method::from_bytes(b"MKCOL")
        .map_err(|e| AppError::Internal(format!("invalid MKCOL method: {}", e)))?;
    let url = webdav_collection_url_for(cfg, relative_path);
    let resp =
        webdav_send_with_retry(|| webdav_with_auth(client.request(method.clone(), &url), cfg))
            .await?;

    let code = resp.status().as_u16();
    if resp.status().is_success() {
        // 创建成功，写入缓存
        let cache = WEBDAV_KNOWN_DIRS.get_or_init(|| Mutex::new(HashSet::new()));
        cache.lock().unwrap().insert(cache_key);
        return Ok(());
    }

    if matches!(code, 301 | 302 | 307 | 308 | 405 | 409)
        && webdav_collection_exists(client, cfg, relative_path).await?
    {
        // 如果服务器反馈目录已存在 (405) 或者发生冲突 (409)，同样记录到缓存中
        let cache = WEBDAV_KNOWN_DIRS.get_or_init(|| Mutex::new(HashSet::new()));
        cache.lock().unwrap().insert(cache_key);
        return Ok(());
    }

    let text = resp.text().await.unwrap_or_default();
    Err(AppError::Network(format!(
        "webdav MKCOL failed for {}: {} {}",
        url, code, text
    )))
}

async fn delete_webdav_resource_if_exists(
    client: &Client,
    cfg: &CloudSyncConfig,
    relative_path: &str,
) -> AppResult<()> {
    let url = webdav_url_for(cfg, relative_path);
    let resp = webdav_send_with_retry(|| webdav_with_auth(client.delete(&url), cfg)).await?;

    if resp.status().is_success() || resp.status().as_u16() == 404 {
        return Ok(());
    }

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Err(AppError::Network(format!(
        "webdav DELETE cleanup failed for {}: {} {}",
        url, status, text
    )))
}

async fn move_webdav_resource(
    client: &Client,
    cfg: &CloudSyncConfig,
    from_relative: &str,
    to_relative: &str,
) -> AppResult<bool> {
    let from_url = webdav_url_for(cfg, from_relative);
    let destination = webdav_url_for(cfg, to_relative);
    let resp = webdav_send_with_retry(|| {
        let method = Method::from_bytes(b"MOVE").expect("MOVE is a valid HTTP method");
        webdav_with_auth(
            client
                .request(method, &from_url)
                .header("Destination", destination.clone())
                .header("Overwrite", "T"),
            cfg,
        )
    })
    .await?;

    if resp.status().is_success() {
        return Ok(true);
    }

    if matches!(resp.status().as_u16(), 405 | 409 | 412 | 501) {
        return Ok(false);
    }

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Err(AppError::Network(format!(
        "webdav MOVE publish failed for {} -> {}: {} {}",
        from_url, destination, status, text
    )))
}

async fn upload_webdav_bytes_resource(
    client: &Client,
    cfg: &CloudSyncConfig,
    relative_path: &str,
    body: Vec<u8>,
    content_type: &str,
    label: &str,
) -> AppResult<()> {
    async fn upload_target(
        client: &Client,
        cfg: &CloudSyncConfig,
        url: &str,
        payload: &[u8],
        content_type: &str,
        label: &str,
    ) -> AppResult<()> {
        let url_owned = url.to_string();
        let payload_owned = payload.to_vec();
        let content_type_owned = content_type.to_string();

        let resp = webdav_send_with_retry(|| {
            webdav_with_auth(
                client
                    .put(&url_owned)
                    .header("Content-Type", &content_type_owned)
                    .body(payload_owned.clone()),
                cfg,
            )
        })
        .await?;

        if resp.status().is_success() {
            return Ok(());
        }

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(AppError::Network(format!(
            "webdav PUT {} failed: {} {}",
            label, status, text
        )))
    }

    let final_url = webdav_url_for(cfg, relative_path);
    let temp_relative = format!(
        "{}.uploading.{}.{}.tmp",
        relative_path.trim_end_matches('/'),
        cfg.device_id,
        now_ms()
    );
    let temp_url = webdav_url_for(cfg, &temp_relative);

    upload_target(client, cfg, &temp_url, &body, content_type, label).await?;

    match move_webdav_resource(client, cfg, &temp_relative, relative_path).await {
        Ok(true) => Ok(()),
        Ok(false) => {
            let fallback = upload_target(client, cfg, &final_url, &body, content_type, label).await;
            let _ = delete_webdav_resource_if_exists(client, cfg, &temp_relative).await;
            fallback
        }
        Err(err) => {
            let _ = delete_webdav_resource_if_exists(client, cfg, &temp_relative).await;
            Err(err)
        }
    }
}

async fn upload_webdav_json_resource(
    client: &Client,
    cfg: &CloudSyncConfig,
    relative_path: &str,
    body: Vec<u8>,
    label: &str,
) -> AppResult<()> {
    upload_webdav_bytes_resource(client, cfg, relative_path, body, "application/json", label).await
}

async fn fetch_webdav_json_resource<T, F>(
    mut make_request: F,
    missing_status: u16,
    fetch_error_label: &str,
    parse_error_label: &str,
) -> AppResult<Option<T>>
where
    T: for<'de> Deserialize<'de>,
    F: FnMut() -> RequestBuilder,
{
    for attempt in 0..=WEBDAV_JSON_READ_RETRIES {
        let resp = webdav_send_with_retry(|| make_request()).await?;

        let status_code = resp.status().as_u16();
        if status_code == missing_status {
            return Ok(None);
        }

        // 兼容坚果云：如果父目录不存在，GET 可能返回 409 Conflict (AncestorsNotFound)
        if status_code == 409 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Network(format!(
                "{}: {} {}",
                fetch_error_label, status, text
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?;
        match serde_json::from_slice::<T>(&bytes) {
            Ok(parsed) => return Ok(Some(parsed)),
            Err(err)
                if matches!(err.classify(), serde_json::error::Category::Eof)
                    && attempt < WEBDAV_JSON_READ_RETRIES =>
            {
                sleep(webdav_retry_delay(attempt)).await;
            }
            Err(err) => return Err(AppError::Network(format!("{}: {}", parse_error_label, err))),
        }
    }

    Err(AppError::Network(format!(
        "{}: exhausted retries",
        parse_error_label
    )))
}

async fn ensure_webdav_directories(
    client: &Client,
    cfg: &CloudSyncConfig,
) -> AppResult<WebDavPaths> {
    let base = normalize_webdav_base_path(&cfg.webdav_base_path);
    let mut current = String::new();

    let paths = WebDavPaths {
        devices_path: if base.is_empty() {
            "devices".into()
        } else {
            format!("{}/devices", base)
        },
        settings_path: if base.is_empty() {
            "settings".into()
        } else {
            format!("{}/settings", base)
        },
        ops_path: if base.is_empty() {
            "ops".into()
        } else {
            format!("{}/ops", base)
        },
        head_path: if base.is_empty() {
            WEBDAV_HEAD_FILENAME.into()
        } else {
            format!("{}/{}", base, WEBDAV_HEAD_FILENAME)
        },
        blobs_path: if base.is_empty() {
            "blobs".into()
        } else {
            format!("{}/blobs", base)
        },
    };

    // 注意：不再使用全局静态标识 WEBDAV_ROOT_INITIALIZED 来跳过初始化，
    // 因为这会导致在切换 WebDAV 配置时无法正确为新地址创建目录。
    // 性能优化现在完全依赖 WEBDAV_KNOWN_DIRS 缓存。

    for segment in base.split('/').filter(|s| !s.is_empty()) {
        current = if current.is_empty() {
            segment.to_string()
        } else {
            format!("{}/{}", current, segment)
        };
        mkcol_if_needed(client, cfg, &current).await?;
    }

    mkcol_if_needed(client, cfg, &paths.devices_path).await?;
    mkcol_if_needed(client, cfg, &paths.settings_path).await?;
    mkcol_if_needed(client, cfg, &paths.ops_path).await?;
    mkcol_if_needed(client, cfg, &paths.blobs_path).await?;

    Ok(paths)
}

async fn upload_webdav_blob(
    client: &Client,
    cfg: &CloudSyncConfig,
    base_blobs: &str,
    kind: &str,
    data: &[u8],
) -> AppResult<String> {
    let hash = sha256_hex(data);
    let prefix = if hash.len() >= 2 { &hash[0..2] } else { "xx" };
    let prefix_path = format!("{}/{}", base_blobs, prefix);
    mkcol_if_needed(client, cfg, &prefix_path).await?;

    let blob_file_path = get_blob_path(base_blobs, kind, &hash);
    upload_webdav_bytes_resource(
        client,
        cfg,
        &blob_file_path,
        data.to_vec(),
        "application/octet-stream",
        "blob",
    )
    .await?;
    Ok(hash)
}

async fn download_webdav_blob(
    client: &Client,
    cfg: &CloudSyncConfig,
    base_blobs: &str,
    kind: &str,
    hash: &str,
) -> AppResult<Vec<u8>> {
    let blob_file_path = get_blob_path(base_blobs, kind, hash);
    let url = webdav_url_for(cfg, &blob_file_path);
    let resp = webdav_with_auth(client.get(&url), cfg)
        .send()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "webdav GET blob failed: {} ({})",
            resp.status(),
            blob_file_path
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
    Ok(bytes.to_vec())
}

fn parse_webdav_snapshot_ids(xml: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r"(?is)<[^>]*href[^>]*>\s*([^<]+)\s*</[^>]*href>") else {
        return Vec::new();
    };

    let mut ids = Vec::new();
    for caps in re.captures_iter(xml) {
        let Some(raw_match) = caps.get(1) else {
            continue;
        };
        let raw_href = raw_match.as_str().trim();
        if raw_href.is_empty() {
            continue;
        }

        let decoded_href = urlencoding::decode(raw_href)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| raw_href.to_string());

        let normalized = decoded_href.trim_end_matches('/');
        let Some(file_name) = normalized.rsplit('/').next() else {
            continue;
        };

        let Some(device_id) = file_name.strip_suffix(".json") else {
            continue;
        };
        if device_id.is_empty() {
            continue;
        }
        if ids.iter().any(|existing| existing == device_id) {
            continue;
        }
        ids.push(device_id.to_string());
    }
    ids
}

async fn upload_webdav_snapshot(
    client: &Client,
    cfg: &CloudSyncConfig,
    devices_path: &str,
    latest_op_seq: i64,
    local_items: &[CloudSyncItem],
) -> AppResult<()> {
    let snapshot = WebDavDeviceSnapshot {
        device_id: cfg.device_id.clone(),
        updated_at: now_ms(),
        latest_op_seq,
        entries: local_items.to_vec(),
    };
    let body = serde_json::to_vec(&snapshot)
        .map_err(|e| AppError::Internal(format!("serialize snapshot failed: {}", e)))?;

    let relative = format!(
        "{}/{}.json",
        devices_path.trim_end_matches('/'),
        cfg.device_id
    );
    upload_webdav_json_resource(client, cfg, &relative, body, "snapshot").await
}

async fn list_webdav_snapshot_ids(
    client: &Client,
    cfg: &CloudSyncConfig,
    devices_path: &str,
) -> AppResult<Vec<String>> {
    let method = Method::from_bytes(b"PROPFIND")
        .map_err(|e| AppError::Internal(format!("invalid PROPFIND method: {}", e)))?;
    let url = webdav_collection_url_for(cfg, devices_path);
    let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getlastmodified />
  </d:prop>
</d:propfind>"#;

    let resp = webdav_send_with_retry(|| {
        webdav_with_auth(
            client
                .request(method.clone(), &url)
                .header("Depth", "1")
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(body.to_string()),
            cfg,
        )
    })
    .await?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 207 {
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Network(format!(
            "webdav PROPFIND failed: {} {}",
            status, text
        )));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
    Ok(parse_webdav_snapshot_ids(&text))
}

async fn fetch_webdav_snapshot(
    client: &Client,
    cfg: &CloudSyncConfig,
    devices_path: &str,
    device_id: &str,
) -> AppResult<Option<WebDavDeviceSnapshot>> {
    let relative = format!("{}/{}.json", devices_path.trim_end_matches('/'), device_id);
    let url = webdav_url_for(cfg, &relative);
    fetch_webdav_json_resource(
        || webdav_with_auth(client.get(&url), cfg),
        404,
        "webdav GET snapshot failed",
        "parse snapshot json failed",
    )
    .await
}

fn webdav_ops_filename(device_id: &str, seq: i64) -> String {
    format!("{}__{:020}.json", device_id, seq.max(0))
}

async fn upload_webdav_ops_batch(
    client: &Client,
    cfg: &CloudSyncConfig,
    ops_path: &str,
    seq: i64,
    entries: &[CloudSyncItem],
) -> AppResult<()> {
    let batch = WebDavOpsBatch {
        device_id: cfg.device_id.clone(),
        seq,
        updated_at: now_ms(),
        entries: entries.to_vec(),
    };
    let body = serde_json::to_vec(&batch)
        .map_err(|e| AppError::Internal(format!("serialize ops batch failed: {}", e)))?;
    let relative = format!(
        "{}/{}",
        ops_path.trim_end_matches('/'),
        webdav_ops_filename(&cfg.device_id, seq)
    );
    upload_webdav_json_resource(client, cfg, &relative, body, "ops batch").await
}

fn parse_webdav_op_refs(xml: &str) -> Vec<WebDavOpRef> {
    let Ok(re_href) = Regex::new(r"(?is)<[^>]*href[^>]*>\s*([^<]+)\s*</[^>]*href>") else {
        return Vec::new();
    };
    let Ok(re_file) = Regex::new(r"^(.+)__(\d+)\.json$") else {
        return Vec::new();
    };

    let mut refs: HashMap<String, WebDavOpRef> = HashMap::new();
    let _start = std::time::Instant::now();
    for caps in re_href.captures_iter(xml) {
        let Some(raw_match) = caps.get(1) else {
            continue;
        };
        let raw_href = raw_match.as_str().trim();
        if raw_href.is_empty() {
            continue;
        }

        let decoded_href = urlencoding::decode(raw_href)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| raw_href.to_string());
        let normalized = decoded_href.trim_end_matches('/');
        let Some(file_name) = normalized.rsplit('/').next() else {
            continue;
        };
        let Some(file_caps) = re_file.captures(file_name) else {
            continue;
        };
        let Some(device_id_match) = file_caps.get(1) else {
            continue;
        };
        let Some(seq_match) = file_caps.get(2) else {
            continue;
        };
        let Ok(seq) = seq_match.as_str().parse::<i64>() else {
            continue;
        };
        let device_id = device_id_match.as_str().to_string();
        let dedup_key = format!("{}:{}", device_id, seq);
        refs.entry(dedup_key)
            .or_insert(WebDavOpRef { device_id, seq });
    }

    let mut out: Vec<WebDavOpRef> = refs.into_values().collect();

    out.sort_by(|a, b| a.device_id.cmp(&b.device_id).then(a.seq.cmp(&b.seq)));
    out
}

async fn list_webdav_op_refs(
    client: &Client,
    cfg: &CloudSyncConfig,
    ops_path: &str,
) -> AppResult<Vec<WebDavOpRef>> {
    let method = Method::from_bytes(b"PROPFIND")
        .map_err(|e| AppError::Internal(format!("invalid PROPFIND method: {}", e)))?;
    let url = webdav_collection_url_for(cfg, ops_path);
    let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getlastmodified />
  </d:prop>
</d:propfind>"#;

    let resp = webdav_send_with_retry(|| {
        webdav_with_auth(
            client
                .request(method.clone(), &url)
                .header("Depth", "1")
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(body.to_string()),
            cfg,
        )
    })
    .await?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 207 {
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Network(format!(
            "webdav PROPFIND ops failed: {} {}",
            status, text
        )));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;

    Ok(parse_webdav_op_refs(&text))
}

async fn fetch_webdav_ops_batch(
    client: &Client,
    cfg: &CloudSyncConfig,
    ops_path: &str,
    op_ref: &WebDavOpRef,
) -> AppResult<Option<WebDavOpsBatch>> {
    let relative = format!(
        "{}/{}",
        ops_path.trim_end_matches('/'),
        webdav_ops_filename(&op_ref.device_id, op_ref.seq)
    );
    let url = webdav_url_for(cfg, &relative);

    fetch_webdav_json_resource(
        || webdav_with_auth(client.get(&url), cfg),
        404,
        "webdav GET ops batch failed",
        "parse ops batch json failed",
    )
    .await
}

fn collect_syncable_settings(app: &AppHandle) -> AppResult<HashMap<String, String>> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let mut map = db_state.settings_repo.get_all().map_err(AppError::from)?;
    map.retain(|k, _| is_setting_sync_eligible(k));

    if let Some(raw) = map.get(EMOJI_FAVORITES_SETTING_KEY).cloned() {
        if let Some(encoded) = encode_emoji_favorites_setting(&raw) {
            map.insert(EMOJI_FAVORITES_SETTING_KEY.to_string(), encoded);
        }
    }

    Ok(map)
}

fn apply_synced_settings(app: &AppHandle, incoming: &HashMap<String, String>) -> AppResult<usize> {
    if incoming.is_empty() {
        return Ok(0);
    }
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let current = db_state.settings_repo.get_all().map_err(AppError::from)?;
    let mut changed = 0usize;
    for (key, value) in incoming {
        if !is_setting_sync_eligible(key) {
            continue;
        }

        let normalized_value = if key == EMOJI_FAVORITES_SETTING_KEY {
            decode_emoji_favorites_setting(app, value)?
        } else {
            value.clone()
        };

        if current
            .get(key)
            .map(|v| v == &normalized_value)
            .unwrap_or(false)
        {
            continue;
        }
        db_state
            .settings_repo
            .set(key, &normalized_value)
            .map_err(AppError::from)?;
        changed += 1;
    }
    Ok(changed)
}

async fn upload_webdav_settings_snapshot(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    settings_path: &str,
) -> AppResult<HashMap<String, String>> {
    let local_settings = collect_syncable_settings(app)?;

    let snapshot = WebDavSettingsSnapshot {
        device_id: cfg.device_id.clone(),
        updated_at: now_ms(),
        settings: local_settings.clone(),
    };
    let body = serde_json::to_vec(&snapshot)
        .map_err(|e| AppError::Internal(format!("serialize settings snapshot failed: {}", e)))?;
    let relative = format!(
        "{}/{}.json",
        settings_path.trim_end_matches('/'),
        cfg.device_id
    );
    upload_webdav_json_resource(client, cfg, &relative, body, "settings snapshot").await?;
    Ok(local_settings)
}

async fn fetch_webdav_settings_snapshot(
    client: &Client,
    cfg: &CloudSyncConfig,
    settings_path: &str,
    device_id: &str,
) -> AppResult<Option<WebDavSettingsSnapshot>> {
    let relative = format!("{}/{}.json", settings_path.trim_end_matches('/'), device_id);
    let url = webdav_url_for(cfg, &relative);
    fetch_webdav_json_resource(
        || webdav_with_auth(client.get(&url), cfg),
        404,
        "webdav GET settings snapshot failed",
        "parse settings snapshot json failed",
    )
    .await
}

async fn pull_remote_settings_snapshot(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    settings_path: &str,
) -> AppResult<usize> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let last_applied_ts = db_state
        .settings_repo
        .get("cloud_sync_settings_applied_at")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    let ids = list_webdav_snapshot_ids(client, cfg, settings_path).await?;
    let mut latest: Option<WebDavSettingsSnapshot> = None;
    for device_id in ids.into_iter().take(MAX_REMOTE_SNAPSHOTS) {
        if crate::app::system::same_anon_id(&device_id, &cfg.device_id) {
            continue;
        }
        if let Some(snapshot) =
            fetch_webdav_settings_snapshot(client, cfg, settings_path, &device_id).await?
        {
            let replace = latest
                .as_ref()
                .map(|cur| snapshot.updated_at > cur.updated_at)
                .unwrap_or(true);
            if replace {
                latest = Some(snapshot);
            }
        }
    }

    let Some(snapshot) = latest else {
        return Ok(0);
    };
    if snapshot.updated_at <= last_applied_ts {
        return Ok(0);
    }

    let changed = apply_synced_settings(app, &snapshot.settings)?;
    db_state
        .settings_repo
        .set(
            "cloud_sync_settings_applied_at",
            &snapshot.updated_at.to_string(),
        )
        .map_err(AppError::from)?;
    Ok(changed)
}

fn should_rebuild_webdav_head(app: &AppHandle, now: i64) -> bool {
    let last = get_setting_i64(app, CLOUD_SYNC_WEBDAV_LAST_HEAD_REBUILD_AT_KEY, 0);
    should_run_periodic_snapshot(last, now, WEBDAV_HEAD_REBUILD_INTERVAL_SECS)
}

fn touch_webdav_head_rebuild_at(app: &AppHandle, now: i64) {
    set_setting_i64(app, CLOUD_SYNC_WEBDAV_LAST_HEAD_REBUILD_AT_KEY, now);
}

fn update_webdav_head_device<F>(head: &mut WebDavSyncHead, device_id: &str, mut update: F)
where
    F: FnMut(&mut WebDavDeviceHead),
{
    let entry = head.devices.entry(device_id.to_string()).or_default();
    update(entry);
}

async fn fetch_webdav_sync_head(
    client: &Client,
    cfg: &CloudSyncConfig,
    head_path: &str,
) -> AppResult<Option<WebDavSyncHead>> {
    let url = webdav_url_for(cfg, head_path);
    fetch_webdav_json_resource(
        || webdav_with_auth(client.get(&url), cfg),
        404,
        "webdav GET head failed",
        "parse head json failed",
    )
    .await
}

async fn upload_webdav_sync_head(
    client: &Client,
    cfg: &CloudSyncConfig,
    head_path: &str,
    head: &WebDavSyncHead,
) -> AppResult<()> {
    let body = serde_json::to_vec(head)
        .map_err(|e| AppError::Internal(format!("serialize head failed: {}", e)))?;
    upload_webdav_json_resource(client, cfg, head_path, body, "sync head").await
}

async fn rebuild_webdav_sync_head(
    client: &Client,
    cfg: &CloudSyncConfig,
    paths: &WebDavPaths,
) -> AppResult<WebDavSyncHead> {
    let mut head = WebDavSyncHead {
        updated_at: now_ms(),
        devices: BTreeMap::new(),
    };

    for op_ref in list_webdav_op_refs(client, cfg, &paths.ops_path).await? {
        update_webdav_head_device(&mut head, &op_ref.device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(op_ref.seq);
        });
    }

    for device_id in list_webdav_snapshot_ids(client, cfg, &paths.devices_path).await? {
        let snapshot = fetch_webdav_snapshot(client, cfg, &paths.devices_path, &device_id).await?;
        let updated_at = snapshot
            .as_ref()
            .map(|snapshot| snapshot.updated_at)
            .unwrap_or(0);
        let snapshot_op_seq = snapshot
            .as_ref()
            .map(|snapshot| snapshot.latest_op_seq)
            .unwrap_or(0);
        update_webdav_head_device(&mut head, &device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(snapshot_op_seq);
            device.snapshot_updated_at = device.snapshot_updated_at.max(updated_at);
            device.snapshot_op_seq = device.snapshot_op_seq.max(snapshot_op_seq);
        });
    }

    for device_id in list_webdav_snapshot_ids(client, cfg, &paths.settings_path).await? {
        let updated_at =
            fetch_webdav_settings_snapshot(client, cfg, &paths.settings_path, &device_id)
                .await?
                .map(|snapshot| snapshot.updated_at)
                .unwrap_or(0);
        update_webdav_head_device(&mut head, &device_id, |device| {
            device.settings_updated_at = device.settings_updated_at.max(updated_at);
        });
    }

    Ok(head)
}

async fn resolve_webdav_sync_head(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    paths: &WebDavPaths,
    now: i64,
) -> AppResult<WebDavSyncHead> {
    let fetched = fetch_webdav_sync_head(client, cfg, &paths.head_path).await?;
    let needs_rebuild = fetched.is_none() || should_rebuild_webdav_head(app, now);

    if !needs_rebuild {
        return Ok(fetched.unwrap_or_default());
    }

    match rebuild_webdav_sync_head(client, cfg, paths).await {
        Ok(mut rebuilt) => {
            rebuilt.updated_at = now_ms();
            if fetched.as_ref() != Some(&rebuilt) {
                upload_webdav_sync_head(client, cfg, &paths.head_path, &rebuilt).await?;
            }
            touch_webdav_head_rebuild_at(app, now);
            Ok(rebuilt)
        }
        Err(err) => {
            if let Some(existing) = fetched {
                Ok(existing)
            } else {
                Err(err)
            }
        }
    }
}

async fn pull_remote_webdav_ops_from_head(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    blobs_path: &str,
    ops_path: &str,
    head: &WebDavSyncHead,
) -> AppResult<(usize, bool)> {
    let mut cursor_map = load_webdav_op_cursor_map(app);
    let mut received = 0usize;
    let mut head_stale = false;

    for (device_id, device_head) in &head.devices {
        if crate::app::system::same_anon_id(device_id, &cfg.device_id) {
            continue;
        }
        if device_head.latest_op_seq <= 0 {
            continue;
        }

        let mut last_seq = cursor_map.get(device_id).copied().unwrap_or(0);
        if device_head.latest_op_seq <= last_seq {
            continue;
        }

        for seq in (last_seq + 1)..=device_head.latest_op_seq {
            if cloud_sync_cancel_requested() {
                return Ok((received, head_stale));
            }

            let op_ref = WebDavOpRef {
                device_id: device_id.clone(),
                seq,
            };
            match fetch_webdav_ops_batch(client, cfg, ops_path, &op_ref).await? {
                Some(mut batch) if batch.device_id == op_ref.device_id => {
                    enrich_item_blobs_after_pull(app, client, cfg, blobs_path, &mut batch.entries)
                        .await?;
                    received += apply_remote_changes(app, &batch.entries, &cfg.content_prefs)?;
                    last_seq = last_seq.max(batch.seq).max(seq);
                    cursor_map.insert(device_id.clone(), last_seq);
                }
                Some(_) | None => {
                    head_stale = true;
                    break;
                }
            }
        }
    }

    save_webdav_op_cursor_map(app, &cursor_map);
    Ok((received, head_stale))
}

async fn pull_remote_webdav_snapshots_from_head(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    blobs_path: &str,
    devices_path: &str,
    head: &WebDavSyncHead,
) -> AppResult<usize> {
    let mut remote_items: Vec<CloudSyncItem> = Vec::new();
    let mut device_ids: Vec<(String, i64)> = head
        .devices
        .iter()
        .filter_map(|(device_id, device_head)| {
            if crate::app::system::same_anon_id(device_id, &cfg.device_id)
                || device_head.snapshot_updated_at <= 0
            {
                None
            } else {
                Some((device_id.clone(), device_head.snapshot_updated_at))
            }
        })
        .collect();

    device_ids.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    for (device_id, _) in device_ids.into_iter().take(MAX_REMOTE_SNAPSHOTS) {
        if cloud_sync_cancel_requested() {
            break;
        }
        if let Some(mut snapshot) =
            fetch_webdav_snapshot(client, cfg, devices_path, &device_id).await?
        {
            enrich_item_blobs_after_pull(app, client, cfg, blobs_path, &mut snapshot.entries)
                .await?;
            remote_items.extend(snapshot.entries);
        }
    }

    remote_items.sort_by_key(item_revision);
    apply_remote_changes(app, &remote_items, &cfg.content_prefs)
}

async fn pull_remote_settings_snapshot_from_head(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    settings_path: &str,
    head: &WebDavSyncHead,
) -> AppResult<usize> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    let last_applied_ts = db_state
        .settings_repo
        .get("cloud_sync_settings_applied_at")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    let Some((device_id, latest_ts)) = head
        .devices
        .iter()
        .filter(|(device_id, device_head)| {
            !crate::app::system::same_anon_id(device_id, &cfg.device_id)
                && device_head.settings_updated_at > 0
        })
        .map(|(device_id, device_head)| (device_id.clone(), device_head.settings_updated_at))
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
    else {
        return Ok(0);
    };

    if latest_ts <= last_applied_ts {
        return Ok(0);
    }

    let Some(snapshot) =
        fetch_webdav_settings_snapshot(client, cfg, settings_path, &device_id).await?
    else {
        return Ok(0);
    };
    if snapshot.updated_at <= last_applied_ts {
        return Ok(0);
    }

    let changed = apply_synced_settings(app, &snapshot.settings)?;
    db_state
        .settings_repo
        .set(
            "cloud_sync_settings_applied_at",
            &snapshot.updated_at.to_string(),
        )
        .map_err(AppError::from)?;
    Ok(changed)
}

async fn cleanup_local_webdav_ops(
    client: &Client,
    cfg: &CloudSyncConfig,
    ops_path: &str,
    max_seq_to_delete: i64,
) -> AppResult<usize> {
    if max_seq_to_delete <= 0 {
        return Ok(0);
    }

    let refs = list_webdav_op_refs(client, cfg, ops_path).await?;
    let mut deleted = 0usize;
    for op_ref in refs {
        if !crate::app::system::same_anon_id(&op_ref.device_id, &cfg.device_id)
            || op_ref.seq > max_seq_to_delete
        {
            continue;
        }

        let relative = format!(
            "{}/{}",
            ops_path.trim_end_matches('/'),
            webdav_ops_filename(&op_ref.device_id, op_ref.seq)
        );
        delete_webdav_resource_if_exists(client, cfg, &relative).await?;
        deleted += 1;
    }

    Ok(deleted)
}

fn should_run_periodic_snapshot(last_ts: i64, now: i64, interval_secs: i64) -> bool {
    if last_ts <= 0 {
        return true;
    }
    now.saturating_sub(last_ts) >= interval_secs.saturating_mul(1000)
}

fn should_push_webdav_snapshot(app: &AppHandle, now: i64, snapshot_interval_secs: i64) -> bool {
    let last = get_setting_i64(app, CLOUD_SYNC_WEBDAV_LAST_SNAPSHOT_PUSH_AT_KEY, 0);
    should_run_periodic_snapshot(last, now, snapshot_interval_secs)
}

fn should_pull_webdav_snapshot(
    app: &AppHandle,
    now: i64,
    has_remote_op_cursor: bool,
    snapshot_interval_secs: i64,
) -> bool {
    let last = get_setting_i64(app, CLOUD_SYNC_WEBDAV_LAST_SNAPSHOT_PULL_AT_KEY, 0);
    if !has_remote_op_cursor {
        // Cold-start fallback for new peers without op cursors yet.
        return should_run_periodic_snapshot(last, now, (5 * 60).min(snapshot_interval_secs));
    }
    should_run_periodic_snapshot(last, now, snapshot_interval_secs)
}

async fn pull_remote_webdav_ops(
    app: &AppHandle,
    client: &Client,
    cfg: &CloudSyncConfig,
    ops_path: &str,
    blobs_path: &str,
) -> AppResult<usize> {
    let refs = list_webdav_op_refs(client, cfg, ops_path).await?;
    if refs.is_empty() {
        return Ok(0);
    }

    let mut cursor_map = load_webdav_op_cursor_map(app);
    let mut received = 0usize;
    for op_ref in refs {
        if cloud_sync_cancel_requested() {
            break;
        }
        if crate::app::system::same_anon_id(&op_ref.device_id, &cfg.device_id) {
            continue;
        }
        let last_seq = cursor_map.get(&op_ref.device_id).copied().unwrap_or(0);
        if op_ref.seq <= last_seq {
            continue;
        }

        if let Some(mut batch) = fetch_webdav_ops_batch(client, cfg, ops_path, &op_ref).await? {
            if batch.device_id != op_ref.device_id {
                continue;
            }
            if cloud_sync_cancel_requested() {
                break;
            }
            enrich_item_blobs_after_pull(app, client, cfg, blobs_path, &mut batch.entries).await?;
            received += apply_remote_changes(app, &batch.entries, &cfg.content_prefs)?;
            let next_seq = batch.seq.max(op_ref.seq).max(last_seq);
            cursor_map.insert(op_ref.device_id.clone(), next_seq);
        }
    }
    save_webdav_op_cursor_map(app, &cursor_map);
    Ok(received)
}

async fn sync_once_http(app: &AppHandle, cfg: &CloudSyncConfig) -> AppResult<CloudSyncStatus> {
    let local_items = collect_local_changes(app, cfg.cursor, &cfg.content_prefs)?;
    let endpoint = format!(
        "{}/api/v1/clipboard/sync",
        cfg.base_url.trim_end_matches('/')
    );
    let request = CloudSyncRequest {
        device_id: cfg.device_id.clone(),
        cursor: cfg.cursor,
        entries: local_items.clone(),
    };

    let client = build_http_client()?;
    let mut http_req = client.post(&endpoint).json(&request);
    if !cfg.api_key.trim().is_empty() {
        http_req = http_req.bearer_auth(cfg.api_key.trim());
    }

    let resp = http_req
        .send()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        let status_code = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Network(format!(
            "sync endpoint failed: {} {}",
            status_code, text
        )));
    }

    let body = resp
        .json::<CloudSyncResponse>()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;

    let received = apply_remote_changes(app, &body.entries, &cfg.content_prefs)?;
    if received > 0 {
        let _ = app.emit("clipboard-changed", ());
    }
    let local_max = local_items
        .iter()
        .map(|x| x.timestamp)
        .max()
        .unwrap_or(cfg.cursor);
    let remote_max = body
        .entries
        .iter()
        .map(|x| x.timestamp)
        .max()
        .unwrap_or(cfg.cursor);
    let next_cursor = body
        .cursor
        .unwrap_or(cfg.cursor)
        .max(local_max)
        .max(remote_max);

    if let Some(db_state) = app.try_state::<DbState>() {
        let _ = db_state
            .settings_repo
            .set("cloud_sync_cursor", &next_cursor.to_string());
    }

    let now = now_ms();
    CLOUD_SYNC_LAST_SYNC_AT.store(now, Ordering::Relaxed);
    Ok(CloudSyncStatus {
        state: "idle".to_string(),
        running: true,
        last_sync_at: Some(now),
        last_error: None,
        uploaded_items: local_items.len(),
        received_items: received,
    })
}

async fn sync_once_webdav(
    app: &AppHandle,
    cfg: &CloudSyncConfig,
    force_snapshot: bool,
) -> AppResult<CloudSyncStatus> {
    if cloud_sync_cancel_requested() {
        return Ok(disabled_status());
    }
    let now = now_ms();
    let local_items = collect_local_syncable_items(app, &cfg.content_prefs)?;
    let (delta_items, collapsed_index) = collect_local_incremental_items(app, &local_items)?;
    let client = build_http_client()?;
    let paths = ensure_webdav_directories(&client, cfg).await?;
    let mut sync_head = resolve_webdav_sync_head(app, &client, cfg, &paths, now).await?;
    let mut webdav_blob_cache = load_webdav_blob_cache(app);
    let should_pull_snapshot = force_snapshot
        || should_pull_webdav_snapshot(
            app,
            now,
            !load_webdav_op_cursor_map(app).is_empty(),
            cfg.snapshot_interval_secs,
        );
    let should_push_snapshot =
        force_snapshot || should_push_webdav_snapshot(app, now, cfg.snapshot_interval_secs);

    let mut uploaded_items = 0usize;
    if !delta_items.is_empty() {
        let published_seq = sync_head
            .devices
            .get(&cfg.device_id)
            .map(|device| device.latest_op_seq)
            .unwrap_or(0);
        let mut next_seq = get_local_webdav_op_seq(app).max(published_seq);
        let mut processed_delta = delta_items.clone();
        process_items_blobs_before_push(
            &client,
            cfg,
            &paths.blobs_path,
            &mut webdav_blob_cache,
            &mut processed_delta,
        )
        .await?;
        for chunk in processed_delta.chunks(WEBDAV_OP_BATCH_SIZE) {
            if cloud_sync_cancel_requested() {
                return Ok(disabled_status());
            }
            next_seq = next_seq.saturating_add(1);
            upload_webdav_ops_batch(&client, cfg, &paths.ops_path, next_seq, chunk).await?;
        }
        // Publish discoverability before suppressing these local deltas. If the head
        // write fails, the sequence/index remain unchanged and the idempotent op files
        // are retried on the next run rather than becoming permanently invisible.
        update_webdav_head_device(&mut sync_head, &cfg.device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(next_seq);
        });
        sync_head.updated_at = now_ms();
        upload_webdav_sync_head(&client, cfg, &paths.head_path, &sync_head).await?;
        set_local_webdav_op_seq(app, next_seq)?;
        replace_local_sync_index(app, &collapsed_index)?;
        uploaded_items += delta_items.len();
    }

    if cloud_sync_cancel_requested() {
        return Ok(disabled_status());
    }

    let (mut received_items, head_stale) = pull_remote_webdav_ops_from_head(
        app,
        &client,
        cfg,
        &paths.blobs_path,
        &paths.ops_path,
        &sync_head,
    )
    .await?;
    if head_stale {
        let rebuilt = rebuild_webdav_sync_head(&client, cfg, &paths).await?;
        if rebuilt != sync_head {
            sync_head = rebuilt;
            sync_head.updated_at = now_ms();
            upload_webdav_sync_head(&client, cfg, &paths.head_path, &sync_head).await?;
            touch_webdav_head_rebuild_at(app, now);
        }
        received_items +=
            pull_remote_webdav_ops(app, &client, cfg, &paths.ops_path, &paths.blobs_path).await?;
        received_items += pull_remote_webdav_snapshots_from_head(
            app,
            &client,
            cfg,
            &paths.blobs_path,
            &paths.devices_path,
            &sync_head,
        )
        .await?;
    }

    // Incremental Emoji Sync check
    if let Ok(emoji_op) = check_and_create_emoji_sync_op(app) {
        if let Some(op) = emoji_op {
            let emoji_hash = op.content_hash;
            let published_seq = sync_head
                .devices
                .get(&cfg.device_id)
                .map(|device| device.latest_op_seq)
                .unwrap_or(0);
            let next_seq = get_local_webdav_op_seq(app)
                .max(published_seq)
                .saturating_add(1);
            upload_webdav_ops_batch(&client, cfg, &paths.ops_path, next_seq, &[op]).await?;
            update_webdav_head_device(&mut sync_head, &cfg.device_id, |device| {
                device.latest_op_seq = device.latest_op_seq.max(next_seq);
            });
            sync_head.updated_at = now_ms();
            upload_webdav_sync_head(&client, cfg, &paths.head_path, &sync_head).await?;
            set_local_webdav_op_seq(app, next_seq)?;
            LAST_PUSHED_EMOJI_HASH.store(emoji_hash, Ordering::Relaxed);
            uploaded_items += 1;
        }
    }

    save_webdav_blob_cache(app, &webdav_blob_cache);

    if should_pull_snapshot {
        received_items += pull_remote_webdav_snapshots_from_head(
            app,
            &client,
            cfg,
            &paths.blobs_path,
            &paths.devices_path,
            &sync_head,
        )
        .await?;

        received_items += pull_remote_settings_snapshot_from_head(
            app,
            &client,
            cfg,
            &paths.settings_path,
            &sync_head,
        )
        .await?;
        set_setting_i64(app, CLOUD_SYNC_WEBDAV_LAST_SNAPSHOT_PULL_AT_KEY, now);
    }

    if should_push_snapshot {
        if cloud_sync_cancel_requested() {
            return Ok(disabled_status());
        }
        let latest_op_seq = get_local_webdav_op_seq(app);
        // Pulls above may have merged newer metadata. Re-read before publishing the
        // snapshot so this device never advertises its stale pre-pull state.
        let snapshot_items = collect_local_syncable_items(app, &cfg.content_prefs)?;
        upload_webdav_snapshot(
            &client,
            cfg,
            &paths.devices_path,
            latest_op_seq,
            &snapshot_items,
        )
        .await?;
        uploaded_items += snapshot_items.len();
        let snapshot_updated_at = now_ms();
        update_webdav_head_device(&mut sync_head, &cfg.device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(latest_op_seq);
            device.snapshot_updated_at = device.snapshot_updated_at.max(snapshot_updated_at);
            device.snapshot_op_seq = device.snapshot_op_seq.max(latest_op_seq);
        });
        let local_settings =
            upload_webdav_settings_snapshot(app, &client, cfg, &paths.settings_path).await?;
        uploaded_items += local_settings.len();
        let settings_updated_at = now_ms();
        update_webdav_head_device(&mut sync_head, &cfg.device_id, |device| {
            device.settings_updated_at = device.settings_updated_at.max(settings_updated_at);
        });

        // Publication is the commit point: retry without advancing the local snapshot
        // clock or deleting covered operations unless the updated head is discoverable.
        sync_head.updated_at = now_ms();
        upload_webdav_sync_head(&client, cfg, &paths.head_path, &sync_head).await?;
        set_setting_i64_result(app, CLOUD_SYNC_WEBDAV_LAST_SNAPSHOT_PUSH_AT_KEY, now)?;
        let _ = cleanup_local_webdav_ops(&client, cfg, &paths.ops_path, latest_op_seq).await;
    }

    if received_items > 0 {
        let _ = app.emit("clipboard-changed", ());
    }
    CLOUD_SYNC_LAST_SYNC_AT.store(now, Ordering::Relaxed);

    if let Some(db_state) = app.try_state::<DbState>() {
        let _ = db_state
            .settings_repo
            .set("cloud_sync_cursor", &now.to_string());
    }

    Ok(CloudSyncStatus {
        state: "idle".to_string(),
        running: true,
        last_sync_at: Some(now),
        last_error: None,
        uploaded_items,
        received_items,
    })
}

async fn sync_once(
    app: &AppHandle,
    cfg: &CloudSyncConfig,
    force_snapshot: bool,
) -> AppResult<CloudSyncStatus> {
    let _run_guard = sync_run_lock().lock().await;
    if cloud_sync_cancel_requested() {
        let status = disabled_status();
        emit_status(Some(app), status.clone());
        return Ok(status);
    }

    if !cfg.enabled {
        let status = CloudSyncStatus {
            state: "disabled".to_string(),
            running: false,
            last_sync_at: None,
            last_error: None,
            uploaded_items: 0,
            received_items: 0,
        };
        emit_status(Some(app), status.clone());
        return Ok(status);
    }

    if !cloud_sync_target_ready(cfg) {
        let msg = cloud_sync_target_not_ready_message(cfg);
        let status = CloudSyncStatus {
            state: "error".to_string(),
            running: true,
            last_sync_at: None,
            last_error: Some(msg.clone()),
            uploaded_items: 0,
            received_items: 0,
        };
        emit_status(Some(app), status);
        return Err(AppError::Validation(msg));
    }

    emit_status(
        Some(app),
        CloudSyncStatus {
            state: "syncing".to_string(),
            running: true,
            last_sync_at: None,
            last_error: None,
            uploaded_items: 0,
            received_items: 0,
        },
    );

    let result = match cfg.provider {
        CloudSyncProvider::Http => sync_once_http(app, cfg).await,
        CloudSyncProvider::WebDav => sync_once_webdav(app, cfg, force_snapshot).await,
    };

    match result {
        Ok(status) => {
            emit_status(Some(app), status.clone());
            Ok(status)
        }
        Err(err) => {
            if cloud_sync_cancel_requested() {
                let status = disabled_status();
                emit_status(Some(app), status.clone());
                return Ok(status);
            }
            emit_status(
                Some(app),
                CloudSyncStatus {
                    state: "error".to_string(),
                    running: true,
                    last_sync_at: None,
                    last_error: Some(format!("[{}] {}", cfg.provider.as_str(), err)),
                    uploaded_items: 0,
                    received_items: 0,
                },
            );
            Err(err)
        }
    }
}

struct CloudSyncTaskGuard;

impl Drop for CloudSyncTaskGuard {
    fn drop(&mut self) {
        CLOUD_SYNC_TASK_ACTIVE.store(false, Ordering::Relaxed);
    }
}

pub fn get_cloud_sync_status() -> CloudSyncStatus {
    if let Ok(guard) = status_store().lock() {
        guard.clone()
    } else {
        CloudSyncStatus {
            state: "error".to_string(),
            running: false,
            last_sync_at: None,
            last_error: Some("status lock poisoned".to_string()),
            uploaded_items: 0,
            received_items: 0,
        }
    }
}

pub fn start_cloud_sync_client(app: AppHandle) {
    if CLOUD_SYNC_TASK_ACTIVE.swap(true, Ordering::Relaxed) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let _guard = CloudSyncTaskGuard;

        loop {
            let mut requested = CLOUD_SYNC_REQUESTED.swap(false, Ordering::Relaxed);
            let cfg = match get_config(&app) {
                Some(c) => c,
                None => {
                    emit_status(
                        Some(&app),
                        CloudSyncStatus {
                            state: "disabled".to_string(),
                            running: false,
                            last_sync_at: None,
                            last_error: None,
                            uploaded_items: 0,
                            received_items: 0,
                        },
                    );
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let now = now_ms();
            let backoff_until = CLOUD_SYNC_BACKOFF_UNTIL.load(Ordering::Relaxed);
            if backoff_until > now {
                let remaining_secs = (backoff_until - now) / 1000;
                if remaining_secs > 0 {
                    emit_status(
                        Some(&app),
                        CloudSyncStatus {
                            state: "idle".to_string(),
                            running: true,
                            last_sync_at: None,
                            last_error: Some(format!(
                                "WebDAV Cooldown (JianGuoYun Rate Limit): {}s remaining",
                                remaining_secs
                            )),
                            uploaded_items: 0,
                            received_items: 0,
                        },
                    );
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }

            if !cfg.enabled || !cloud_sync_target_ready(&cfg) {
                if !cfg.enabled {
                    emit_status(
                        Some(&app),
                        CloudSyncStatus {
                            state: "disabled".to_string(),
                            running: false,
                            last_sync_at: None,
                            last_error: None,
                            uploaded_items: 0,
                            received_items: 0,
                        },
                    );
                } else {
                    emit_status(
                        Some(&app),
                        CloudSyncStatus {
                            state: "error".to_string(),
                            running: false,
                            last_sync_at: None,
                            last_error: Some(cloud_sync_target_not_ready_message(&cfg)),
                            uploaded_items: 0,
                            received_items: 0,
                        },
                    );
                }
            } else if cfg.auto_sync || requested {
                if let Err(e) = sync_once(&app, &cfg, false).await {
                    emit_status(
                        Some(&app),
                        CloudSyncStatus {
                            state: "error".to_string(),
                            running: true,
                            last_sync_at: None,
                            last_error: Some(e.to_string()),
                            uploaded_items: 0,
                            received_items: 0,
                        },
                    );
                }
            } else {
                emit_status(
                    Some(&app),
                    CloudSyncStatus {
                        state: "idle".to_string(),
                        running: true,
                        last_sync_at: None,
                        last_error: None,
                        uploaded_items: 0,
                        received_items: 0,
                    },
                );
            }

            if cfg.auto_sync {
                let interval = cfg
                    .interval_secs
                    .clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS);
                let mut elapsed = 0u64;
                while elapsed < interval {
                    requested = CLOUD_SYNC_REQUESTED.swap(false, Ordering::Relaxed);
                    if requested {
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                    elapsed += 1;
                }
            } else {
                loop {
                    requested = CLOUD_SYNC_REQUESTED.swap(false, Ordering::Relaxed);
                    if requested {
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
}

pub fn restart_cloud_sync_client(app: AppHandle) {
    CLOUD_SYNC_CANCEL_REQUESTED.store(false, Ordering::Relaxed);
    start_cloud_sync_client(app);
    CLOUD_SYNC_REQUESTED.store(true, Ordering::Relaxed);
}

pub fn request_cloud_sync(app: AppHandle) {
    let Some(cfg) = get_config(&app) else {
        return;
    };
    if !cfg.enabled || !cfg.auto_sync || !cloud_sync_target_ready(&cfg) {
        return;
    }
    CLOUD_SYNC_CANCEL_REQUESTED.store(false, Ordering::Relaxed);
    start_cloud_sync_client(app);
    CLOUD_SYNC_REQUESTED.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub fn stop_cloud_sync_client(app: AppHandle) {
    CLOUD_SYNC_CANCEL_REQUESTED.store(true, Ordering::Relaxed);
    emit_status(Some(&app), disabled_status());
}

pub async fn cloud_sync_now(app: AppHandle) -> AppResult<CloudSyncStatus> {
    let current = get_cloud_sync_status();
    if current.state == "syncing" {
        return Ok(current);
    }
    CLOUD_SYNC_CANCEL_REQUESTED.store(false, Ordering::Relaxed);
    let cfg =
        get_config(&app).ok_or_else(|| AppError::Internal("DB state unavailable".to_string()))?;
    sync_once(&app, &cfg, true).await
}

fn check_and_create_emoji_sync_op(app: &AppHandle) -> AppResult<Option<CloudSyncItem>> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB unavailable".to_string()))?;

    let emoji_prefs = db_state
        .settings_repo
        .get("cloud_sync_content_prefs")
        .ok()
        .flatten()
        .map(|raw| serde_json::from_str::<CloudSyncContentPrefs>(&raw).unwrap_or_default())
        .unwrap_or_default();
    if !emoji_prefs.emoji {
        return Ok(None);
    }

    let emoji_json = db_state
        .settings_repo
        .get(EMOJI_FAVORITES_SETTING_KEY)
        .ok()
        .flatten()
        .unwrap_or_default();

    if emoji_json.trim().is_empty() || emoji_json == "[]" {
        return Ok(None);
    }

    let Some(sync_payload) = encode_emoji_favorites_setting(&emoji_json) else {
        return Ok(None);
    };
    if sync_payload.trim().is_empty() || sync_payload == "[]" {
        return Ok(None);
    }

    let current_hash = emoji_sync_payload_hash(&sync_payload);

    if current_hash == LAST_PUSHED_EMOJI_HASH.load(Ordering::Relaxed) {
        return Ok(None);
    }

    Ok(Some(CloudSyncItem {
        content_type: "emoji_sync".to_string(),
        content: sync_payload,
        content_hash: current_hash,
        hash_version: HASH_VERSION_WHITESPACE,
        deleted_at: 0,
        html_content: None,
        content_blob_hash: None,
        html_blob_hash: None,
        source_app: "TieZ".to_string(),
        timestamp: now_ms(),
        updated_at: now_ms(),
        updated_by: db_state
            .settings_repo
            .get("app.anon_id")
            .ok()
            .flatten()
            .unwrap_or_default(),
        preview: "⭐ Emoji Sync".to_string(),
        is_pinned: false,
        pinned_order: 0,
        tags: vec![],
        use_count: 0,
    }))
}

fn merge_remote_emojis(app: &AppHandle, remote_json: &str) -> AppResult<()> {
    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("DB unavailable".to_string()))?;
    let local_json = db_state
        .settings_repo
        .get(EMOJI_FAVORITES_SETTING_KEY)
        .ok()
        .flatten()
        .unwrap_or_default();

    let local_paths = if local_json.trim().is_empty() || local_json == "[]" {
        Vec::new()
    } else {
        materialize_emoji_favorite_paths(app, &local_json)?
    };
    let remote_paths = materialize_emoji_favorite_paths(app, remote_json)?;
    let normalized_local_json = serde_json::to_string(&local_paths).unwrap_or_default();

    let mut merged: std::collections::HashSet<String> = local_paths.iter().cloned().collect();
    for path in &remote_paths {
        merged.insert(path.clone());
    }

    let mut merged_paths: Vec<String> = merged.into_iter().collect();
    merged_paths.sort();
    let new_json = serde_json::to_string(&merged_paths).unwrap_or_default();

    if merged_paths != local_paths || normalized_local_json != local_json {
        db_state
            .settings_repo
            .set(EMOJI_FAVORITES_SETTING_KEY, &new_json)
            .map_err(AppError::from)?;
        let _ = app.emit("settings-changed", ());
    }

    let sync_payload =
        encode_emoji_favorites_setting(&new_json).unwrap_or_else(|| "[]".to_string());
    if let Some(hash) = merged_emoji_suppression_hash(&remote_paths, &merged_paths, &sync_payload) {
        LAST_PUSHED_EMOJI_HASH.store(hash, Ordering::Relaxed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_remote_emoji_item, apply_remote_live_item_with_conn,
        apply_remote_tombstone_with_conn, canonicalize_incoming_live_hash,
        clear_matching_tombstones, collapse_items_by_sync_key, decode_persisted_sync_text,
        emoji_sync_payload_hash, find_existing_entry_for_sync, load_persisted_sync_item,
        matching_tombstone_deleted_at, merged_emoji_suppression_hash, normalize_item_for_sync,
        record_remote_sync_digest, remote_tombstone_matches_stored_entry,
        rewrite_rich_html_resources_for_sync, sync_digest_for_item, sync_key_for_item,
        update_existing_entry_from_sync, CloudSyncItem, HASH_VERSION_LEGACY,
        HASH_VERSION_WHITESPACE, RICH_IMAGE_FALLBACK_PREFIX, RICH_IMAGE_FALLBACK_SUFFIX,
    };
    use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
    use rusqlite::Connection;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_PNG_BYTES: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 15, 4, 0, 9,
        251, 3, 253, 160, 164, 95, 122, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    fn make_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tiez-cloud-sync-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_sync_item(updated_at: i64, updated_by: &str, tags: &[&str]) -> CloudSyncItem {
        CloudSyncItem {
            content_type: "text".to_string(),
            content: "shared".to_string(),
            content_hash: crate::database::calc_text_hash("shared") as i64,
            hash_version: HASH_VERSION_WHITESPACE,
            deleted_at: 0,
            html_content: None,
            content_blob_hash: None,
            html_blob_hash: None,
            source_app: "Test".to_string(),
            timestamp: 100,
            updated_at,
            updated_by: updated_by.to_string(),
            preview: "shared".to_string(),
            is_pinned: false,
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            use_count: 0,
            pinned_order: 0,
        }
    }

    #[test]
    fn emoji_merge_failure_propagates_to_sync_caller() {
        let mut item = test_sync_item(10, "remote", &[]);
        item.content_type = "emoji_sync".to_string();
        item.content = "not valid emoji data".to_string();
        let error =
            apply_remote_emoji_item(&item, &super::CloudSyncContentPrefs::default(), |_| {
                Err(crate::error::AppError::Internal("merge failed".to_string()))
            })
            .expect_err("emoji merge error must propagate");
        assert!(error.to_string().contains("merge failed"));
    }

    #[test]
    fn exact_remote_emoji_payload_suppresses_echo_upload() {
        let remote_paths = vec!["remote-favorite.png".to_string()];
        let merged_paths = remote_paths.clone();
        let resulting_payload = r#"["data:image/png;base64,cmVtb3Rl"]"#;

        let last_pushed =
            merged_emoji_suppression_hash(&remote_paths, &merged_paths, resulting_payload)
                .expect("exact remote result should be suppressible");

        assert_eq!(last_pushed, emoji_sync_payload_hash(resulting_payload));
    }

    #[test]
    fn remote_plus_local_emoji_union_remains_pending_for_upload() {
        let remote_paths = vec!["remote-favorite.png".to_string()];
        let merged_paths = vec![
            "local-favorite.png".to_string(),
            "remote-favorite.png".to_string(),
        ];
        let resulting_payload =
            r#"["data:image/png;base64,bG9jYWw=","data:image/png;base64,cmVtb3Rl"]"#;

        assert_eq!(
            merged_emoji_suppression_hash(&remote_paths, &merged_paths, resulting_payload),
            None,
            "the enlarged union must not update LAST_PUSHED_EMOJI_HASH"
        );
    }

    #[test]
    fn old_live_payload_is_canonicalized_to_whitespace_preserving_hash() {
        let raw = r#"{
            "content_type":"text","content":"hello ",
            "content_hash":123,"deleted_at":0,"source_app":"old-peer",
            "timestamp":10,"preview":"hello "
        }"#;
        let mut item: CloudSyncItem = serde_json::from_str(raw).expect("deserialize v1 item");
        assert_eq!(item.hash_version, HASH_VERSION_LEGACY);

        canonicalize_incoming_live_hash(&mut item);

        assert_eq!(item.hash_version, HASH_VERSION_WHITESPACE);
        assert_eq!(
            item.content_hash,
            crate::database::calc_text_hash("hello ") as i64
        );
        assert_ne!(
            item.content_hash,
            crate::database::calc_text_hash("hello") as i64
        );
    }

    #[test]
    fn unversioned_hashless_image_matches_migrated_v2_row_and_tombstone() {
        let content = super::image_data_url_from_blob_bytes(TEST_PNG_BYTES)
            .expect("create test image data url");
        let expected_hash = crate::database::calc_image_hash(&content).expect("hash test image");
        let mut item: CloudSyncItem = serde_json::from_value(serde_json::json!({
            "content_type": "image",
            "content": content,
            "content_hash": 0,
            "source_app": "old-peer",
            "timestamp": 100,
            "updated_at": 100,
            "preview": "[Image Content]"
        }))
        .expect("deserialize unversioned image");
        assert_eq!(item.hash_version, HASH_VERSION_LEGACY);

        canonicalize_incoming_live_hash(&mut item);

        assert_eq!(item.hash_version, HASH_VERSION_WHITESPACE);
        assert_eq!(item.content_hash, expected_hash);

        let conn = Connection::open_in_memory().expect("open image identity db");
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );",
        )
        .expect("create image identity schema");
        conn.execute(
            "INSERT INTO clipboard_history
                (id, content_type, content_hash, content_hash_version)
             VALUES (1, 'image', ?1, 2)",
            [expected_hash],
        )
        .expect("insert migrated v2 image row");
        conn.execute(
            "INSERT INTO cloud_sync_tombstones
                (content_type, content_hash, hash_version, deleted_at)
             VALUES ('image', ?1, 2, 200)",
            [expected_hash],
        )
        .expect("insert v2 image tombstone");

        assert_eq!(
            find_existing_entry_for_sync(&conn, &item).expect("match migrated image row"),
            Some(1)
        );
        let tombstone_deleted_at =
            matching_tombstone_deleted_at(&conn, &item).expect("match v2 image tombstone");
        assert_eq!(tombstone_deleted_at, 200);
        assert!(tombstone_deleted_at >= super::item_updated_at(&item));
    }

    #[test]
    fn unversioned_image_uses_supplied_hash_when_recomputation_is_unavailable() {
        let mut item = test_sync_item(100, "old-peer", &[]);
        item.content_type = "image".to_string();
        item.content = "/definitely/missing/tiez-sync-image.png".to_string();
        item.content_hash = 42;
        item.hash_version = HASH_VERSION_LEGACY;

        canonicalize_incoming_live_hash(&mut item);

        assert_eq!(item.content_hash, 42);
        assert_eq!(item.hash_version, HASH_VERSION_WHITESPACE);
    }

    #[test]
    fn legacy_tombstones_accept_trimmed_and_pre_version_exact_hashes() {
        let conn = Connection::open_in_memory().expect("open tombstone db");
        conn.execute_batch(
            "CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );",
        )
        .expect("create tombstone table");

        let mut spaced = test_sync_item(50, "old-peer", &[]);
        spaced.content = "hello ".to_string();
        canonicalize_incoming_live_hash(&mut spaced);
        let legacy_trimmed = crate::database::calc_legacy_text_hash("hello ") as i64;
        let pre_version_exact = crate::database::calc_text_hash("hello ") as i64;

        for predecessor_hash in [legacy_trimmed, pre_version_exact] {
            conn.execute("DELETE FROM cloud_sync_tombstones", [])
                .expect("clear predecessor tombstone");
            conn.execute(
                "INSERT INTO cloud_sync_tombstones
                    (content_type, content_hash, hash_version, deleted_at)
                 VALUES ('text', ?1, 1, 100)",
                [predecessor_hash],
            )
            .expect("insert v1 tombstone");
            assert_eq!(
                matching_tombstone_deleted_at(&conn, &spaced)
                    .expect("match ambiguous v1 tombstone"),
                100
            );
        }

        conn.execute("DELETE FROM cloud_sync_tombstones", [])
            .expect("clear predecessor tombstones");
        for predecessor_hash in [legacy_trimmed, pre_version_exact] {
            conn.execute(
                "INSERT INTO cloud_sync_tombstones
                    (content_type, content_hash, hash_version, deleted_at)
                 VALUES ('text', ?1, 1, 40)",
                [predecessor_hash],
            )
            .expect("insert clearable v1 tombstone");
        }
        clear_matching_tombstones(&conn, &spaced).expect("clear ambiguous v1 tombstones");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count cleared tombstones");
        assert_eq!(remaining, 0);

        conn.execute(
            "INSERT INTO cloud_sync_tombstones
                (content_type, content_hash, hash_version, deleted_at)
             VALUES ('text', ?1, 2, 100)",
            [crate::database::calc_text_hash("hello") as i64],
        )
        .expect("insert v2 tombstone");
        assert_eq!(
            matching_tombstone_deleted_at(&conn, &spaced).expect("check exact v2 tombstone"),
            0
        );
    }

    #[test]
    fn remote_v1_tombstone_matching_accepts_both_predecessor_algorithms() {
        let stored_content = "hello ";
        let stored_exact = crate::database::calc_text_hash(stored_content) as i64;
        assert!(remote_tombstone_matches_stored_entry(
            "text",
            crate::database::calc_legacy_text_hash(stored_content) as i64,
            HASH_VERSION_LEGACY,
            stored_content,
            stored_exact,
            HASH_VERSION_WHITESPACE,
        ));
        assert!(remote_tombstone_matches_stored_entry(
            "text",
            stored_exact,
            HASH_VERSION_LEGACY,
            stored_content,
            stored_exact,
            HASH_VERSION_WHITESPACE,
        ));
    }

    fn setup_merge_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory merge db");
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL DEFAULT 'text',
                content TEXT NOT NULL DEFAULT 'shared',
                content_hash INTEGER NOT NULL DEFAULT 0,
                content_hash_version INTEGER NOT NULL DEFAULT 2,
                html_content TEXT,
                timestamp INTEGER NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                pinned_order INTEGER NOT NULL DEFAULT 0,
                preview TEXT NOT NULL,
                source_app TEXT NOT NULL,
                use_count INTEGER NOT NULL DEFAULT 0,
                tags TEXT NOT NULL DEFAULT '[]',
                source_app_path TEXT,
                is_external INTEGER NOT NULL DEFAULT 0,
                sync_updated_at INTEGER NOT NULL,
                sync_updated_by TEXT NOT NULL
             );
             CREATE TABLE entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );
             CREATE TABLE cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
             );
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO settings (key, value) VALUES ('app.anon_id', 'local-device');",
        )
        .expect("create merge schema");
        conn
    }

    #[test]
    fn v2_existing_lookup_does_not_alias_undecryptable_v1_row() {
        let conn = setup_merge_db();
        let hash = crate::database::calc_text_hash("hello ") as i64;
        conn.execute(
            "INSERT INTO clipboard_history
             (id, content, content_hash, content_hash_version, timestamp, preview, source_app,
              sync_updated_at, sync_updated_by)
             VALUES (1, 'dpapi:not-decryptable', ?1, 1, 100, 'legacy', 'Test', 100, 'old')",
            [hash],
        )
        .expect("insert undecryptable legacy row");
        let mut item = test_sync_item(200, "new", &[]);
        item.content = "hello ".to_string();
        canonicalize_incoming_live_hash(&mut item);

        assert_eq!(
            find_existing_entry_for_sync(&conn, &item).expect("lookup compatible entry"),
            None
        );
        let (stored_hash, stored_version): (i64, i64) = conn
            .query_row(
                "SELECT content_hash, content_hash_version FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read preserved legacy identity");
        assert_eq!((stored_hash, stored_version), (hash, HASH_VERSION_LEGACY));
    }

    #[test]
    fn v2_remote_deletion_requires_v2_stored_hash_semantics() {
        let hash = crate::database::calc_text_hash("hello ") as i64;
        assert!(!remote_tombstone_matches_stored_entry(
            "text",
            hash,
            HASH_VERSION_WHITESPACE,
            "dpapi:not-decryptable",
            hash,
            HASH_VERSION_LEGACY,
        ));
        assert!(remote_tombstone_matches_stored_entry(
            "text",
            hash,
            HASH_VERSION_WHITESPACE,
            "hello ",
            hash,
            HASH_VERSION_WHITESPACE,
        ));
    }

    #[test]
    fn pulled_tombstone_cleans_metadata_and_attachment_without_echo() {
        let root = make_temp_dir("remote-tombstone");
        let attachments = root.join("attachments");
        fs::create_dir_all(&attachments).expect("create attachment directory");
        let attachment = attachments.join("remote-delete.txt");
        fs::write(&attachment, b"payload").expect("write attachment");
        let content = attachment.to_string_lossy().to_string();
        let content_hash = crate::database::calc_text_hash(&content) as i64;

        let conn = Connection::open_in_memory().expect("open remote deletion db");
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL,
                html_content TEXT,
                is_external INTEGER NOT NULL DEFAULT 0,
                sync_updated_at INTEGER NOT NULL,
                sync_updated_by TEXT NOT NULL
             );
             CREATE TABLE entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );
             CREATE TABLE cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
             );",
        )
        .expect("create remote deletion schema");
        conn.execute(
            "INSERT INTO clipboard_history
                (id, content_type, content, content_hash, content_hash_version, is_external,
                 sync_updated_at, sync_updated_by)
             VALUES (1, 'file', ?1, ?2, 2, 1, 10, 'local')",
            rusqlite::params![content, content_hash],
        )
        .expect("insert remotely deleted row");
        conn.execute(
            "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remote')",
            [],
        )
        .expect("insert remotely deleted metadata");
        let shared = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(shared.clone());
        let mut tombstone = test_sync_item(999, "remote", &[]);
        tombstone.content_type = "file".to_string();
        tombstone.content.clear();
        tombstone.content_hash = content_hash;
        tombstone.hash_version = HASH_VERSION_WHITESPACE;
        tombstone.deleted_at = 77;
        tombstone.timestamp = 999;

        {
            let guard = shared.lock().expect("lock remote deletion db");
            assert_eq!(
                apply_remote_tombstone_with_conn(&repo, &guard, &tombstone, Some(&root))
                    .expect("apply remote tombstone"),
                1
            );
            let history_count: i64 = guard
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get(0)
                })
                .expect("count deleted history");
            let tag_count: i64 = guard
                .query_row("SELECT COUNT(*) FROM entry_tags", [], |row| row.get(0))
                .expect("count deleted tags");
            let tombstones: Vec<(i64, i64)> = {
                let mut stmt = guard
                    .prepare(
                        "SELECT hash_version, deleted_at FROM cloud_sync_tombstones
                         ORDER BY hash_version",
                    )
                    .expect("prepare tombstone query");
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .expect("query tombstones")
                    .collect::<Result<_, _>>()
                    .expect("read tombstones")
            };
            assert_eq!(history_count, 0);
            assert_eq!(tag_count, 0);
            assert_eq!(tombstones, vec![(HASH_VERSION_WHITESPACE, 77)]);
        }
        assert!(!attachment.exists());
        let _ = fs::remove_dir_all(root);
    }

    fn setup_remote_tombstone_failure_fixture(
        name: &str,
        failure_trigger: &str,
    ) -> (
        PathBuf,
        PathBuf,
        Arc<Mutex<Connection>>,
        SqliteClipboardRepository,
        CloudSyncItem,
    ) {
        let root = make_temp_dir(name);
        let attachments = root.join("attachments");
        fs::create_dir_all(&attachments).expect("create attachment directory");
        let attachment = attachments.join("keep-on-failure.txt");
        fs::write(&attachment, b"payload").expect("write attachment");
        let content = attachment.to_string_lossy().to_string();
        let content_hash = crate::database::calc_text_hash(&content) as i64;

        let conn = Connection::open_in_memory().expect("open remote deletion db");
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL,
                html_content TEXT,
                is_external INTEGER NOT NULL DEFAULT 0,
                sync_updated_at INTEGER NOT NULL,
                sync_updated_by TEXT NOT NULL
             );
             CREATE TABLE entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );
             CREATE TABLE cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
             );",
        )
        .expect("create remote deletion schema");
        conn.execute(
            "INSERT INTO clipboard_history
                (id, content_type, content, content_hash, content_hash_version, is_external,
                 sync_updated_at, sync_updated_by)
             VALUES (1, 'file', ?1, ?2, 2, 1, 10, 'local')",
            rusqlite::params![content, content_hash],
        )
        .expect("insert remotely deleted row");
        conn.execute(
            "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remote')",
            [],
        )
        .expect("insert remotely deleted tag");
        conn.execute_batch(failure_trigger)
            .expect("install remote deletion failure trigger");

        let shared = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(shared.clone());
        let mut tombstone = test_sync_item(999, "remote", &[]);
        tombstone.content_type = "file".to_string();
        tombstone.content.clear();
        tombstone.content_hash = content_hash;
        tombstone.deleted_at = 77;
        tombstone.timestamp = 999;
        (root, attachment, shared, repo, tombstone)
    }

    fn assert_failed_remote_tombstone_is_atomic(
        root: PathBuf,
        attachment: PathBuf,
        shared: Arc<Mutex<Connection>>,
        repo: SqliteClipboardRepository,
        tombstone: CloudSyncItem,
        expected_error: &str,
    ) {
        {
            let guard = shared.lock().expect("lock remote deletion db");
            let error =
                apply_remote_tombstone_with_conn(&repo, &guard, &tombstone, Some(root.as_path()))
                    .expect_err("remote deletion must fail atomically");
            assert!(error.to_string().contains(expected_error));

            for (table, expected) in [
                ("clipboard_history", 1_i64),
                ("entry_tags", 1_i64),
                ("cloud_sync_tombstones", 0_i64),
                ("cloud_sync_local_index", 0_i64),
            ] {
                let count: i64 = guard
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .expect("count atomic remote deletion state");
                assert_eq!(count, expected, "unexpected row count in {table}");
            }
        }
        assert!(
            attachment.exists(),
            "cleanup must wait for a successful commit"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remote_tombstone_entry_tag_failure_rolls_back_all_state_and_cleanup() {
        let fixture = setup_remote_tombstone_failure_fixture(
            "remote-tombstone-tag-failure",
            "CREATE TRIGGER reject_remote_tag_delete
             BEFORE DELETE ON entry_tags
             BEGIN SELECT RAISE(ABORT, 'remote tag delete unavailable'); END;",
        );
        assert_failed_remote_tombstone_is_atomic(
            fixture.0,
            fixture.1,
            fixture.2,
            fixture.3,
            fixture.4,
            "remote tag delete unavailable",
        );
    }

    #[test]
    fn remote_tombstone_digest_failure_rolls_back_all_state_and_cleanup() {
        let fixture = setup_remote_tombstone_failure_fixture(
            "remote-tombstone-digest-failure",
            "CREATE TRIGGER reject_remote_delete_digest
             BEFORE INSERT ON cloud_sync_local_index
             BEGIN SELECT RAISE(ABORT, 'remote digest unavailable'); END;",
        );
        assert_failed_remote_tombstone_is_atomic(
            fixture.0,
            fixture.1,
            fixture.2,
            fixture.3,
            fixture.4,
            "remote digest unavailable",
        );
    }

    #[test]
    fn remote_tombstone_preserves_fallback_referenced_by_surviving_entry() {
        let root = make_temp_dir("remote-shared-fallback");
        let attachments = root.join("attachments");
        fs::create_dir_all(&attachments).expect("create attachment directory");
        let fallback = attachments.join("shared-fallback.png");
        fs::write(&fallback, b"shared fallback").expect("write fallback");
        let html = format!(
            "<p>shared</p><!--TIEZ_RICH_IMAGE:file://{}-->",
            fallback.to_string_lossy()
        );
        let conn = Connection::open_in_memory().expect("open remote deletion db");
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL,
                html_content TEXT,
                is_external INTEGER NOT NULL DEFAULT 0,
                sync_updated_at INTEGER NOT NULL,
                sync_updated_by TEXT NOT NULL
             );
             CREATE TABLE entry_tags (entry_id INTEGER NOT NULL, tag TEXT NOT NULL);
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );
             CREATE TABLE cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
             );",
        )
        .expect("create remote deletion schema");
        conn.execute(
            "INSERT INTO clipboard_history
                (id, content_type, content, content_hash, content_hash_version, html_content,
                 sync_updated_at, sync_updated_by)
             VALUES (1, 'rich_text', 'delete me', 41, 2, ?1, 10, 'local'),
                    (2, 'rich_text', 'keep me', 42, 2, ?1, 10, 'local')",
            [html],
        )
        .expect("insert shared fallback entries");
        let shared = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(shared.clone());
        let mut tombstone = test_sync_item(999, "remote", &[]);
        tombstone.content_type = "rich_text".to_string();
        tombstone.content.clear();
        tombstone.content_hash = 41;
        tombstone.deleted_at = 77;

        {
            let guard = shared.lock().expect("lock remote deletion db");
            assert_eq!(
                apply_remote_tombstone_with_conn(&repo, &guard, &tombstone, Some(&root))
                    .expect("apply remote tombstone"),
                1
            );
            let surviving: i64 = guard
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_history WHERE id = 2",
                    [],
                    |row| row.get(0),
                )
                .expect("count surviving fallback reference");
            assert_eq!(surviving, 1);
        }
        assert!(
            fallback.exists(),
            "the surviving row still references the fallback"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_remote_metadata_does_not_overwrite_newer_local_tags() {
        let conn = setup_merge_db();
        conn.execute(
            "INSERT INTO clipboard_history
             (id, timestamp, preview, source_app, tags, sync_updated_at, sync_updated_by)
             VALUES (1, 100, 'shared', 'Test', '[\"local\"]', 200, 'device-a')",
            [],
        )
        .expect("insert local entry");
        conn.execute(
            "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'local')",
            [],
        )
        .expect("insert local tag");

        let stale = test_sync_item(150, "device-b", &["stale"]);
        let (changed, remote_accepted) =
            update_existing_entry_from_sync(&conn, 1, &stale, stale.timestamp)
                .expect("merge stale metadata");

        assert!(!changed);
        assert!(!remote_accepted);
        let tags: String = conn
            .query_row(
                "SELECT tags FROM clipboard_history WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read tags");
        assert_eq!(tags, "[\"local\"]");
    }

    #[test]
    fn newer_remote_metadata_replaces_tags_and_revision() {
        let conn = setup_merge_db();
        conn.execute(
            "INSERT INTO clipboard_history
             (id, timestamp, preview, source_app, tags, sync_updated_at, sync_updated_by)
             VALUES (1, 100, 'shared', 'Test', '[\"old\"]', 200, 'device-a')",
            [],
        )
        .expect("insert local entry");
        conn.execute(
            "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'old')",
            [],
        )
        .expect("insert old tag");

        let newer = test_sync_item(300, "device-b", &["new"]);
        let (changed, remote_accepted) =
            update_existing_entry_from_sync(&conn, 1, &newer, newer.timestamp)
                .expect("merge newer metadata");

        assert!(changed);
        assert!(remote_accepted);
        let (tags, updated_at, updated_by): (String, i64, String) = conn
            .query_row(
                "SELECT tags, sync_updated_at, sync_updated_by
                 FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read merged revision");
        assert_eq!(tags, "[\"new\"]");
        assert_eq!(updated_at, 300);
        assert_eq!(updated_by, "device-b");
        let entry_tag: String = conn
            .query_row("SELECT tag FROM entry_tags WHERE entry_id = 1", [], |row| {
                row.get(0)
            })
            .expect("read normalized tag");
        assert_eq!(entry_tag, "new");
    }

    #[test]
    fn remote_tag_insert_failure_rolls_back_history_and_normalized_tags() {
        let conn = setup_merge_db();
        conn.execute(
            "INSERT INTO clipboard_history
             (id, content, timestamp, preview, source_app, tags, sync_updated_at, sync_updated_by)
             VALUES (1, 'shared', 100, 'local preview', 'Local', '[\"old\"]', 200, 'device-a')",
            [],
        )
        .expect("insert local entry");
        conn.execute(
            "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'old')",
            [],
        )
        .expect("insert old tag");
        conn.execute_batch(
            "CREATE TRIGGER reject_synced_tag
             BEFORE INSERT ON entry_tags
             WHEN NEW.tag = 'blocked'
             BEGIN SELECT RAISE(ABORT, 'synced tag unavailable'); END;",
        )
        .expect("install synced-tag failure trigger");

        let newer = test_sync_item(300, "device-b", &["blocked"]);
        let error = update_existing_entry_from_sync(&conn, 1, &newer, newer.timestamp)
            .expect_err("normalized tag insertion must reject the entire merge");
        assert!(error.to_string().contains("synced tag unavailable"));

        let (tags, preview, source_app, updated_at, updated_by): (
            String,
            String,
            String,
            i64,
            String,
        ) = conn
            .query_row(
                "SELECT tags, preview, source_app, sync_updated_at, sync_updated_by
                 FROM clipboard_history WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("read rolled-back history");
        let entry_tags: Vec<String> = conn
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = 1 ORDER BY tag")
            .expect("prepare rolled-back tag query")
            .query_map([], |row| row.get(0))
            .expect("query rolled-back tags")
            .collect::<Result<_, _>>()
            .expect("read rolled-back tags");
        let digest_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cloud_sync_local_index", [], |row| {
                row.get(0)
            })
            .expect("count unrecorded remote digests");
        assert_eq!(tags, "[\"old\"]");
        assert_eq!(preview, "local preview");
        assert_eq!(source_app, "Local");
        assert_eq!(updated_at, 200);
        assert_eq!(updated_by, "device-a");
        assert_eq!(entry_tags, vec!["old"]);
        assert_eq!(digest_count, 0);
    }

    fn assert_failed_live_apply_is_atomic(
        existing_entry: bool,
        failure_trigger: &str,
        expected_error: &str,
    ) {
        let conn = setup_merge_db();
        let item = test_sync_item(300, "device-b", &["remote"]);
        conn.execute(
            "INSERT INTO cloud_sync_tombstones
                (content_type, content_hash, hash_version, deleted_at)
             VALUES ('text', ?1, 2, 50)",
            [item.content_hash],
        )
        .expect("insert matching tombstone");
        if existing_entry {
            conn.execute(
                "INSERT INTO clipboard_history
                    (id, content, content_hash, timestamp, preview, source_app, tags,
                     sync_updated_at, sync_updated_by)
                 VALUES (1, 'shared', ?1, 100, 'local preview', 'Local',
                         '[\"old\"]', 200, 'device-a')",
                [item.content_hash],
            )
            .expect("insert existing local entry");
            conn.execute(
                "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'old')",
                [],
            )
            .expect("insert existing local tag");
        }
        conn.execute_batch(failure_trigger)
            .expect("install live apply failure trigger");

        let shared = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(shared.clone());
        let guard = shared.lock().expect("lock live apply db");
        let error = apply_remote_live_item_with_conn(&repo, &guard, &item, item.timestamp, None)
            .expect_err("remote live apply must fail atomically");
        assert!(error.to_string().contains(expected_error));

        let history_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })
            .expect("count live history");
        let tags: Vec<String> = guard
            .prepare("SELECT tag FROM entry_tags ORDER BY tag")
            .expect("prepare live tag query")
            .query_map([], |row| row.get(0))
            .expect("query live tags")
            .collect::<Result<_, _>>()
            .expect("read live tags");
        let tombstone_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count retained tombstone");
        let digest_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_local_index", [], |row| {
                row.get(0)
            })
            .expect("count rolled-back digest");

        if existing_entry {
            let (preview, tags_json, updated_at, updated_by): (String, String, i64, String) = guard
                .query_row(
                    "SELECT preview, tags, sync_updated_at, sync_updated_by
                     FROM clipboard_history WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .expect("read rolled-back existing entry");
            assert_eq!(history_count, 1);
            assert_eq!(tags, vec!["old"]);
            assert_eq!(preview, "local preview");
            assert_eq!(tags_json, "[\"old\"]");
            assert_eq!((updated_at, updated_by.as_str()), (200, "device-a"));
        } else {
            assert_eq!(history_count, 0);
            assert!(tags.is_empty());
        }
        assert_eq!(tombstone_count, 1);
        assert_eq!(
            digest_count, 0,
            "a failed pull must not create an echo-suppression index"
        );
    }

    #[test]
    fn existing_remote_live_digest_failure_rolls_back_content_tags_and_tombstone() {
        assert_failed_live_apply_is_atomic(
            true,
            "CREATE TRIGGER reject_live_digest
             BEFORE INSERT ON cloud_sync_local_index
             BEGIN SELECT RAISE(ABORT, 'live digest unavailable'); END;",
            "live digest unavailable",
        );
    }

    #[test]
    fn new_remote_live_digest_failure_rolls_back_insert_tags_and_tombstone() {
        assert_failed_live_apply_is_atomic(
            false,
            "CREATE TRIGGER reject_new_live_digest
             BEFORE INSERT ON cloud_sync_local_index
             BEGIN SELECT RAISE(ABORT, 'new live digest unavailable'); END;",
            "new live digest unavailable",
        );
    }

    #[test]
    fn existing_remote_live_tombstone_clear_failure_rolls_back_merge() {
        assert_failed_live_apply_is_atomic(
            true,
            "CREATE TRIGGER reject_live_tombstone_clear
             BEFORE DELETE ON cloud_sync_tombstones
             BEGIN SELECT RAISE(ABORT, 'live tombstone clear unavailable'); END;",
            "live tombstone clear unavailable",
        );
    }

    #[test]
    fn new_remote_live_tombstone_clear_failure_rolls_back_insert() {
        assert_failed_live_apply_is_atomic(
            false,
            "CREATE TRIGGER reject_new_live_tombstone_clear
             BEFORE DELETE ON cloud_sync_tombstones
             BEGIN SELECT RAISE(ABORT, 'new live tombstone clear unavailable'); END;",
            "new live tombstone clear unavailable",
        );
    }

    #[test]
    fn newer_rich_text_payload_converges_while_sensitive_fields_stay_protected() {
        let conn = setup_merge_db();
        conn.execute(
            "INSERT INTO clipboard_history
             (id, content_type, content, content_hash, html_content, timestamp, preview,
              source_app, tags, sync_updated_at, sync_updated_by)
             VALUES (1, 'rich_text', 'hello', ?1, '<p>old</p>', 100, 'hello',
                     'Test', '[\"sensitive\"]', 200, 'device-a')",
            [crate::database::calc_text_hash("hello") as i64],
        )
        .expect("insert local rich entry");

        let mut newer = test_sync_item(300, "device-b", &["sensitive"]);
        newer.content_type = "rich_text".to_string();
        newer.html_content = Some("<p><strong>new</strong></p>".to_string());
        let (changed, accepted) =
            update_existing_entry_from_sync(&conn, 1, &newer, newer.timestamp)
                .expect("merge rich payload");

        assert!(changed);
        assert!(accepted);
        let persisted = load_persisted_sync_item(&conn, 1).expect("load persisted rich item");
        assert_eq!(persisted.content, "shared");
        assert_eq!(persisted.html_content, newer.html_content);
        let (raw_content, raw_html): (String, Option<String>) = conn
            .query_row(
                "SELECT content, html_content FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read protected rich fields");
        assert_eq!(decode_persisted_sync_text(raw_content.clone()), "shared");
        assert_eq!(
            raw_html.clone().map(decode_persisted_sync_text),
            newer.html_content
        );
        #[cfg(all(windows, not(feature = "portable")))]
        {
            assert!(raw_content.starts_with(crate::database::ENCRYPT_PREFIX));
            assert!(raw_html
                .as_deref()
                .is_some_and(|html| html.starts_with(crate::database::ENCRYPT_PREFIX)));
        }
    }

    #[test]
    fn pulled_remote_digest_is_recorded_to_prevent_echo_upload() {
        let conn = setup_merge_db();
        let item = test_sync_item(300, "device-b", &["remote"]);
        let key = sync_key_for_item(&item).expect("sync key");

        record_remote_sync_digest(&conn, &item).expect("record remote digest");

        let digest: String = conn
            .query_row(
                "SELECT digest FROM cloud_sync_local_index WHERE sync_key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .expect("read remote digest");
        assert_eq!(digest, sync_digest_for_item(&item));
    }

    #[test]
    fn merged_digest_uses_persisted_max_use_count_not_raw_remote_payload() {
        let conn = setup_merge_db();
        conn.execute(
            "INSERT INTO clipboard_history
             (id, content, content_hash, timestamp, preview, source_app, tags, use_count,
              sync_updated_at, sync_updated_by)
             VALUES (1, 'shared', ?1, 100, 'shared', 'Test', '[\"local\"]', 9, 200, 'device-a')",
            [crate::database::calc_text_hash("shared") as i64],
        )
        .expect("insert local entry");
        let remote = test_sync_item(300, "device-b", &["remote"]);
        let (_, accepted) = update_existing_entry_from_sync(&conn, 1, &remote, remote.timestamp)
            .expect("merge remote metadata");
        assert!(accepted);

        let persisted = load_persisted_sync_item(&conn, 1).expect("load merged item");
        assert_eq!(persisted.use_count, 9);
        record_remote_sync_digest(&conn, &persisted).expect("record merged digest");
        let key = sync_key_for_item(&persisted).expect("sync key");
        let digest: String = conn
            .query_row(
                "SELECT digest FROM cloud_sync_local_index WHERE sync_key = ?1",
                [key],
                |row| row.get(0),
            )
            .expect("read merged digest");
        assert_eq!(digest, sync_digest_for_item(&persisted));
        assert_ne!(digest, sync_digest_for_item(&remote));
    }

    #[test]
    fn collapse_prefers_newer_metadata_revision_for_same_content() {
        let older = test_sync_item(100, "device-a", &["old"]);
        let newer = test_sync_item(200, "device-b", &["new"]);
        let key = sync_key_for_item(&newer).expect("sync key");

        let collapsed = collapse_items_by_sync_key(&[newer.clone(), older]);

        assert_eq!(
            collapsed.get(&key).map(|item| &item.tags),
            Some(&newer.tags)
        );
    }

    #[test]
    fn rewrite_rich_html_resources_for_sync_inlines_local_images_and_fallbacks() {
        let dir = make_temp_dir("rich-html");
        let image_path = dir.join("inline.png");
        fs::write(&image_path, TEST_PNG_BYTES).expect("write test png");

        let image_path_str = image_path.to_string_lossy().replace('\\', "/");
        let html = format!(
            "<div><img src=\"file://{}\"></div>\n{}{}{}",
            image_path_str, RICH_IMAGE_FALLBACK_PREFIX, image_path_str, RICH_IMAGE_FALLBACK_SUFFIX
        );

        let rewritten = rewrite_rich_html_resources_for_sync(&html);

        assert!(rewritten.contains("src=\"data:image/png;base64,"));
        assert!(rewritten.contains(RICH_IMAGE_FALLBACK_PREFIX));
        assert!(rewritten.contains("data:image/png;base64,"));
        assert!(!rewritten.contains(&format!("file://{}", image_path_str)));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_item_for_sync_rewrites_rich_html_local_resources() {
        let dir = make_temp_dir("normalize-item");
        let image_path = dir.join("entry.png");
        fs::write(&image_path, TEST_PNG_BYTES).expect("write test png");

        let item = CloudSyncItem {
            content_type: "rich_text".to_string(),
            content: "hello".to_string(),
            content_hash: 0,
            hash_version: HASH_VERSION_WHITESPACE,
            deleted_at: 0,
            html_content: Some(format!(
                "<p>Hello</p><img src=\"{}\">",
                image_path.to_string_lossy()
            )),
            content_blob_hash: None,
            html_blob_hash: None,
            source_app: "Test".to_string(),
            timestamp: 1,
            updated_at: 1,
            updated_by: "test-device".to_string(),
            preview: "hello".to_string(),
            is_pinned: false,
            tags: vec![],
            use_count: 0,
            pinned_order: 0,
        };

        let normalized = normalize_item_for_sync(item).expect("normalized item");
        let html = normalized.html_content.expect("html content");

        assert!(html.contains("src=\"data:image/png;base64,"));
        assert!(!html.contains("entry.png"));

        let _ = fs::remove_dir_all(dir);
    }
}
