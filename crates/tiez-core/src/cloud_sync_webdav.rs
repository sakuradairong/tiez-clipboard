//! Tauri-independent WebDAV transport used by every TieZ desktop frontend.
//!
//! This module deliberately owns only HTTP/WebDAV behavior. Clipboard conflict
//! resolution and database mutations stay in the sync runner so native and
//! legacy frontends can share the wire contract without sharing a UI runtime.

use regex::Regex;
use reqwest::redirect::Policy;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
pub const DEFAULT_BASE_PATH: &str = "tiez-sync";
pub const HEAD_FILENAME: &str = "head.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebDavRetryPolicy {
    pub max_request_retries: usize,
    pub max_json_read_retries: usize,
    pub base_delay: Duration,
}

impl Default for WebDavRetryPolicy {
    fn default() -> Self {
        Self {
            max_request_retries: 3,
            max_json_read_retries: 3,
            base_delay: Duration::from_millis(600),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebDavTransportErrorKind {
    Configuration,
    Network,
    Protocol,
    Serialization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavTransportError {
    kind: WebDavTransportErrorKind,
    message: String,
    status_code: Option<u16>,
}

impl WebDavTransportError {
    fn new(kind: WebDavTransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status_code: None,
        }
    }

    fn http(message: impl Into<String>, status: StatusCode) -> Self {
        Self {
            kind: WebDavTransportErrorKind::Protocol,
            message: message.into(),
            status_code: Some(status.as_u16()),
        }
    }

    pub fn kind(&self) -> WebDavTransportErrorKind {
        self.kind
    }

    pub fn status_code(&self) -> Option<u16> {
        self.status_code
    }
}

impl fmt::Display for WebDavTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for WebDavTransportError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavLayout {
    pub devices_path: String,
    pub settings_path: String,
    pub ops_path: String,
    pub head_path: String,
    pub blobs_path: String,
}

impl WebDavLayout {
    pub fn for_base_path(base_path: &str) -> Self {
        let base = normalize_base_path(base_path);
        Self {
            devices_path: child_path(&base, "devices"),
            settings_path: child_path(&base, "settings"),
            ops_path: child_path(&base, "ops"),
            head_path: child_path(&base, HEAD_FILENAME),
            blobs_path: child_path(&base, "blobs"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavOpReference {
    pub device_id: String,
    pub seq: i64,
}

#[derive(Clone)]
pub struct WebDavTransport {
    client: Client,
    endpoint: String,
    username: String,
    password: String,
    device_id: String,
    retry_policy: WebDavRetryPolicy,
    known_collections: Arc<Mutex<HashSet<String>>>,
}

impl WebDavTransport {
    pub fn new(
        endpoint: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        device_id: impl Into<String>,
    ) -> Result<Self, WebDavTransportError> {
        Self::with_client(
            build_webdav_http_client()?,
            endpoint,
            username,
            password,
            device_id,
        )
    }

    pub fn with_client(
        client: Client,
        endpoint: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        device_id: impl Into<String>,
    ) -> Result<Self, WebDavTransportError> {
        let endpoint = validate_endpoint(endpoint.into())?;
        Ok(Self {
            client,
            endpoint,
            username: username.into().trim().to_owned(),
            password: password.into(),
            device_id: device_id.into(),
            retry_policy: WebDavRetryPolicy::default(),
            known_collections: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn with_retry_policy(mut self, retry_policy: WebDavRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub fn resource_url(&self, relative_path: &str) -> String {
        resource_url(&self.endpoint, relative_path)
    }

    pub fn collection_url(&self, relative_path: &str) -> String {
        collection_url(&self.endpoint, relative_path)
    }

    pub async fn put_bytes_atomic(
        &self,
        relative_path: &str,
        body: &[u8],
        content_type: &str,
        label: &str,
    ) -> Result<(), WebDavTransportError> {
        let final_url = self.resource_url(relative_path);
        let temp_relative = format!(
            "{}.uploading.{}.{}.tmp",
            relative_path.trim_end_matches('/'),
            safe_temporary_component(&self.device_id),
            now_millis()
        );
        let temp_url = self.resource_url(&temp_relative);

        self.put_target(&temp_url, body, content_type, label)
            .await?;
        match self.move_resource(&temp_relative, relative_path).await {
            Ok(true) => Ok(()),
            Ok(false) => {
                let fallback = self.put_target(&final_url, body, content_type, label).await;
                let _ = self.delete_if_exists(&temp_relative).await;
                fallback
            }
            Err(error) => {
                let _ = self.delete_if_exists(&temp_relative).await;
                Err(error)
            }
        }
    }

    pub async fn put_json_atomic<T: serde::Serialize + ?Sized>(
        &self,
        relative_path: &str,
        value: &T,
        label: &str,
    ) -> Result<(), WebDavTransportError> {
        let body = serde_json::to_vec(value).map_err(|error| {
            WebDavTransportError::new(
                WebDavTransportErrorKind::Serialization,
                format!("serialize {label} failed: {error}"),
            )
        })?;
        self.put_bytes_atomic(relative_path, &body, "application/json", label)
            .await
    }

    pub async fn get_json<T: DeserializeOwned>(
        &self,
        relative_path: &str,
        missing_status: u16,
        fetch_error_label: &str,
        parse_error_label: &str,
    ) -> Result<Option<T>, WebDavTransportError> {
        let url = self.resource_url(relative_path);
        fetch_json_with_retry(
            || self.authenticated(self.client.get(&url)),
            missing_status,
            fetch_error_label,
            parse_error_label,
            self.retry_policy,
        )
        .await
    }

    pub async fn get_bytes(
        &self,
        relative_path: &str,
        label: &str,
    ) -> Result<Vec<u8>, WebDavTransportError> {
        let url = self.resource_url(relative_path);
        let response = send_with_retry(
            || self.authenticated(self.client.get(&url)),
            self.retry_policy,
        )
        .await?;
        if !response.status().is_success() {
            return Err(response_error(label, response).await);
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| {
                WebDavTransportError::new(WebDavTransportErrorKind::Network, error.to_string())
            })
    }

    pub async fn ensure_layout(
        &self,
        base_path: &str,
    ) -> Result<WebDavLayout, WebDavTransportError> {
        let base = normalize_base_path(base_path);
        let layout = WebDavLayout::for_base_path(&base);
        let mut current = String::new();
        for segment in base.split('/').filter(|segment| !segment.is_empty()) {
            current = child_path(&current, segment);
            self.ensure_collection(&current).await?;
        }
        self.ensure_collection(&layout.devices_path).await?;
        self.ensure_collection(&layout.settings_path).await?;
        self.ensure_collection(&layout.ops_path).await?;
        self.ensure_collection(&layout.blobs_path).await?;
        Ok(layout)
    }

    pub async fn upload_blob(
        &self,
        blobs_path: &str,
        kind: &str,
        data: &[u8],
    ) -> Result<String, WebDavTransportError> {
        let hash = sha256_hex(data);
        let prefix_path = child_path(blobs_path, &hash[..2]);
        self.ensure_collection(&prefix_path).await?;
        let relative_path = blob_path(blobs_path, kind, &hash)?;
        self.put_bytes_atomic(&relative_path, data, "application/octet-stream", "blob")
            .await?;
        Ok(hash)
    }

    pub async fn download_blob(
        &self,
        blobs_path: &str,
        kind: &str,
        hash: &str,
    ) -> Result<Vec<u8>, WebDavTransportError> {
        let relative_path = blob_path(blobs_path, kind, hash)?;
        self.get_bytes(&relative_path, "webdav GET blob failed")
            .await
    }

    pub async fn list_snapshot_ids(
        &self,
        devices_path: &str,
    ) -> Result<Vec<String>, WebDavTransportError> {
        let xml = self.propfind(devices_path).await?;
        Ok(parse_snapshot_ids(&xml))
    }

    pub async fn list_op_references(
        &self,
        ops_path: &str,
    ) -> Result<Vec<WebDavOpReference>, WebDavTransportError> {
        let xml = self.propfind(ops_path).await?;
        Ok(parse_op_references(&xml))
    }

    async fn put_target(
        &self,
        url: &str,
        body: &[u8],
        content_type: &str,
        label: &str,
    ) -> Result<(), WebDavTransportError> {
        let url = url.to_owned();
        let body = body.to_vec();
        let content_type = content_type.to_owned();
        let response = send_with_retry(
            || {
                self.authenticated(
                    self.client
                        .put(&url)
                        .header("Content-Type", &content_type)
                        .body(body.clone()),
                )
            },
            self.retry_policy,
        )
        .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(response_error(&format!("webdav PUT {label} failed"), response).await)
        }
    }

    async fn move_resource(
        &self,
        from_relative: &str,
        to_relative: &str,
    ) -> Result<bool, WebDavTransportError> {
        let from_url = self.resource_url(from_relative);
        let destination = self.resource_url(to_relative);
        let response = send_with_retry(
            || {
                self.authenticated(
                    self.client
                        .request(
                            Method::from_bytes(b"MOVE").expect("valid MOVE method"),
                            &from_url,
                        )
                        .header("Destination", &destination)
                        .header("Overwrite", "T"),
                )
            },
            self.retry_policy,
        )
        .await?;
        if response.status().is_success() {
            return Ok(true);
        }
        if matches!(response.status().as_u16(), 405 | 409 | 412 | 501) {
            return Ok(false);
        }
        Err(response_error("webdav MOVE publish failed", response).await)
    }

    pub async fn delete_if_exists(&self, relative_path: &str) -> Result<(), WebDavTransportError> {
        let url = self.resource_url(relative_path);
        let response = send_with_retry(
            || self.authenticated(self.client.delete(&url)),
            self.retry_policy,
        )
        .await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(response_error("webdav DELETE cleanup failed", response).await)
        }
    }

    async fn ensure_collection(&self, relative_path: &str) -> Result<(), WebDavTransportError> {
        let cache_key = format!("{}:{}", self.endpoint, relative_path);
        if self
            .known_collections
            .lock()
            .expect("WebDAV collection cache poisoned")
            .contains(&cache_key)
        {
            return Ok(());
        }

        let url = self.collection_url(relative_path);
        let response = send_with_retry(
            || {
                self.authenticated(self.client.request(
                    Method::from_bytes(b"MKCOL").expect("valid MKCOL method"),
                    &url,
                ))
            },
            self.retry_policy,
        )
        .await?;
        let status = response.status();
        let exists = if status.is_success() {
            true
        } else if matches!(status.as_u16(), 301 | 302 | 307 | 308 | 405 | 409) {
            self.collection_exists(relative_path).await?
        } else {
            return Err(response_error("webdav MKCOL failed", response).await);
        };

        if exists {
            self.known_collections
                .lock()
                .expect("WebDAV collection cache poisoned")
                .insert(cache_key);
            Ok(())
        } else {
            Err(WebDavTransportError::new(
                WebDavTransportErrorKind::Protocol,
                format!("WebDAV collection was not created: {relative_path}"),
            ))
        }
    }

    async fn collection_exists(&self, relative_path: &str) -> Result<bool, WebDavTransportError> {
        let url = self.collection_url(relative_path);
        let response = send_with_retry(
            || {
                self.authenticated(
                    self.client
                        .request(
                            Method::from_bytes(b"PROPFIND").expect("valid PROPFIND method"),
                            &url,
                        )
                        .header("Depth", "0")
                        .header("Content-Type", "application/xml; charset=utf-8"),
                )
            },
            self.retry_policy,
        )
        .await?;
        Ok(response.status().is_success() || response.status().as_u16() == 207)
    }

    async fn propfind(&self, relative_path: &str) -> Result<String, WebDavTransportError> {
        let url = self.collection_url(relative_path);
        let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:"><d:prop><d:getlastmodified /></d:prop></d:propfind>"#;
        let response = send_with_retry(
            || {
                self.authenticated(
                    self.client
                        .request(
                            Method::from_bytes(b"PROPFIND").expect("valid PROPFIND method"),
                            &url,
                        )
                        .header("Depth", "1")
                        .header("Content-Type", "application/xml; charset=utf-8")
                        .body(body),
                )
            },
            self.retry_policy,
        )
        .await?;
        if !response.status().is_success() && response.status().as_u16() != 207 {
            return Err(response_error("webdav PROPFIND failed", response).await);
        }
        response.text().await.map_err(|error| {
            WebDavTransportError::new(WebDavTransportErrorKind::Network, error.to_string())
        })
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        if self.username.is_empty() {
            request
        } else {
            request.basic_auth(&self.username, Some(&self.password))
        }
    }
}

pub fn build_webdav_http_client() -> Result<Client, WebDavTransportError> {
    build_webdav_http_client_with_timeout(DEFAULT_REQUEST_TIMEOUT)
}

pub fn build_webdav_http_client_with_timeout(
    timeout: Duration,
) -> Result<Client, WebDavTransportError> {
    Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .build()
        .map_err(|error| {
            WebDavTransportError::new(WebDavTransportErrorKind::Network, error.to_string())
        })
}

pub async fn send_with_retry<F>(
    mut make_request: F,
    policy: WebDavRetryPolicy,
) -> Result<Response, WebDavTransportError>
where
    F: FnMut() -> RequestBuilder,
{
    let mut last_error = None;
    for attempt in 0..=policy.max_request_retries {
        match make_request().send().await {
            Ok(response) => {
                if is_retryable_status(response.status()) && attempt < policy.max_request_retries {
                    last_error = Some(format!("transient WebDAV status {}", response.status()));
                    sleep(retry_delay(policy, attempt)).await;
                    continue;
                }
                return Ok(response);
            }
            Err(error) => {
                last_error = Some(error.to_string());
                if attempt < policy.max_request_retries {
                    sleep(retry_delay(policy, attempt)).await;
                }
            }
        }
    }
    Err(WebDavTransportError::new(
        WebDavTransportErrorKind::Network,
        last_error.unwrap_or_else(|| "webdav request failed".to_owned()),
    ))
}

pub async fn fetch_json_with_retry<T, F>(
    mut make_request: F,
    missing_status: u16,
    fetch_error_label: &str,
    parse_error_label: &str,
    policy: WebDavRetryPolicy,
) -> Result<Option<T>, WebDavTransportError>
where
    T: DeserializeOwned,
    F: FnMut() -> RequestBuilder,
{
    for attempt in 0..=policy.max_json_read_retries {
        let response = send_with_retry(|| make_request(), policy).await?;
        let status = response.status();
        if status.as_u16() == missing_status || status == StatusCode::CONFLICT {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(response_error(fetch_error_label, response).await);
        }
        let bytes = response.bytes().await.map_err(|error| {
            WebDavTransportError::new(WebDavTransportErrorKind::Network, error.to_string())
        })?;
        match serde_json::from_slice::<T>(&bytes) {
            Ok(value) => return Ok(Some(value)),
            Err(error)
                if matches!(error.classify(), serde_json::error::Category::Eof)
                    && attempt < policy.max_json_read_retries =>
            {
                sleep(retry_delay(policy, attempt)).await;
            }
            Err(error) => {
                return Err(WebDavTransportError::new(
                    WebDavTransportErrorKind::Serialization,
                    format!("{parse_error_label}: {error}"),
                ));
            }
        }
    }
    Err(WebDavTransportError::new(
        WebDavTransportErrorKind::Serialization,
        format!("{parse_error_label}: exhausted retries"),
    ))
}

pub fn encode_relative_path(relative_path: &str, collection: bool) -> String {
    let mut encoded = relative_path
        .replace('\\', "/")
        .split('/')
        .filter_map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() {
                None
            } else if matches!(segment, "." | "..") {
                Some(segment.replace('.', "%2E"))
            } else {
                Some(urlencoding::encode(segment).into_owned())
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    if collection && !encoded.is_empty() {
        encoded.push('/');
    }
    encoded
}

pub fn resource_url(endpoint: &str, relative_path: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let encoded = encode_relative_path(relative_path, false);
    if encoded.is_empty() {
        endpoint.to_owned()
    } else {
        format!("{endpoint}/{encoded}")
    }
}

pub fn collection_url(endpoint: &str, relative_path: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let encoded = encode_relative_path(relative_path, true);
    if encoded.is_empty() {
        format!("{endpoint}/")
    } else {
        format!("{endpoint}/{encoded}")
    }
}

pub fn parse_snapshot_ids(xml: &str) -> Vec<String> {
    hrefs(xml)
        .into_iter()
        .filter_map(|href| {
            href.trim_end_matches('/')
                .rsplit('/')
                .next()
                .and_then(|name| name.strip_suffix(".json"))
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        })
        .fold(Vec::new(), |mut ids, id| {
            if !ids.contains(&id) {
                ids.push(id);
            }
            ids
        })
}

pub fn parse_op_references(xml: &str) -> Vec<WebDavOpReference> {
    let file_pattern = Regex::new(r"^(.+)__(\d+)\.json$").expect("valid op filename regex");
    let mut references = HashMap::new();
    for href in hrefs(xml) {
        let Some(file_name) = href.trim_end_matches('/').rsplit('/').next() else {
            continue;
        };
        let Some(captures) = file_pattern.captures(file_name) else {
            continue;
        };
        let Some(device_id) = captures.get(1).map(|value| value.as_str().to_owned()) else {
            continue;
        };
        let Some(seq) = captures
            .get(2)
            .and_then(|value| value.as_str().parse::<i64>().ok())
        else {
            continue;
        };
        references
            .entry((device_id.clone(), seq))
            .or_insert(WebDavOpReference { device_id, seq });
    }
    let mut references = references.into_values().collect::<Vec<_>>();
    references.sort_by(|left, right| {
        left.device_id
            .cmp(&right.device_id)
            .then(left.seq.cmp(&right.seq))
    });
    references
}

fn hrefs(xml: &str) -> Vec<String> {
    let pattern = Regex::new(r"(?is)<[^>]*href[^>]*>\s*([^<]+)\s*</[^>]*href>")
        .expect("valid WebDAV href regex");
    pattern
        .captures_iter(xml)
        .filter_map(|captures| {
            let raw = captures.get(1)?.as_str().trim();
            if raw.is_empty() {
                return None;
            }
            Some(
                urlencoding::decode(raw)
                    .map(|value| value.into_owned())
                    .unwrap_or_else(|_| raw.to_owned()),
            )
        })
        .collect()
}

fn validate_endpoint(endpoint: String) -> Result<String, WebDavTransportError> {
    let endpoint = endpoint.trim().trim_end_matches('/').to_owned();
    let parsed = Url::parse(&endpoint).map_err(|_| {
        WebDavTransportError::new(
            WebDavTransportErrorKind::Configuration,
            "WebDAV endpoint must be an absolute HTTP or HTTPS URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(WebDavTransportError::new(
            WebDavTransportErrorKind::Configuration,
            "WebDAV endpoint must be an absolute HTTP or HTTPS URL",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(WebDavTransportError::new(
            WebDavTransportErrorKind::Configuration,
            "WebDAV endpoint must not embed credentials",
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(WebDavTransportError::new(
            WebDavTransportErrorKind::Configuration,
            "WebDAV endpoint must not contain a query string or fragment",
        ));
    }
    Ok(endpoint)
}

fn normalize_base_path(base_path: &str) -> String {
    let base_path = base_path.trim().trim_matches('/');
    if base_path.is_empty() {
        DEFAULT_BASE_PATH.to_owned()
    } else {
        base_path.to_owned()
    }
}

fn child_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn blob_path(blobs_path: &str, kind: &str, hash: &str) -> Result<String, WebDavTransportError> {
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(WebDavTransportError::new(
            WebDavTransportErrorKind::Configuration,
            "invalid WebDAV blob identity",
        ));
    }
    Ok(format!(
        "{}/{}/{}_{}.blob",
        blobs_path.trim_end_matches('/'),
        &hash[..2],
        kind,
        hash.to_ascii_lowercase()
    ))
}

fn safe_temporary_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "device".to_owned()
    } else {
        value
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn retry_delay(policy: WebDavRetryPolicy, attempt: usize) -> Duration {
    let factor = 1u32 << attempt.min(4);
    policy.base_delay.saturating_mul(factor)
}

async fn response_error(label: &str, response: Response) -> WebDavTransportError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body = truncate_body(&body);
    WebDavTransportError::http(format!("{label}: {status} {body}"), status)
}

fn truncate_body(body: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 2048;
    let mut chars = body.chars();
    let truncated = chars
        .by_ref()
        .take(MAX_ERROR_BODY_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    #[derive(Debug)]
    struct CapturedRequest {
        head: String,
        body: Vec<u8>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct TestDocument {
        value: String,
    }

    fn fast_policy() -> WebDavRetryPolicy {
        WebDavRetryPolicy {
            max_request_retries: 1,
            max_json_read_retries: 1,
            base_delay: Duration::from_millis(1),
        }
    }

    fn spawn_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, thread::JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            responses
                .into_iter()
                .map(|(status, body)| {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    request
                })
                .collect()
        });
        (format!("http://{address}/dav"), server)
    }

    fn read_request(stream: &mut TcpStream) -> CapturedRequest {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        let header_end;
        loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = position + 4;
                break;
            }
        }
        let head = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
        let content_length = head
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
        }
        CapturedRequest {
            head,
            body: bytes[header_end..header_end + content_length].to_vec(),
        }
    }

    #[test]
    fn paths_are_segment_encoded_and_dot_segments_cannot_escape() {
        assert_eq!(
            resource_url("https://dav.example/root/", "设备 A/../head.json"),
            "https://dav.example/root/%E8%AE%BE%E5%A4%87%20A/%2E%2E/head.json"
        );
        assert_eq!(
            collection_url("https://dav.example/root", "tiez sync/devices"),
            "https://dav.example/root/tiez%20sync/devices/"
        );
    }

    #[test]
    fn list_parsers_decode_deduplicate_and_sort_protocol_names() {
        let xml = r#"
            <d:href>/dav/devices/device%20one.json</d:href>
            <d:href>/dav/devices/device%20one.json</d:href>
            <d:href>/dav/devices/device-two.json</d:href>
            <d:href>/dav/ops/z__00000000000000000002.json</d:href>
            <d:href>/dav/ops/a__00000000000000000003.json</d:href>
        "#;
        assert_eq!(
            parse_snapshot_ids(xml),
            vec![
                "device one",
                "device-two",
                "z__00000000000000000002",
                "a__00000000000000000003"
            ]
        );
        assert_eq!(
            parse_op_references(xml),
            vec![
                WebDavOpReference {
                    device_id: "a".to_owned(),
                    seq: 3,
                },
                WebDavOpReference {
                    device_id: "z".to_owned(),
                    seq: 2,
                },
            ]
        );
    }

    #[tokio::test]
    async fn atomic_put_uses_authenticated_temporary_move() {
        let (endpoint, server) = spawn_server(vec![("201 Created", ""), ("201 Created", "")]);
        let transport = WebDavTransport::new(&endpoint, "alice", "secret", "设备/one")
            .unwrap()
            .with_retry_policy(fast_policy());
        transport
            .put_bytes_atomic("sync/head.json", b"payload", "application/json", "head")
            .await
            .unwrap();

        let requests = server.join().unwrap();
        assert!(requests[0]
            .head
            .starts_with("PUT /dav/sync/head.json.uploading."));
        assert!(requests[0]
            .head
            .to_ascii_lowercase()
            .contains("authorization: basic ywxpy2u6c2vjcmv0"));
        assert_eq!(requests[0].body, b"payload");
        assert!(requests[1]
            .head
            .starts_with("MOVE /dav/sync/head.json.uploading."));
        assert!(requests[1]
            .head
            .to_ascii_lowercase()
            .contains("destination: http://127.0.0.1:"));
        assert!(requests[1].head.contains("/dav/sync/head.json"));
    }

    #[tokio::test]
    async fn atomic_put_falls_back_and_cleans_up_when_move_is_unsupported() {
        let (endpoint, server) = spawn_server(vec![
            ("201 Created", ""),
            ("405 Method Not Allowed", ""),
            ("204 No Content", ""),
            ("204 No Content", ""),
        ]);
        let transport = WebDavTransport::new(&endpoint, "", "", "device")
            .unwrap()
            .with_retry_policy(fast_policy());
        transport
            .put_bytes_atomic("ops/device__1.json", b"{}", "application/json", "ops")
            .await
            .unwrap();

        let requests = server.join().unwrap();
        assert!(requests[0]
            .head
            .starts_with("PUT /dav/ops/device__1.json.uploading."));
        assert!(requests[1]
            .head
            .starts_with("MOVE /dav/ops/device__1.json.uploading."));
        assert!(requests[2]
            .head
            .starts_with("PUT /dav/ops/device__1.json HTTP/1.1"));
        assert!(requests[3]
            .head
            .starts_with("DELETE /dav/ops/device__1.json.uploading."));
    }

    #[tokio::test]
    async fn json_reads_retry_an_incomplete_atomic_view() {
        let (endpoint, server) = spawn_server(vec![
            ("200 OK", "{\"value\":"),
            ("200 OK", "{\"value\":\"ready\"}"),
        ]);
        let transport = WebDavTransport::new(&endpoint, "", "", "device")
            .unwrap()
            .with_retry_policy(fast_policy());
        let document = transport
            .get_json::<TestDocument>("head.json", 404, "fetch failed", "parse failed")
            .await
            .unwrap();
        assert_eq!(
            document,
            Some(TestDocument {
                value: "ready".to_owned()
            })
        );
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn webdav_client_does_not_follow_redirects_with_basic_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/credential-target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            request
        });
        let transport =
            WebDavTransport::new(format!("http://{address}/dav"), "alice", "secret", "device")
                .unwrap()
                .with_retry_policy(fast_policy());

        let error = transport
            .get_bytes("head.json", "redirect rejected")
            .await
            .unwrap_err();
        assert_eq!(error.status_code(), Some(302));
        assert!(server
            .join()
            .unwrap()
            .head
            .to_ascii_lowercase()
            .contains("authorization: basic ywxpy2u6c2vjcmv0"));
    }

    #[test]
    fn blob_identity_rejects_path_injection() {
        assert!(blob_path("blobs", "../image", &"a".repeat(64)).is_err());
        assert!(blob_path("blobs", "image", "../not-a-hash").is_err());
    }
}
