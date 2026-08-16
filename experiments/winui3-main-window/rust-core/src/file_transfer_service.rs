//! Authenticated LAN transfer server owned by the WinUI runtime.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use qrcode::QrCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Cursor, SeekFrom};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiez_core::file_transfer::{
    allocate_receive_path, classify_file_name, validate_chunk, validate_transfer_text,
    ChunkMetadata, FileTransferPreferences, FileTransferPreferencesSnapshot,
    FileTransferPreferencesUpdate, TransferMessage, TransferMessageStore, MAX_CHUNK_BYTES,
    MAX_FILE_BYTES,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const AUTO_CLOSE_AFTER: Duration = Duration::from_secs(5 * 60);
const DEVICE_ACTIVE_FOR: Duration = Duration::from_secs(30);
const DIRECT_UPLOAD_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum ReceivedTransfer {
    Text {
        content: String,
        sender_name: String,
    },
    File {
        path: PathBuf,
        sender_name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OnlineDevice {
    pub id: String,
    pub name: String,
    pub last_seen: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeFileTransferStatus {
    pub state: String,
    pub enabled: bool,
    pub port: u16,
    pub ip: String,
    pub pairing_url: Option<String>,
    pub qr_png_base64: Option<String>,
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub last_error: Option<String>,
}

impl NativeFileTransferStatus {
    fn stopped() -> Self {
        Self {
            state: "stopped".to_owned(),
            enabled: false,
            port: 0,
            ip: String::new(),
            pairing_url: None,
            qr_png_base64: None,
            started_at: None,
            last_activity_at: None,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileTransferSnapshot {
    pub preferences: FileTransferPreferencesSnapshot,
    pub status: NativeFileTransferStatus,
    pub messages: Vec<TransferMessage>,
    pub devices: Vec<OnlineDevice>,
}

struct UploadSession {
    temp_path: PathBuf,
    final_path: PathBuf,
    next_index: usize,
    total_chunks: usize,
    total_size: u64,
    written: u64,
    file_name: String,
    sender_id: String,
    sender_name: String,
}

struct SharedState {
    pairing_token: String,
    pairing_url: String,
    preferences: FileTransferPreferencesSnapshot,
    messages: Mutex<TransferMessageStore>,
    devices: Mutex<HashMap<String, OnlineDevice>>,
    shares: Mutex<HashMap<String, PathBuf>>,
    uploads: Mutex<HashMap<String, UploadSession>>,
    last_activity_epoch: AtomicU64,
    on_received: Arc<dyn Fn(ReceivedTransfer) -> Result<(), String> + Send + Sync>,
}

impl SharedState {
    fn touch(&self) {
        self.last_activity_epoch
            .store(now_epoch(), Ordering::Release);
    }

    fn note_device(&self, id: &str, name: &str) {
        let id = bounded_identity(id, "mobile");
        let name = bounded_identity(name, "手机");
        if let Ok(mut devices) = self.devices.lock() {
            devices.insert(
                id.clone(),
                OnlineDevice {
                    id,
                    name,
                    last_seen: now_epoch() as i64,
                },
            );
        }
    }

    fn push_message(&self, message: TransferMessage) -> TransferMessage {
        self.messages
            .lock()
            .map(|mut messages| messages.push(message.clone()))
            .unwrap_or(message)
    }

    fn online_devices(&self) -> Vec<OnlineDevice> {
        let cutoff = now_epoch().saturating_sub(DEVICE_ACTIVE_FOR.as_secs()) as i64;
        self.devices
            .lock()
            .map(|devices| {
                devices
                    .values()
                    .filter(|device| device.last_seen >= cutoff)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct RuntimeContext {
    stop: Arc<AtomicBool>,
    shared: Arc<SharedState>,
    worker: JoinHandle<()>,
}

pub struct NativeFileTransferService {
    preferences: Mutex<FileTransferPreferences>,
    status: Arc<Mutex<NativeFileTransferStatus>>,
    runtime: Mutex<Option<RuntimeContext>>,
    on_received: Arc<dyn Fn(ReceivedTransfer) -> Result<(), String> + Send + Sync>,
}

impl NativeFileTransferService {
    pub fn new(
        preferences: FileTransferPreferences,
        on_received: impl Fn(ReceivedTransfer) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            preferences: Mutex::new(preferences),
            status: Arc::new(Mutex::new(NativeFileTransferStatus::stopped())),
            runtime: Mutex::new(None),
            on_received: Arc::new(on_received),
        }
    }

    pub fn snapshot(&self) -> Result<FileTransferSnapshot, String> {
        let preferences = self
            .preferences
            .lock()
            .map_err(|_| "文件传输设置锁已损坏".to_owned())?
            .snapshot()
            .map_err(|error| error.to_string())?;
        let mut status = self
            .status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "文件传输运行时锁已损坏".to_owned())?;
        let (messages, mut devices) = if let Some(runtime) = runtime.as_ref() {
            status.last_activity_at =
                Some(runtime.shared.last_activity_epoch.load(Ordering::Acquire) as i64);
            let messages = runtime
                .shared
                .messages
                .lock()
                .map(|messages| messages.since(0))
                .unwrap_or_default();
            let devices = runtime.shared.online_devices();
            (messages, devices)
        } else {
            (Vec::new(), Vec::new())
        };
        devices.sort_by(|left: &OnlineDevice, right| right.last_seen.cmp(&left.last_seen));
        Ok(FileTransferSnapshot {
            preferences,
            status,
            messages,
            devices,
        })
    }

    pub fn update_preferences(
        &self,
        update: FileTransferPreferencesUpdate,
    ) -> Result<FileTransferSnapshot, String> {
        let was_enabled = self.snapshot()?.status.enabled;
        let requested_enabled = update.enabled;
        {
            let mut preferences = self
                .preferences
                .lock()
                .map_err(|_| "文件传输设置锁已损坏".to_owned())?;
            preferences
                .update(update)
                .map_err(|error| error.to_string())?;
        }
        match requested_enabled {
            Some(true) if !was_enabled => self.start()?,
            Some(false) if was_enabled => self.stop(),
            _ => {}
        }
        self.snapshot()
    }

    pub fn start(&self) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "文件传输运行时锁已损坏".to_owned())?;
        if runtime
            .as_ref()
            .is_some_and(|context| context.worker.is_finished())
        {
            if let Some(context) = runtime.take() {
                let _ = context.worker.join();
                cleanup_uploads(&context.shared);
            }
        }
        if runtime.is_some() {
            return Ok(());
        }
        let preferences = self
            .preferences
            .lock()
            .map_err(|_| "文件传输设置锁已损坏".to_owned())?
            .snapshot()
            .map_err(|error| error.to_string())?;
        if preferences.read_only {
            return Err("当前数据库为只读，不能启动局域网文件传输".to_owned());
        }
        let ip = preferred_local_ip();
        let pairing_token = Uuid::new_v4().simple().to_string();
        let pairing_url = format!("http://{ip}:{}/?token={pairing_token}", preferences.port);
        let qr_png_base64 = qr_png_base64(&pairing_url)?;
        let shared = Arc::new(SharedState {
            pairing_token,
            pairing_url: pairing_url.clone(),
            preferences: preferences.clone(),
            messages: Mutex::new(TransferMessageStore::default()),
            devices: Mutex::new(HashMap::new()),
            shares: Mutex::new(HashMap::new()),
            uploads: Mutex::new(HashMap::new()),
            last_activity_epoch: AtomicU64::new(now_epoch()),
            on_received: Arc::clone(&self.on_received),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let thread_status = Arc::clone(&self.status);
        update_status(&self.status, |status| {
            *status = NativeFileTransferStatus {
                state: "starting".to_owned(),
                enabled: true,
                port: preferences.port,
                ip: ip.clone(),
                pairing_url: Some(pairing_url),
                qr_png_base64: Some(qr_png_base64),
                started_at: None,
                last_activity_at: Some(now_epoch() as i64),
                last_error: None,
            };
        });
        let worker = thread::Builder::new()
            .name("tiez-winui-file-transfer".to_owned())
            .spawn(move || run_server(thread_shared, thread_stop, thread_status))
            .map_err(|error| format!("无法创建文件传输线程：{error}"))?;
        *runtime = Some(RuntimeContext {
            stop,
            shared,
            worker,
        });
        drop(runtime);

        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            let status = self
                .status
                .lock()
                .map(|status| status.clone())
                .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
            match status.state.as_str() {
                "running" => return Ok(()),
                "error" => {
                    let error = status
                        .last_error
                        .unwrap_or_else(|| "文件传输启动失败".to_owned());
                    self.stop();
                    set_server_error(&self.status, error.clone());
                    return Err(error);
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
        self.stop();
        set_server_error(&self.status, "文件传输启动超时".to_owned());
        Err("文件传输启动超时".to_owned())
    }

    pub fn stop(&self) {
        let context = self
            .runtime
            .lock()
            .ok()
            .and_then(|mut runtime| runtime.take());
        if let Some(context) = context {
            context.stop.store(true, Ordering::Release);
            let _ = context.worker.join();
            cleanup_uploads(&context.shared);
        }
        update_status(&self.status, |status| {
            status.state = "stopped".to_owned();
            status.enabled = false;
            status.pairing_url = None;
            status.qr_png_base64 = None;
        });
    }

    pub fn send_text(&self, content: &str) -> Result<FileTransferSnapshot, String> {
        validate_transfer_text(content).map_err(|error| error.to_string())?;
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "文件传输运行时锁已损坏".to_owned())?;
        let shared = runtime
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.shared))
            .ok_or_else(|| "请先启动局域网文件传输".to_owned())?;
        shared.touch();
        shared.push_message(new_message("out", "text", content, "pc", "TieZ 电脑", None));
        drop(runtime);
        self.snapshot()
    }

    pub fn share_files(&self, paths: Vec<PathBuf>) -> Result<FileTransferSnapshot, String> {
        if paths.is_empty() || paths.len() > 50 {
            return Err("请选择 1 到 50 个文件".to_owned());
        }
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "文件传输运行时锁已损坏".to_owned())?;
        let shared = runtime
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.shared))
            .ok_or_else(|| "请先启动局域网文件传输".to_owned())?;
        for path in paths {
            let metadata = std::fs::metadata(&path)
                .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
            if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES {
                return Err(format!("文件大小无效：{}", path.display()));
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("未命名文件")
                .to_owned();
            let share_id = Uuid::new_v4().simple().to_string();
            let download_url = format!(
                "{}/download/{}?token={}",
                shared
                    .pairing_url
                    .split('?')
                    .next()
                    .unwrap_or(&shared.pairing_url),
                share_id,
                shared.pairing_token
            );
            shared
                .shares
                .lock()
                .map_err(|_| "共享文件锁已损坏".to_owned())?
                .insert(share_id, path);
            shared.push_message(new_message(
                "out",
                classify_file_name(&name).message_type(),
                &name,
                "pc",
                "TieZ 电脑",
                Some(download_url),
            ));
        }
        shared.touch();
        drop(runtime);
        self.snapshot()
    }
}

impl Drop for NativeFileTransferService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_server(
    shared: Arc<SharedState>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<NativeFileTransferStatus>>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            set_server_error(&status, format!("无法创建文件传输运行时：{error}"));
            return;
        }
    };
    runtime.block_on(async move {
        let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, shared.preferences.port));
        let listener = match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                set_server_error(
                    &status,
                    format!("无法监听端口 {}：{error}", shared.preferences.port),
                );
                return;
            }
        };
        update_status(&status, |status| {
            status.state = "running".to_owned();
            status.started_at = Some(now_epoch() as i64);
            status.last_error = None;
        });
        let app = Router::new()
            .route("/", get(index))
            .route("/api/snapshot", get(api_snapshot))
            .route("/api/text", post(receive_text))
            .route(
                "/api/upload",
                post(upload).layer(DefaultBodyLimit::max(DIRECT_UPLOAD_LIMIT)),
            )
            .route(
                "/api/upload/chunk",
                post(upload_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
            )
            .route("/download/{share_id}", get(download))
            .with_state(Arc::clone(&shared));
        let shutdown_shared = Arc::clone(&shared);
        let shutdown_stop = Arc::clone(&stop);
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            loop {
                if shutdown_stop.load(Ordering::Acquire) {
                    break;
                }
                if shutdown_shared.preferences.auto_close {
                    let last = shutdown_shared.last_activity_epoch.load(Ordering::Acquire);
                    if now_epoch().saturating_sub(last) >= AUTO_CLOSE_AFTER.as_secs() {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
        if let Err(error) = server.await {
            set_server_error(&status, format!("文件传输服务异常退出：{error}"));
            return;
        }
        update_status(&status, |status| {
            if !stop.load(Ordering::Acquire) && shared.preferences.auto_close {
                status.state = "auto_closed".to_owned();
                status.last_error = Some("已在 5 分钟无活动后自动关闭".to_owned());
            } else {
                status.state = "stopped".to_owned();
            }
            status.enabled = false;
        });
    });
}

#[derive(Default, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

fn is_authorized(headers: &HeaderMap, query: &AuthQuery, shared: &SharedState) -> bool {
    query.token.as_deref() == Some(shared.pairing_token.as_str())
        || headers
            .get("x-tiez-token")
            .and_then(|value| value.to_str().ok())
            == Some(shared.pairing_token.as_str())
}

async fn index(
    State(shared): State<Arc<SharedState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&headers, &query, &shared) {
        return (StatusCode::UNAUTHORIZED, "配对链接无效或已过期").into_response();
    }
    shared.touch();
    Html(MOBILE_PAGE).into_response()
}

async fn api_snapshot(
    State(shared): State<Arc<SharedState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&headers, &query, &shared) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    shared.touch();
    let messages = shared
        .messages
        .lock()
        .map(|messages| messages.since(0))
        .unwrap_or_default();
    let devices = shared.online_devices();
    Json(serde_json::json!({ "messages": messages, "devices": devices })).into_response()
}

#[derive(Deserialize)]
struct ReceiveTextRequest {
    content: String,
    #[serde(default = "default_sender_id")]
    sender_id: String,
    #[serde(default = "default_sender_name")]
    sender_name: String,
}

async fn receive_text(
    State(shared): State<Arc<SharedState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(request): Json<ReceiveTextRequest>,
) -> Response {
    if !is_authorized(&headers, &query, &shared) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err(error) = validate_transfer_text(&request.content) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    shared.touch();
    shared.note_device(&request.sender_id, &request.sender_name);
    shared.push_message(new_message(
        "in",
        "text",
        &request.content,
        &request.sender_id,
        &request.sender_name,
        None,
    ));
    if shared.preferences.auto_copy {
        if let Err(error) = (shared.on_received)(ReceivedTransfer::Text {
            content: request.content,
            sender_name: bounded_identity(&request.sender_name, "手机"),
        }) {
            return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
        }
    }
    StatusCode::OK.into_response()
}

async fn upload(
    State(shared): State<Arc<SharedState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if !is_authorized(&headers, &query, &shared) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut sender_id = default_sender_id();
    let mut sender_name = default_sender_name();
    let mut received = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or_default() {
            "sender_id" => {
                if let Ok(value) = field.text().await {
                    sender_id = bounded_identity(&value, "mobile");
                }
            }
            "sender_name" => {
                if let Ok(value) = field.text().await {
                    sender_name = bounded_identity(&value, "手机");
                }
            }
            "file" => {
                if received.is_some() {
                    if let Some((path, _)) = received.take() {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                    return (StatusCode::BAD_REQUEST, "一次请求只能上传一个文件").into_response();
                }
                let file_name = field.file_name().unwrap_or("未命名文件").to_owned();
                match field.bytes().await {
                    Ok(bytes) if !bytes.is_empty() => {
                        let path = match allocate_receive_path(
                            Path::new(&shared.preferences.receive_directory),
                            &file_name,
                        ) {
                            Ok(path) => path,
                            Err(error) => {
                                return (StatusCode::BAD_REQUEST, error.to_string()).into_response()
                            }
                        };
                        if let Err(error) = tokio::fs::write(&path, bytes).await {
                            let _ = tokio::fs::remove_file(&path).await;
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("保存文件失败：{error}"),
                            )
                                .into_response();
                        }
                        received = Some((path, file_name));
                    }
                    _ => return (StatusCode::BAD_REQUEST, "文件为空").into_response(),
                }
            }
            _ => {}
        }
    }
    let Some((path, file_name)) = received else {
        return (StatusCode::BAD_REQUEST, "缺少文件").into_response();
    };
    complete_received_file(&shared, path, file_name, sender_id, sender_name).await
}

#[derive(Deserialize)]
struct ChunkQuery {
    token: Option<String>,
    upload_id: String,
    chunk_index: usize,
    total_chunks: usize,
    total_size: u64,
    file_name_b64: String,
    #[serde(default = "default_sender_id")]
    sender_id: String,
    #[serde(default = "default_sender_name")]
    sender_name: String,
}

async fn upload_chunk(
    State(shared): State<Arc<SharedState>>,
    Query(query): Query<ChunkQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let auth = AuthQuery {
        token: query.token.clone(),
    };
    if !is_authorized(&headers, &auth, &shared) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let file_name = match BASE64
        .decode(query.file_name_b64.as_bytes())
        .ok()
        .and_then(|value| String::from_utf8(value).ok())
    {
        Some(value) => value,
        None => return (StatusCode::BAD_REQUEST, "文件名编码无效").into_response(),
    };
    let metadata = ChunkMetadata {
        upload_id: query.upload_id.clone(),
        chunk_index: query.chunk_index,
        total_chunks: query.total_chunks,
        file_name: file_name.clone(),
        total_size: query.total_size,
    };
    let mut session = if query.chunk_index == 0 {
        let final_path = match allocate_receive_path(
            Path::new(&shared.preferences.receive_directory),
            &file_name,
        ) {
            Ok(path) => path,
            Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        };
        UploadSession {
            temp_path: final_path.with_file_name(format!(".{}.tiez-part", Uuid::new_v4().simple())),
            final_path,
            next_index: 0,
            total_chunks: query.total_chunks,
            total_size: query.total_size,
            written: 0,
            file_name: file_name.clone(),
            sender_id: bounded_identity(&query.sender_id, "mobile"),
            sender_name: bounded_identity(&query.sender_name, "手机"),
        }
    } else {
        match shared
            .uploads
            .lock()
            .ok()
            .and_then(|mut uploads| uploads.remove(&query.upload_id))
        {
            Some(session) => session,
            None => return (StatusCode::CONFLICT, "上传会话不存在或分片并发").into_response(),
        }
    };
    if session.total_chunks != query.total_chunks
        || session.total_size != query.total_size
        || session.file_name != file_name
    {
        let _ = tokio::fs::remove_file(&session.temp_path).await;
        return (StatusCode::BAD_REQUEST, "分片元数据不一致").into_response();
    }
    if let Err(error) = validate_chunk(&metadata, session.next_index, body.len()) {
        if query.chunk_index > 0 {
            if let Ok(mut uploads) = shared.uploads.lock() {
                uploads.insert(query.upload_id, session);
            }
        }
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).append(true).write(true);
    let write_result = async {
        let mut file = options.open(&session.temp_path).await?;
        file.write_all(&body).await?;
        file.flush().await
    }
    .await;
    if let Err(error) = write_result {
        let _ = tokio::fs::remove_file(&session.temp_path).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入分片失败：{error}"),
        )
            .into_response();
    }
    session.written = session.written.saturating_add(body.len() as u64);
    session.next_index += 1;
    if session.written > session.total_size {
        let _ = tokio::fs::remove_file(&session.temp_path).await;
        return (StatusCode::BAD_REQUEST, "分片总大小超过声明值").into_response();
    }
    if session.next_index == session.total_chunks {
        if session.written != session.total_size {
            let _ = tokio::fs::remove_file(&session.temp_path).await;
            return (StatusCode::BAD_REQUEST, "文件实际大小与声明值不一致").into_response();
        }
        if let Err(error) = tokio::fs::rename(&session.temp_path, &session.final_path).await {
            let _ = tokio::fs::remove_file(&session.temp_path).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("完成文件失败：{error}"),
            )
                .into_response();
        }
        return complete_received_file(
            &shared,
            session.final_path,
            session.file_name,
            session.sender_id,
            session.sender_name,
        )
        .await;
    }
    if let Ok(mut uploads) = shared.uploads.lock() {
        uploads.insert(query.upload_id, session);
    }
    shared.touch();
    StatusCode::OK.into_response()
}

async fn complete_received_file(
    shared: &Arc<SharedState>,
    path: PathBuf,
    file_name: String,
    sender_id: String,
    sender_name: String,
) -> Response {
    shared.touch();
    shared.note_device(&sender_id, &sender_name);
    shared.push_message(new_message(
        "in",
        classify_file_name(&file_name).message_type(),
        &file_name,
        &sender_id,
        &sender_name,
        Some(path.to_string_lossy().into_owned()),
    ));
    if shared.preferences.auto_copy {
        if let Err(error) = (shared.on_received)(ReceivedTransfer::File {
            path: path.clone(),
            sender_name,
        }) {
            return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
        }
    }
    if shared.preferences.auto_open {
        open_received_file_location(&path);
    }
    StatusCode::OK.into_response()
}

async fn download(
    State(shared): State<Arc<SharedState>>,
    AxumPath(share_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&headers, &query, &shared) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let path = shared
        .shares
        .lock()
        .ok()
        .and_then(|shares| shares.get(&share_id).cloned());
    let Some(path) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let total_size = match file.metadata().await {
        Ok(metadata) => metadata.len(),
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    shared.touch();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("download.bin");
    let requested_range = match headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(|value| parse_byte_range(value, total_size))
        .transpose()
    {
        Ok(range) => range.flatten(),
        Err(()) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{total_size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return response;
        }
    };
    let (mut response, response_length) = if let Some((start, end)) = requested_range {
        let mut file = file;
        if file.seek(SeekFrom::Start(start)).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let length = end - start + 1;
        let mut response = Body::from_stream(ReaderStream::new(file.take(length))).into_response();
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        if let Ok(value) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total_size}")) {
            response.headers_mut().insert(header::CONTENT_RANGE, value);
        }
        (response, length)
    } else {
        (
            Body::from_stream(ReaderStream::new(file)).into_response(),
            total_size,
        )
    };
    let content_type = mime_guess::from_path(&path).first_or_octet_stream();
    if let Ok(value) = HeaderValue::from_str(content_type.as_ref()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    let ascii_name: String = file_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || ".-_".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{ascii_name}\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&response_length.to_string()) {
        response.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    response
}

fn parse_byte_range(value: &str, total_size: u64) -> Result<Option<(u64, u64)>, ()> {
    let value = value.strip_prefix("bytes=").ok_or(())?;
    if total_size == 0 || value.contains(',') {
        return Err(());
    }
    let (start, end) = value.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let length = suffix.min(total_size);
        return Ok(Some((total_size - length, total_size - 1)));
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= total_size {
        return Err(());
    }
    let end = if end.is_empty() {
        total_size - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(total_size - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn new_message(
    direction: &str,
    msg_type: &str,
    content: &str,
    sender_id: &str,
    sender_name: &str,
    file_path: Option<String>,
) -> TransferMessage {
    TransferMessage {
        id: 0,
        direction: direction.to_owned(),
        msg_type: msg_type.to_owned(),
        content: content.to_owned(),
        timestamp: now_epoch() as i64,
        sender_id: bounded_identity(sender_id, "mobile"),
        sender_name: bounded_identity(sender_name, "手机"),
        file_path,
    }
}

fn bounded_identity(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value
            .chars()
            .filter(|character| !character.is_control())
            .take(80)
            .collect()
    }
}

fn default_sender_id() -> String {
    "mobile".to_owned()
}

fn default_sender_name() -> String {
    "手机".to_owned()
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn preferred_local_ip() -> String {
    local_ip_address::local_ip()
        .ok()
        .filter(|address| matches!(address, IpAddr::V4(value) if !value.is_loopback()))
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .to_string()
}

fn qr_png_base64(value: &str) -> Result<String, String> {
    let code =
        QrCode::new(value.as_bytes()).map_err(|error| format!("无法生成配对二维码：{error}"))?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(256, 256)
        .build();
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|error| format!("无法编码配对二维码：{error}"))?;
    Ok(BASE64.encode(bytes.into_inner()))
}

fn update_status(
    status: &Arc<Mutex<NativeFileTransferStatus>>,
    update: impl FnOnce(&mut NativeFileTransferStatus),
) {
    if let Ok(mut status) = status.lock() {
        update(&mut status);
    }
}

fn set_server_error(status: &Arc<Mutex<NativeFileTransferStatus>>, message: String) {
    update_status(status, |status| {
        status.state = "error".to_owned();
        status.enabled = false;
        status.last_error = Some(message);
    });
}

fn cleanup_uploads(shared: &SharedState) {
    let paths: Vec<PathBuf> = shared
        .uploads
        .lock()
        .map(|mut uploads| {
            uploads
                .drain()
                .map(|(_, session)| session.temp_path)
                .collect()
        })
        .unwrap_or_default();
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
fn open_received_file_location(path: &Path) {
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

#[cfg(not(windows))]
fn open_received_file_location(_path: &Path) {}

const MOBILE_PAGE: &str = r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="referrer" content="no-referrer"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TieZ 局域网传输</title><style>
:root{color-scheme:light dark;font-family:"Segoe UI","Microsoft YaHei",sans-serif}body{max-width:720px;margin:auto;padding:20px;background:#f5f5f5;color:#202020}.card{background:#fff;border-radius:14px;padding:16px;margin:12px 0;box-shadow:0 2px 12px #0001}textarea,input,button{box-sizing:border-box;font:inherit}textarea{width:100%;min-height:100px;padding:10px;border-radius:9px;border:1px solid #aaa}button{border:0;border-radius:8px;padding:10px 16px;background:#0067c0;color:white;margin:6px 4px 0 0}.message{padding:10px 0;border-bottom:1px solid #ddd;overflow-wrap:anywhere}.meta{font-size:12px;color:#666}.error{color:#c42b1c}@media(prefers-color-scheme:dark){body{background:#181818;color:#eee}.card{background:#242424}.meta{color:#aaa}}
</style></head><body><h1>TieZ 局域网传输</h1><p>此页面只在本次配对有效。请勿把链接转发给不信任的人。</p>
<section class="card"><h2>发送文字到电脑</h2><textarea id="text" placeholder="输入要发送的文字"></textarea><button onclick="sendText()">发送文字</button></section>
<section class="card"><h2>发送文件到电脑</h2><input id="files" type="file" multiple><button onclick="sendFiles()">发送文件</button><div id="progress"></div></section>
<section class="card"><h2>消息</h2><div id="messages">正在连接……</div></section>
<script>
const token=new URLSearchParams(location.search).get('token')||'';const api=p=>`${p}?token=${encodeURIComponent(token)}`;
let deviceId=localStorage.tiezDeviceId||(localStorage.tiezDeviceId=crypto.randomUUID());let deviceName=navigator.platform||'手机';
async function checked(response){if(!response.ok)throw new Error(await response.text()||`HTTP ${response.status}`);return response}
async function sendText(){try{let el=document.querySelector('#text');await checked(await fetch(api('/api/text'),{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({content:el.value,sender_id:deviceId,sender_name:deviceName})}));el.value='';await refresh()}catch(e){alert(e.message)}}
function name64(name){return btoa(unescape(encodeURIComponent(name)))}
async function sendFiles(){let files=[...document.querySelector('#files').files];for(let file of files){let progress=document.querySelector('#progress');progress.textContent=`正在发送 ${file.name}`;if(file.size<=64*1024*1024){let form=new FormData();form.append('sender_id',deviceId);form.append('sender_name',deviceName);form.append('file',file,file.name);await checked(await fetch(api('/api/upload'),{method:'POST',body:form}))}else{let size=4*1024*1024,total=Math.ceil(file.size/size),id=crypto.randomUUID().replaceAll('-','');for(let i=0;i<total;i++){let params=new URLSearchParams({token,upload_id:id,chunk_index:i,total_chunks:total,total_size:file.size,file_name_b64:name64(file.name),sender_id:deviceId,sender_name:deviceName});await checked(await fetch('/api/upload/chunk?'+params,{method:'POST',body:file.slice(i*size,Math.min(file.size,(i+1)*size))}));progress.textContent=`${file.name}：${i+1}/${total}`}}}document.querySelector('#progress').textContent='发送完成';await refresh()}
async function refresh(){try{let data=await (await checked(await fetch(api('/api/snapshot')))).json();let root=document.querySelector('#messages');root.textContent='';for(let m of data.messages.slice().reverse()){let row=document.createElement('div');row.className='message';let text=document.createElement('div');text.textContent=(m.direction==='in'?'手机 → 电脑：':'电脑 → 手机：')+m.content;row.append(text);if(m.direction==='out'&&m.file_path){let link=document.createElement('a');link.href=m.file_path;link.textContent='下载文件';link.style.display='block';row.append(link)}let meta=document.createElement('div');meta.className='meta';meta.textContent=m.sender_name;row.append(meta);root.append(row)}if(!data.messages.length)root.textContent='暂无消息'}catch(e){let root=document.querySelector('#messages');root.textContent=e.message;root.className='error'}}
refresh();setInterval(refresh,2000);
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn qr_code_is_png_and_identity_is_bounded() {
        let encoded = qr_png_base64("http://127.0.0.1:12345?token=test").expect("qr");
        let bytes = BASE64.decode(encoded).expect("base64");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(bounded_identity("", "手机"), "手机");
        assert_eq!(bounded_identity(&"a".repeat(100), "手机").len(), 80);
        assert_eq!(parse_byte_range("bytes=10-19", 100), Ok(Some((10, 19))));
        assert_eq!(parse_byte_range("bytes=90-", 100), Ok(Some((90, 99))));
        assert_eq!(parse_byte_range("bytes=-10", 100), Ok(Some((90, 99))));
        assert!(parse_byte_range("bytes=100-101", 100).is_err());
        assert!(parse_byte_range("bytes=0-1,4-5", 100).is_err());
    }

    #[test]
    fn running_server_rejects_unpaired_requests_and_stops_cleanly() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve port");
        let port = listener.local_addr().expect("address").port();
        drop(listener);
        let test_root =
            std::env::temp_dir().join(format!("tiez-transfer-runtime-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&test_root).expect("test root");
        let mut preferences = FileTransferPreferences::in_memory(&test_root);
        preferences
            .update(FileTransferPreferencesUpdate {
                port: Some(port),
                ..FileTransferPreferencesUpdate::default()
            })
            .expect("port");
        let service = NativeFileTransferService::new(preferences, |_| Ok(()));
        service.start().expect("start");
        let snapshot = service.snapshot().expect("snapshot");
        assert_eq!(snapshot.status.state, "running");

        let unauthorized = http_get(port, "/");
        assert!(unauthorized.starts_with("HTTP/1.1 401"), "{unauthorized}");
        let pairing_url = snapshot.status.pairing_url.expect("pairing url");
        let target = pairing_url
            .find("/?")
            .map(|index| &pairing_url[index..])
            .expect("target");
        let authorized = http_get(port, target);
        assert!(authorized.starts_with("HTTP/1.1 200"), "{authorized}");

        let shared_file = test_root.join("共享.txt");
        std::fs::write(&shared_file, b"0123456789").expect("shared file");
        let shared_snapshot = service.share_files(vec![shared_file]).expect("share file");
        let download_url = shared_snapshot
            .messages
            .last()
            .and_then(|message| message.file_path.as_deref())
            .expect("download url");
        let download_target = download_url
            .find("/download/")
            .map(|index| &download_url[index..])
            .expect("download target");
        let partial = http_get_with_headers(port, download_target, "Range: bytes=2-4\r\n");
        assert!(partial.starts_with("HTTP/1.1 206"), "{partial}");
        assert!(partial.ends_with("234"), "{partial}");

        service.stop();
        assert_eq!(service.snapshot().expect("stopped").status.state, "stopped");
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
        std::fs::remove_dir_all(test_root).expect("cleanup");
    }

    fn http_get(port: u16, target: &str) -> String {
        http_get_with_headers(port, target, "")
    }

    fn http_get_with_headers(port: u16, target: &str, headers: &str) -> String {
        let mut stream =
            std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
        stream
            .write_all(
                format!(
                    "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Connection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        response
    }
}
