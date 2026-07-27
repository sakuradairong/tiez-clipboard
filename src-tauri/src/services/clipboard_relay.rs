use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use futures::StreamExt;
use regex::Regex;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const RELAY_SCHEMA_VERSION: u8 = 1;
const RELAY_DEFAULT_TTL_MS: i64 = 10 * 60 * 1000;
const RELAY_MAX_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const RELAY_CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;
const RELAY_MAX_TEXT_BYTES: usize = 64 * 1024;
const RELAY_MAX_JSON_BYTES: usize = 70 * 1024;
const RELAY_MAX_PROPFIND_BYTES: usize = 2 * 1024 * 1024;
const RELAY_MAX_QUEUE_MESSAGES: usize = 2_000;
const RELAY_REQUEST_TIMEOUT_SECS: u64 = 30;
const RELAY_MAX_RETRIES: usize = 2;
const RELAY_CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const RELAY_BROADCAST_TARGET: &str = "*";

#[derive(Debug, Clone)]
struct RelayConfig {
    webdav_url: String,
    webdav_username: String,
    webdav_password: String,
    webdav_base_path: String,
    device_id: String,
    shared_key: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelayTextMessageV1 {
    schema_version: u8,
    message_id: String,
    sender_device_id: String,
    target_device_ids: Vec<String>,
    created_at: i64,
    expires_at: i64,
    content_type: String,
    encryption: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelayAckV1 {
    schema_version: u8,
    message_id: String,
    recipient_device_id: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RelayAckPayloadV1 {
    message_id: String,
    recipient_device_id: String,
    copied_at: i64,
}

fn ack_aad(message_id: &str, recipient_device_id: &str, file_name: &str) -> Vec<u8> {
    encode_aad_fields(
        &[
            "tiez-relay-ack-v1",
            file_name,
            message_id,
            recipient_device_id,
        ],
        &[RELAY_SCHEMA_VERSION as i64],
    )
}

fn create_ack(
    message_id: &str,
    recipient_device_id: &str,
    file_name: &str,
    copied_at: i64,
    key: &[u8; 32],
) -> AppResult<RelayAckV1> {
    let payload = RelayAckPayloadV1 {
        message_id: message_id.to_string(),
        recipient_device_id: recipient_device_id.to_string(),
        copied_at,
    };
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|error| AppError::Internal(format!("serialize relay ack: {}", error)))?;
    let (nonce, ciphertext) = encrypt_payload(
        &plaintext,
        &ack_aad(message_id, recipient_device_id, file_name),
        key,
    )?;
    Ok(RelayAckV1 {
        schema_version: RELAY_SCHEMA_VERSION,
        message_id: message_id.to_string(),
        recipient_device_id: recipient_device_id.to_string(),
        nonce,
        ciphertext,
    })
}

fn validate_ack(
    ack: &RelayAckV1,
    message_id: &str,
    recipient_device_id: &str,
    file_name: &str,
    key: &[u8; 32],
) -> AppResult<()> {
    if ack.schema_version != RELAY_SCHEMA_VERSION
        || ack.message_id != message_id
        || ack.recipient_device_id != recipient_device_id
    {
        return Err(AppError::Validation(
            "clipboard relay ack metadata is invalid".to_string(),
        ));
    }
    let plaintext = decrypt_payload(
        &ack.nonce,
        &ack.ciphertext,
        &ack_aad(message_id, recipient_device_id, file_name),
        key,
    )?;
    let payload: RelayAckPayloadV1 = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::Validation("clipboard relay ack is invalid".to_string()))?;
    if payload.message_id != message_id
        || payload.recipient_device_id != recipient_device_id
        || payload.copied_at < 0
    {
        return Err(AppError::Validation(
            "clipboard relay ack payload is invalid".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelayMessageRef {
    created_at: i64,
    message_id: String,
    file_name: String,
}

#[derive(Debug, Serialize)]
pub struct RelaySendResult {
    pub message_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub byte_len: usize,
}

#[derive(Debug, Serialize)]
pub struct RelayFetchResult {
    pub outcome: String,
    pub message_id: Option<String>,
    pub sender_device_id: Option<String>,
    pub created_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub acked: bool,
}

fn relay_run_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn normalize_base_path(raw: &str) -> AppResult<String> {
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() {
        return Ok("tiez-sync".to_string());
    }
    if normalized
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(AppError::Validation(
            "WebDAV 接力目录包含无效路径段".to_string(),
        ));
    }
    Ok(normalized.to_string())
}

fn validate_relay_url(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| AppError::Validation("WebDAV 地址格式无效".to_string()))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "剪贴板接力要求 WebDAV 地址使用 HTTPS".to_string(),
        ));
    }
    if parsed.fragment().is_some() {
        return Err(AppError::Validation(
            "WebDAV 地址不能包含 fragment".to_string(),
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn validate_device_id(device_id: &str) -> AppResult<()> {
    if device_id.is_empty()
        || device_id.len() > 128
        || !device_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(AppError::Validation(
            "clipboard relay device id is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_text(text: &str) -> AppResult<usize> {
    if text.is_empty() {
        return Err(AppError::Validation("剪贴板中没有可发送的文本".to_string()));
    }
    let byte_len = text.len();
    if byte_len > RELAY_MAX_TEXT_BYTES {
        return Err(AppError::Validation(format!(
            "剪贴板文本超过 {} KiB 接力上限",
            RELAY_MAX_TEXT_BYTES / 1024
        )));
    }
    Ok(byte_len)
}

fn encode_aad_fields(fields: &[&str], numbers: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    for number in numbers {
        out.extend_from_slice(&number.to_be_bytes());
    }
    out
}

fn message_aad(message: &RelayTextMessageV1, file_name: &str) -> Vec<u8> {
    let targets = message.target_device_ids.join("\u{1f}");
    encode_aad_fields(
        &[
            "tiez-relay-message-v1",
            file_name,
            &message.message_id,
            &message.sender_device_id,
            &targets,
            &message.content_type,
            &message.encryption,
        ],
        &[
            message.schema_version as i64,
            message.created_at,
            message.expires_at,
        ],
    )
}

fn encrypt_payload(plaintext: &[u8], aad: &[u8], key: &[u8; 32]) -> AppResult<(String, String)> {
    use base64::Engine;
    use chacha20poly1305::aead::rand_core::RngCore;

    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| AppError::Encryption("无法初始化接力加密".to_string()))?;
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| AppError::Encryption("无法加密剪贴板接力内容".to_string()))?;
    Ok((
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(nonce),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(ciphertext),
    ))
}

fn decrypt_payload(
    nonce: &str,
    ciphertext: &str,
    aad: &[u8],
    key: &[u8; 32],
) -> AppResult<Vec<u8>> {
    use base64::Engine;

    let nonce = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(nonce)
        .map_err(|_| AppError::Encryption("接力消息 nonce 格式无效".to_string()))?;
    if nonce.len() != 24 {
        return Err(AppError::Encryption("接力消息 nonce 长度无效".to_string()));
    }
    let ciphertext = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(ciphertext)
        .map_err(|_| AppError::Encryption("接力消息密文格式无效".to_string()))?;
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| AppError::Encryption("无法初始化接力解密".to_string()))?;
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        )
        .map_err(|_| AppError::Encryption("接力消息认证失败，请检查共享密钥".to_string()))
}

fn decrypt_text(
    message: &RelayTextMessageV1,
    file_name: &str,
    key: &[u8; 32],
) -> AppResult<String> {
    let plaintext = decrypt_payload(
        &message.nonce,
        &message.ciphertext,
        &message_aad(message, file_name),
        key,
    )?;
    String::from_utf8(plaintext)
        .map_err(|_| AppError::Encryption("接力消息不是有效 UTF-8 文本".to_string()))
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

    let configured_webdav_url = setting("cloud_sync_webdav_url");
    let fallback_url = setting("cloud_sync_server");
    let webdav_url = if configured_webdav_url.trim().is_empty() {
        fallback_url
    } else {
        configured_webdav_url
    };
    if webdav_url.trim().is_empty() {
        return Err(AppError::Validation(
            "请先在云同步设置中配置 WebDAV 地址".to_string(),
        ));
    }
    let webdav_url = validate_relay_url(&webdav_url)?;

    let configured_password = setting("cloud_sync_webdav_password");
    let fallback_password = setting("cloud_sync_api_key");
    let webdav_password = if configured_password.trim().is_empty() {
        fallback_password
    } else {
        configured_password
    };

    let stored_device_id = setting("app.anon_id");
    let device_id = crate::app::system::normalize_anon_id(&stored_device_id).unwrap_or_else(|| {
        crate::app::system::build_anon_id(&crate::app::system::get_machine_id())
    });
    validate_device_id(&device_id)?;
    if stored_device_id.trim() != device_id {
        db.settings_repo
            .set("app.anon_id", &device_id)
            .map_err(AppError::from)?;
    }

    Ok(RelayConfig {
        webdav_url,
        webdav_username: setting("cloud_sync_webdav_username"),
        webdav_password,
        webdav_base_path: normalize_base_path(&setting("cloud_sync_webdav_base_path"))?,
        device_id,
        shared_key: crate::services::relay_key::load()?
            .ok_or_else(|| AppError::Validation("请先配置剪贴板接力共享密钥".to_string()))?,
    })
}

fn build_http_client() -> AppResult<Client> {
    Client::builder()
        .timeout(Duration::from_secs(RELAY_REQUEST_TIMEOUT_SECS))
        .https_only(true)
        .build()
        .map_err(|error| AppError::Network(error.to_string()))
}

fn with_auth(request: RequestBuilder, config: &RelayConfig) -> RequestBuilder {
    if config.webdav_username.trim().is_empty() {
        request
    } else {
        request.basic_auth(
            config.webdav_username.trim(),
            Some(config.webdav_password.trim()),
        )
    }
}

fn encode_relative_path(relative_path: &str, collection: bool) -> String {
    let mut encoded = relative_path
        .replace('\\', "/")
        .split('/')
        .filter_map(|segment| {
            let segment = segment.trim();
            (!segment.is_empty()).then(|| urlencoding::encode(segment).into_owned())
        })
        .collect::<Vec<_>>()
        .join("/");
    if collection && !encoded.is_empty() {
        encoded.push('/');
    }
    encoded
}

fn resource_url(config: &RelayConfig, relative_path: &str) -> AppResult<String> {
    let encoded = encode_relative_path(relative_path, false);
    let value = if encoded.is_empty() {
        config.webdav_url.clone()
    } else {
        format!("{}/{}", config.webdav_url, encoded)
    };
    validate_relay_url(&value)
}

fn collection_url(config: &RelayConfig, relative_path: &str) -> AppResult<String> {
    let encoded = encode_relative_path(relative_path, true);
    let value = if encoded.is_empty() {
        format!("{}/", config.webdav_url)
    } else {
        format!("{}/{}", config.webdav_url, encoded)
    };
    validate_relay_url(&value)
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

async fn send_with_retry<F>(mut make_request: F) -> AppResult<Response>
where
    F: FnMut() -> RequestBuilder,
{
    let mut last_error = None;
    for attempt in 0..=RELAY_MAX_RETRIES {
        match make_request().send().await {
            Ok(response)
                if is_retryable_status(response.status()) && attempt < RELAY_MAX_RETRIES =>
            {
                last_error = Some(format!("transient WebDAV status {}", response.status()));
            }
            Ok(response) => return Ok(response),
            Err(error) if attempt < RELAY_MAX_RETRIES => {
                last_error = Some(error.to_string());
            }
            Err(error) => return Err(AppError::Network(error.to_string())),
        }
        tokio::time::sleep(Duration::from_millis(300 * (1u64 << attempt))).await;
    }
    Err(AppError::Network(
        last_error.unwrap_or_else(|| "WebDAV request failed".to_string()),
    ))
}

async fn read_limited(response: Response, max_bytes: usize, label: &str) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(AppError::Network(format!(
            "{} response is too large",
            label
        )));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| AppError::Network(error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(AppError::Network(format!(
                "{} response is too large",
                label
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn collection_exists(
    client: &Client,
    config: &RelayConfig,
    relative_path: &str,
) -> AppResult<bool> {
    let method =
        Method::from_bytes(b"PROPFIND").map_err(|error| AppError::Internal(error.to_string()))?;
    let url = collection_url(config, relative_path)?;
    let response = send_with_retry(|| {
        with_auth(
            client
                .request(method.clone(), &url)
                .header("Depth", "0")
                .header("Content-Type", "application/xml; charset=utf-8"),
            config,
        )
    })
    .await?;
    Ok(response.status().is_success() || response.status().as_u16() == 207)
}

async fn ensure_collection(
    client: &Client,
    config: &RelayConfig,
    relative_path: &str,
) -> AppResult<()> {
    let method =
        Method::from_bytes(b"MKCOL").map_err(|error| AppError::Internal(error.to_string()))?;
    let url = collection_url(config, relative_path)?;
    let response =
        send_with_retry(|| with_auth(client.request(method.clone(), &url), config)).await?;
    let status = response.status();
    if status.is_success() || status == StatusCode::METHOD_NOT_ALLOWED {
        return Ok(());
    }
    if matches!(status.as_u16(), 301 | 302 | 307 | 308 | 409)
        && collection_exists(client, config, relative_path).await?
    {
        return Ok(());
    }
    Err(AppError::Network(format!(
        "WebDAV 无法创建接力目录（状态码 {}）",
        status.as_u16()
    )))
}

#[derive(Debug, Clone)]
struct RelayPaths {
    messages: String,
    recipient_acks: String,
}

async fn ensure_relay_directories(client: &Client, config: &RelayConfig) -> AppResult<RelayPaths> {
    let base = normalize_base_path(&config.webdav_base_path)?;
    let relay = format!("{}/relay", base);
    let version = format!("{}/v1", relay);
    let messages = format!("{}/messages", version);
    let acks = format!("{}/acks", version);
    let recipient_acks = format!("{}/{}", acks, config.device_id);

    let mut current = String::new();
    for segment in base.split('/').filter(|segment| !segment.is_empty()) {
        current = if current.is_empty() {
            segment.to_string()
        } else {
            format!("{}/{}", current, segment)
        };
        ensure_collection(client, config, &current).await?;
    }
    for path in [&relay, &version, &messages, &acks, &recipient_acks] {
        ensure_collection(client, config, path).await?;
    }

    Ok(RelayPaths {
        messages,
        recipient_acks,
    })
}

async fn fetch_resource(
    client: &Client,
    config: &RelayConfig,
    relative_path: &str,
    max_bytes: usize,
) -> AppResult<Option<Vec<u8>>> {
    let url = resource_url(config, relative_path)?;
    let response = send_with_retry(|| with_auth(client.get(&url), config)).await?;
    if matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::CONFLICT
    ) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "WebDAV 读取接力数据失败（状态码 {}）",
            response.status().as_u16()
        )));
    }
    read_limited(response, max_bytes, "clipboard relay")
        .await
        .map(Some)
}

async fn create_json_resource<T: Serialize>(
    client: &Client,
    config: &RelayConfig,
    relative_path: &str,
    value: &T,
) -> AppResult<()> {
    let body = serde_json::to_vec(value)
        .map_err(|error| AppError::Internal(format!("serialize relay payload: {}", error)))?;
    if body.len() > RELAY_MAX_JSON_BYTES {
        return Err(AppError::Validation(
            "clipboard relay payload is too large".to_string(),
        ));
    }
    let url = resource_url(config, relative_path)?;
    let response = send_with_retry(|| {
        with_auth(
            client
                .put(&url)
                .header("Content-Type", "application/json")
                .header("If-None-Match", "*")
                .body(body.clone()),
            config,
        )
    })
    .await?;
    if response.status().is_success() {
        return Ok(());
    }
    if response.status() == StatusCode::PRECONDITION_FAILED {
        let existing = fetch_resource(client, config, relative_path, RELAY_MAX_JSON_BYTES).await?;
        return if existing.as_deref() == Some(body.as_slice()) {
            Ok(())
        } else {
            Err(AppError::Network(
                "clipboard relay resource already exists with different content".to_string(),
            ))
        };
    }
    Err(AppError::Network(format!(
        "WebDAV 发布接力数据失败（状态码 {}）",
        response.status().as_u16()
    )))
}

async fn list_message_refs(
    client: &Client,
    config: &RelayConfig,
    messages_path: &str,
) -> AppResult<Vec<RelayMessageRef>> {
    let method =
        Method::from_bytes(b"PROPFIND").map_err(|error| AppError::Internal(error.to_string()))?;
    let url = collection_url(config, messages_path)?;
    let response = send_with_retry(|| {
        with_auth(
            client
                .request(method.clone(), &url)
                .header("Depth", "1")
                .header("Content-Type", "application/xml; charset=utf-8"),
            config,
        )
    })
    .await?;
    if !(response.status().is_success() || response.status().as_u16() == 207) {
        return Err(AppError::Network(format!(
            "WebDAV 列出接力消息失败（状态码 {}）",
            response.status().as_u16()
        )));
    }
    let xml = String::from_utf8_lossy(
        &read_limited(
            response,
            RELAY_MAX_PROPFIND_BYTES,
            "clipboard relay listing",
        )
        .await?,
    )
    .into_owned();
    Ok(parse_message_refs(&xml))
}

fn parse_message_ref(file_name: &str) -> Option<RelayMessageRef> {
    let file_name = file_name.strip_suffix(".json")?;
    let (created_at, message_id) = file_name.split_once("__")?;
    let created_at = created_at.parse::<i64>().ok()?;
    if created_at < 0 {
        return None;
    }
    let message_id = Uuid::parse_str(message_id).ok()?.to_string();
    Some(RelayMessageRef {
        created_at,
        file_name: format!("{}__{}.json", created_at, message_id),
        message_id,
    })
}

fn parse_message_refs(xml: &str) -> Vec<RelayMessageRef> {
    let Ok(href_regex) = Regex::new(r"(?is)<[^>]*href[^>]*>\s*([^<]+)\s*</[^>]*href>") else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut refs = Vec::new();
    for capture in href_regex.captures_iter(xml) {
        let Some(raw_href) = capture.get(1) else {
            continue;
        };
        let decoded = urlencoding::decode(raw_href.as_str().trim())
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| raw_href.as_str().trim().to_string());
        let normalized = decoded
            .split('?')
            .next()
            .unwrap_or(&decoded)
            .trim_end_matches('/');
        let Some(file_name) = normalized.rsplit('/').next() else {
            continue;
        };
        let Some(message_ref) = parse_message_ref(file_name) else {
            continue;
        };
        if seen.insert(message_ref.file_name.clone()) {
            refs.push(message_ref);
        }
    }
    refs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.message_id.cmp(&left.message_id))
    });
    refs
}

fn split_remote_queue(
    refs: Vec<RelayMessageRef>,
    now: i64,
    max_refs: usize,
) -> (Vec<RelayMessageRef>, Vec<RelayMessageRef>) {
    let stale_before = now.saturating_sub(RELAY_MAX_TTL_MS + RELAY_CLOCK_SKEW_MS);
    let mut kept = Vec::with_capacity(refs.len().min(max_refs));
    let mut removed = Vec::new();
    for message_ref in refs {
        if message_ref.created_at < stale_before || kept.len() >= max_refs {
            removed.push(message_ref);
        } else {
            kept.push(message_ref);
        }
    }
    (kept, removed)
}

fn validate_incoming_message(
    message: &RelayTextMessageV1,
    message_ref: &RelayMessageRef,
    recipient_device_id: &str,
    shared_key: &[u8; 32],
    now: i64,
) -> AppResult<Option<String>> {
    if message.schema_version != RELAY_SCHEMA_VERSION
        || message.message_id != message_ref.message_id
        || message.created_at != message_ref.created_at
        || message.content_type != RELAY_CONTENT_TYPE
    {
        return Err(AppError::Validation(
            "clipboard relay message metadata is invalid".to_string(),
        ));
    }
    validate_device_id(&message.sender_device_id)?;
    if message.target_device_ids.is_empty() || message.target_device_ids.len() > 64 {
        return Err(AppError::Validation(
            "clipboard relay target list is invalid".to_string(),
        ));
    }
    for target in &message.target_device_ids {
        if target != RELAY_BROADCAST_TARGET {
            validate_device_id(target)?;
        }
    }
    let ttl = message.expires_at.saturating_sub(message.created_at);
    if message.created_at > now.saturating_add(RELAY_CLOCK_SKEW_MS)
        || ttl <= 0
        || ttl > RELAY_MAX_TTL_MS
    {
        return Err(AppError::Validation(
            "clipboard relay message has expired or has invalid timing".to_string(),
        ));
    }
    if message.encryption != "xchacha20poly1305" {
        return Err(AppError::Validation(
            "clipboard relay message encryption is unsupported".to_string(),
        ));
    }
    if message.expires_at <= now {
        return Ok(None);
    }
    let content = decrypt_text(message, &message_ref.file_name, shared_key)?;
    validate_text(&content)?;
    if message.sender_device_id == recipient_device_id
        || !message
            .target_device_ids
            .iter()
            .any(|target| target == RELAY_BROADCAST_TARGET || target == recipient_device_id)
    {
        return Ok(None);
    }
    Ok(Some(content))
}

fn message_is_locally_ineligible(
    message: &RelayTextMessageV1,
    recipient_device_id: &str,
    now: i64,
) -> bool {
    message.expires_at <= now
        || message.sender_device_id == recipient_device_id
        || !message
            .target_device_ids
            .iter()
            .any(|target| target == RELAY_BROADCAST_TARGET || target == recipient_device_id)
}

fn prune_receipts(db: &DbState, now: i64) -> AppResult<()> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    conn.execute(
        "DELETE FROM clipboard_relay_receipts WHERE expires_at <= ?1",
        params![now],
    )
    .map_err(AppError::from)?;
    Ok(())
}

fn receipt_state(db: &DbState, message_id: &str) -> AppResult<Option<String>> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    let mut stmt = conn
        .prepare("SELECT state FROM clipboard_relay_receipts WHERE message_id = ?1")
        .map_err(AppError::from)?;
    match stmt.query_row(params![message_id], |row| row.get(0)) {
        Ok(state) => Ok(Some(state)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(AppError::from(error)),
    }
}

fn reserve_receipt(db: &DbState, message: &RelayTextMessageV1, now: i64) -> AppResult<bool> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO clipboard_relay_receipts
                (message_id, expires_at, state, ack_json, updated_at)
             VALUES (?1, ?2, 'reserved', '', ?3)",
            params![message.message_id, message.expires_at, now],
        )
        .map_err(AppError::from)?;
    Ok(inserted == 1)
}

fn remove_reserved_receipt(db: &DbState, message_id: &str) -> AppResult<()> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    conn.execute(
        "DELETE FROM clipboard_relay_receipts WHERE message_id = ?1 AND state = 'reserved'",
        params![message_id],
    )
    .map_err(AppError::from)?;
    Ok(())
}

fn persist_pending_ack(
    db: &DbState,
    message_id: &str,
    ack: &RelayAckV1,
    now: i64,
) -> AppResult<()> {
    let ack_json = serde_json::to_string(ack)
        .map_err(|error| AppError::Internal(format!("serialize relay ack: {}", error)))?;
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    conn.execute(
        "UPDATE clipboard_relay_receipts
         SET state = 'copied_pending_ack', ack_json = ?2, updated_at = ?3
         WHERE message_id = ?1",
        params![message_id, ack_json, now],
    )
    .map_err(AppError::from)?;
    Ok(())
}

fn mark_receipt_acked(db: &DbState, message_id: &str, now: i64) -> AppResult<()> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    conn.execute(
        "UPDATE clipboard_relay_receipts SET state = 'acked', updated_at = ?2
         WHERE message_id = ?1",
        params![message_id, now],
    )
    .map_err(AppError::from)?;
    Ok(())
}

fn pending_ack(db: &DbState, message_id: &str) -> AppResult<Option<RelayAckV1>> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::Internal("database lock poisoned".to_string()))?;
    let result: Result<(String, String), rusqlite::Error> = conn.query_row(
        "SELECT state, ack_json FROM clipboard_relay_receipts WHERE message_id = ?1",
        params![message_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    match result {
        Ok((state, ack_json)) if state == "copied_pending_ack" => serde_json::from_str(&ack_json)
            .map(Some)
            .map_err(|_| AppError::Internal("stored relay ack is invalid".to_string())),
        Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(AppError::from(error)),
    }
}

async fn delete_resource_best_effort(client: &Client, config: &RelayConfig, relative_path: &str) {
    let Ok(url) = resource_url(config, relative_path) else {
        return;
    };
    let _ = send_with_retry(|| with_auth(client.delete(&url), config)).await;
}

pub async fn send_current_clipboard(app: &AppHandle) -> AppResult<RelaySendResult> {
    let _guard = relay_run_lock()
        .try_lock()
        .map_err(|_| AppError::Validation("剪贴板接力正在执行，请稍后重试".to_string()))?;
    let text = crate::services::clipboard_ops::read_plain_text_exact()?;
    let byte_len = validate_text(&text)?;
    let config = get_config(app)?;
    let client = build_http_client()?;
    let paths = ensure_relay_directories(&client, &config).await?;
    let now = now_ms();
    let (existing, removed) = split_remote_queue(
        list_message_refs(&client, &config, &paths.messages).await?,
        now,
        RELAY_MAX_QUEUE_MESSAGES.saturating_sub(1),
    );
    for message_ref in removed {
        delete_resource_best_effort(
            &client,
            &config,
            &format!("{}/{}", paths.messages, message_ref.file_name),
        )
        .await;
    }
    debug_assert!(existing.len() < RELAY_MAX_QUEUE_MESSAGES);

    let created_at = now;
    let expires_at = created_at.saturating_add(RELAY_DEFAULT_TTL_MS);
    let message_id = Uuid::new_v4().to_string();
    let file_name = format!("{}__{}.json", created_at, message_id);
    let mut message = RelayTextMessageV1 {
        schema_version: RELAY_SCHEMA_VERSION,
        message_id: message_id.clone(),
        sender_device_id: config.device_id.clone(),
        target_device_ids: vec![RELAY_BROADCAST_TARGET.to_string()],
        created_at,
        expires_at,
        content_type: RELAY_CONTENT_TYPE.to_string(),
        encryption: "xchacha20poly1305".to_string(),
        nonce: String::new(),
        ciphertext: String::new(),
    };
    let (nonce, ciphertext) = encrypt_payload(
        text.as_bytes(),
        &message_aad(&message, &file_name),
        &config.shared_key,
    )?;
    message.nonce = nonce;
    message.ciphertext = ciphertext;
    create_json_resource(
        &client,
        &config,
        &format!("{}/{}", paths.messages, file_name),
        &message,
    )
    .await?;

    Ok(RelaySendResult {
        message_id,
        created_at,
        expires_at,
        byte_len,
    })
}

pub async fn fetch_latest_to_clipboard(app: &AppHandle) -> AppResult<RelayFetchResult> {
    let _guard = relay_run_lock()
        .try_lock()
        .map_err(|_| AppError::Validation("剪贴板接力正在执行，请稍后重试".to_string()))?;
    let config = get_config(app)?;
    let client = build_http_client()?;
    let paths = ensure_relay_directories(&client, &config).await?;
    let now = now_ms();
    let (message_refs, removed) = split_remote_queue(
        list_message_refs(&client, &config, &paths.messages).await?,
        now,
        RELAY_MAX_QUEUE_MESSAGES,
    );
    for message_ref in removed {
        delete_resource_best_effort(
            &client,
            &config,
            &format!("{}/{}", paths.messages, message_ref.file_name),
        )
        .await;
        delete_resource_best_effort(
            &client,
            &config,
            &format!("{}/{}", paths.recipient_acks, message_ref.file_name),
        )
        .await;
    }
    let db = app
        .try_state::<DbState>()
        .ok_or_else(|| AppError::Internal("database state unavailable".to_string()))?;
    prune_receipts(&db, now)?;

    for message_ref in message_refs {
        let message_path = format!("{}/{}", paths.messages, message_ref.file_name);
        let ack_path = format!("{}/{}", paths.recipient_acks, message_ref.file_name);
        if let Some(ack) = pending_ack(&db, &message_ref.message_id)? {
            if create_json_resource(&client, &config, &ack_path, &ack)
                .await
                .is_ok()
            {
                mark_receipt_acked(&db, &message_ref.message_id, now_ms())?;
                continue;
            }
            return Ok(RelayFetchResult {
                outcome: "pending_ack_retry_failed".to_string(),
                message_id: Some(message_ref.message_id),
                sender_device_id: None,
                created_at: Some(message_ref.created_at),
                expires_at: None,
                acked: false,
            });
        }
        if receipt_state(&db, &message_ref.message_id)?.is_some() {
            continue;
        }
        if message_ref.created_at < now.saturating_sub(RELAY_MAX_TTL_MS + RELAY_CLOCK_SKEW_MS) {
            delete_resource_best_effort(&client, &config, &message_path).await;
            delete_resource_best_effort(&client, &config, &ack_path).await;
            continue;
        }
        if let Some(ack_body) =
            fetch_resource(&client, &config, &ack_path, RELAY_MAX_JSON_BYTES).await?
        {
            let ack: RelayAckV1 = serde_json::from_slice(&ack_body).map_err(|_| {
                AppError::Validation("clipboard relay ack is malformed".to_string())
            })?;
            validate_ack(
                &ack,
                &message_ref.message_id,
                &config.device_id,
                &message_ref.file_name,
                &config.shared_key,
            )?;
            continue;
        }
        let Some(body) =
            fetch_resource(&client, &config, &message_path, RELAY_MAX_JSON_BYTES).await?
        else {
            continue;
        };
        let message: RelayTextMessageV1 = serde_json::from_slice(&body).map_err(|_| {
            AppError::Validation("clipboard relay message is malformed".to_string())
        })?;
        if message_is_locally_ineligible(&message, &config.device_id, now) {
            continue;
        }
        let Some(content) = validate_incoming_message(
            &message,
            &message_ref,
            &config.device_id,
            &config.shared_key,
            now,
        )?
        else {
            continue;
        };

        if !reserve_receipt(&db, &message, now)? {
            continue;
        }
        if let Err(error) = crate::services::clipboard_ops::set_plain_text_from_app(&content).await
        {
            remove_reserved_receipt(&db, &message.message_id)?;
            return Err(error);
        }
        let ack = create_ack(
            &message.message_id,
            &config.device_id,
            &message_ref.file_name,
            now_ms(),
            &config.shared_key,
        )?;
        persist_pending_ack(&db, &message.message_id, &ack, now_ms())?;
        let acked = create_json_resource(&client, &config, &ack_path, &ack)
            .await
            .is_ok();
        if acked {
            mark_receipt_acked(&db, &message.message_id, now_ms())?;
        }
        return Ok(RelayFetchResult {
            outcome: if acked {
                "copied".to_string()
            } else {
                "copied_but_ack_failed".to_string()
            },
            message_id: Some(message.message_id),
            sender_device_id: Some(message.sender_device_id),
            created_at: Some(message.created_at),
            expires_at: Some(message.expires_at),
            acked,
        });
    }

    Ok(RelayFetchResult {
        outcome: "empty".to_string(),
        message_id: None,
        sender_device_id: None,
        created_at: None,
        expires_at: None,
        acked: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [7u8; 32]
    }

    fn valid_message(now: i64) -> (RelayTextMessageV1, RelayMessageRef, String) {
        let message_id = Uuid::new_v4().to_string();
        let content = "  keep\r\nwhitespace  ".to_string();
        let reference = RelayMessageRef {
            created_at: now - 1_000,
            message_id: message_id.clone(),
            file_name: format!("{}__{}.json", now - 1_000, message_id),
        };
        let mut message = RelayTextMessageV1 {
            schema_version: RELAY_SCHEMA_VERSION,
            message_id: message_id.clone(),
            sender_device_id: "sender-1".to_string(),
            target_device_ids: vec![RELAY_BROADCAST_TARGET.to_string()],
            created_at: reference.created_at,
            expires_at: now + 60_000,
            content_type: RELAY_CONTENT_TYPE.to_string(),
            encryption: "xchacha20poly1305".to_string(),
            nonce: String::new(),
            ciphertext: String::new(),
        };
        let (nonce, ciphertext) = encrypt_payload(
            content.as_bytes(),
            &message_aad(&message, &reference.file_name),
            &test_key(),
        )
        .unwrap();
        message.nonce = nonce;
        message.ciphertext = ciphertext;
        (message, reference, content)
    }

    #[test]
    fn text_validation_preserves_whitespace_and_enforces_byte_limit() {
        assert_eq!(validate_text("   ").unwrap(), 3);
        assert!(validate_text("").is_err());
        assert!(validate_text(&"x".repeat(RELAY_MAX_TEXT_BYTES)).is_ok());
        assert!(validate_text(&"x".repeat(RELAY_MAX_TEXT_BYTES + 1)).is_err());
    }

    #[test]
    fn message_refs_are_validated_deduplicated_and_sorted() {
        let older = Uuid::new_v4().to_string();
        let newer = Uuid::new_v4().to_string();
        let xml = format!(
            "<d:multistatus><d:href>/relay/10__{older}.json</d:href><d:href>/relay/20__{newer}.json</d:href><d:href>/relay/10__{older}.json</d:href><d:href>/relay/../bad.json</d:href></d:multistatus>"
        );
        let refs = parse_message_refs(&xml);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].created_at, 20);
        assert_eq!(refs[1].created_at, 10);
    }

    #[test]
    fn remote_queue_is_bounded_and_prunes_stale_entries() {
        let now = RELAY_MAX_TTL_MS + RELAY_CLOCK_SKEW_MS + 10;
        let refs = (0..RELAY_MAX_QUEUE_MESSAGES + 2)
            .map(|index| RelayMessageRef {
                created_at: if index == 0 { 0 } else { now - index as i64 },
                message_id: Uuid::new_v4().to_string(),
                file_name: format!("{index}.json"),
            })
            .collect();
        let (kept, removed) =
            split_remote_queue(refs, now, RELAY_MAX_QUEUE_MESSAGES.saturating_sub(1));
        assert_eq!(kept.len(), RELAY_MAX_QUEUE_MESSAGES - 1);
        assert_eq!(removed.len(), 3);
        assert!(removed.iter().any(|item| item.created_at == 0));
    }

    #[test]
    fn incoming_message_authenticates_target_timing_and_ciphertext() {
        let now = 1_000_000;
        let (message, reference, content) = valid_message(now);
        assert_eq!(
            validate_incoming_message(&message, &reference, "recipient-1", &test_key(), now)
                .unwrap(),
            Some(content)
        );

        assert_eq!(
            validate_incoming_message(
                &message,
                &reference,
                "recipient-1",
                &test_key(),
                now + 120_000
            )
            .unwrap(),
            None
        );

        let mut wrong_target = message.clone();
        wrong_target.target_device_ids = vec!["recipient-2".to_string()];
        assert!(validate_incoming_message(
            &wrong_target,
            &reference,
            "recipient-1",
            &test_key(),
            now
        )
        .is_err());

        let mut corrupted = message.clone();
        corrupted.ciphertext.push('A');
        assert!(
            validate_incoming_message(&corrupted, &reference, "recipient-1", &test_key(), now)
                .is_err()
        );
        assert!(message_is_locally_ineligible(
            &wrong_target,
            "recipient-1",
            now
        ));
        assert!(!message_is_locally_ineligible(&message, "recipient-1", now));
    }

    #[test]
    fn message_and_ack_authentication_round_trip_is_strict() {
        let now = 1_000_000;
        let (message, reference, content) = valid_message(now);
        assert_eq!(
            decrypt_text(&message, &reference.file_name, &test_key()).unwrap(),
            content
        );
        assert!(decrypt_text(&message, &reference.file_name, &[2u8; 32]).is_err());

        let ack = create_ack(
            &message.message_id,
            "recipient-1",
            &reference.file_name,
            now,
            &test_key(),
        )
        .unwrap();
        validate_ack(
            &ack,
            &message.message_id,
            "recipient-1",
            &reference.file_name,
            &test_key(),
        )
        .unwrap();
        assert!(validate_ack(
            &ack,
            &message.message_id,
            "recipient-2",
            &reference.file_name,
            &test_key(),
        )
        .is_err());
    }

    #[test]
    fn message_aad_binds_file_name_and_metadata() {
        let now = 1_000_000;
        let (message, reference, _) = valid_message(now);
        assert!(decrypt_text(&message, "other.json", &test_key()).is_err());

        let mut rebound = message.clone();
        rebound.sender_device_id = "attacker".to_string();
        assert!(decrypt_text(&rebound, &reference.file_name, &test_key()).is_err());
    }

    #[test]
    fn serialized_message_has_no_plaintext_fingerprint() {
        let (message, _, _) = valid_message(1_000_000);
        let json = serde_json::to_string(&message).unwrap();
        assert!(!json.contains("sha256"));
        assert!(!json.contains("whitespace"));
    }

    #[test]
    fn relay_url_requires_https_and_rejects_fragments() {
        assert_eq!(
            validate_relay_url("https://dav.example.test/root/").unwrap(),
            "https://dav.example.test/root"
        );
        assert!(validate_relay_url("http://dav.example.test/root").is_err());
        assert!(validate_relay_url("https://dav.example.test/root#fragment").is_err());
        assert!(validate_relay_url("https://dav.example.test/root/#fragment").is_err());
        assert_eq!(normalize_base_path("/safe/root/").unwrap(), "safe/root");
        assert!(normalize_base_path("safe/../root").is_err());
    }

    #[tokio::test]
    async fn relay_http_client_rejects_http_requests() {
        let error = build_http_client()
            .expect("build HTTPS-only relay client")
            .get("http://127.0.0.1:9/redirect-target")
            .send()
            .await
            .expect_err("HTTP request must be rejected before network I/O");
        assert!(error.is_builder());
    }
}
