//! Runtime ports and deterministic scheduling decisions for native cloud sync.
//!
//! Network behavior lives in `cloud_sync_webdav`; wire models and conflict
//! identity live in `cloud_sync_protocol`. This module defines the remaining
//! boundary between the shared runner and a desktop host's database, files,
//! settings, cancellation signal, and UI notifications.

use crate::cloud_sync_protocol::{
    item_revision, CloudSyncContentPrefs, CloudSyncItem, WebDavDeviceHead, WebDavDeviceSnapshot,
    WebDavOpsBatch, WebDavSettingsSnapshot, WebDavSyncHead,
};
use crate::cloud_sync_webdav::{
    WebDavLayout, WebDavOpReference, WebDavTransport, WebDavTransportError,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;

pub const DEFAULT_MAX_OPS_PER_RUN: usize = 2_000;
pub const DEFAULT_MAX_REMOTE_SNAPSHOTS: usize = 24;
pub const WEBDAV_OP_BATCH_SIZE: usize = 400;
pub const WEBDAV_HEAD_REBUILD_INTERVAL_SECS: i64 = 5 * 60;

const BLOB_KIND_IMAGE: &str = "image";
const BLOB_KIND_CONTENT: &str = "content";
const BLOB_KIND_HTML: &str = "html";
const BLOB_THRESHOLD_CONTENT: usize = 12 * 1024;
const BLOB_THRESHOLD_HTML: usize = 24 * 1024;
const MAX_REMOTE_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_BLOB_CACHE_ENTRIES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudSyncHostErrorKind {
    Storage,
    Payload,
    Settings,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSyncHostError {
    kind: CloudSyncHostErrorKind,
    message: String,
}

impl CloudSyncHostError {
    pub fn new(kind: CloudSyncHostErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CloudSyncHostErrorKind {
        self.kind
    }
}

impl fmt::Display for CloudSyncHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CloudSyncHostError {}

pub type CloudSyncHostResult<T> = Result<T, CloudSyncHostError>;

#[derive(Debug)]
pub enum CloudSyncRunnerError {
    Host(CloudSyncHostError),
    Transport(WebDavTransportError),
    Protocol(String),
}

impl fmt::Display for CloudSyncRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => write!(formatter, "cloud sync host failed: {error}"),
            Self::Transport(error) => write!(formatter, "cloud sync transport failed: {error}"),
            Self::Protocol(message) => write!(formatter, "cloud sync protocol failed: {message}"),
        }
    }
}

impl Error for CloudSyncRunnerError {}

impl From<CloudSyncHostError> for CloudSyncRunnerError {
    fn from(error: CloudSyncHostError) -> Self {
        Self::Host(error)
    }
}

impl From<WebDavTransportError> for CloudSyncRunnerError {
    fn from(error: WebDavTransportError) -> Self {
        Self::Transport(error)
    }
}

pub type CloudSyncRunnerResult<T> = Result<T, CloudSyncRunnerError>;

#[derive(Clone)]
pub struct CloudSyncRunnerConfig {
    pub device_id: String,
    pub webdav_url: String,
    pub webdav_username: String,
    pub webdav_password: String,
    pub webdav_base_path: String,
    pub interval_secs: u64,
    pub snapshot_interval_secs: i64,
    pub content_prefs: CloudSyncContentPrefs,
    pub max_ops_per_run: usize,
    pub max_remote_snapshots: usize,
}

impl CloudSyncRunnerConfig {
    pub fn new(
        device_id: impl Into<String>,
        webdav_url: impl Into<String>,
        webdav_username: impl Into<String>,
        webdav_password: impl Into<String>,
        webdav_base_path: impl Into<String>,
        interval_secs: u64,
        snapshot_interval_secs: i64,
        content_prefs: CloudSyncContentPrefs,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            webdav_url: webdav_url.into(),
            webdav_username: webdav_username.into(),
            webdav_password: webdav_password.into(),
            webdav_base_path: webdav_base_path.into(),
            interval_secs,
            snapshot_interval_secs,
            content_prefs,
            max_ops_per_run: DEFAULT_MAX_OPS_PER_RUN,
            max_remote_snapshots: DEFAULT_MAX_REMOTE_SNAPSHOTS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSyncRuntimeState {
    #[serde(default)]
    pub local_op_seq: i64,
    #[serde(default)]
    pub op_cursors: BTreeMap<String, i64>,
    #[serde(default)]
    pub blob_cache: HashMap<String, i64>,
    #[serde(default)]
    pub last_snapshot_push_at: i64,
    #[serde(default)]
    pub last_snapshot_pull_at: i64,
    #[serde(default)]
    pub last_head_rebuild_at: i64,
    #[serde(default)]
    pub settings_applied_at: i64,
    #[serde(default)]
    pub cursor: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudSyncLocalDelta {
    pub items: Vec<CloudSyncItem>,
    pub collapsed_index: BTreeMap<String, CloudSyncItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSyncRunStatus {
    pub state: String,
    pub running: bool,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
    pub uploaded_items: usize,
    pub received_items: usize,
}

impl CloudSyncRunStatus {
    pub fn syncing() -> Self {
        Self {
            state: "syncing".to_owned(),
            running: true,
            last_sync_at: None,
            last_error: None,
            uploaded_items: 0,
            received_items: 0,
        }
    }

    pub fn idle(now_ms: i64, uploaded_items: usize, received_items: usize) -> Self {
        Self {
            state: "idle".to_owned(),
            running: true,
            last_sync_at: Some(now_ms),
            last_error: None,
            uploaded_items,
            received_items,
        }
    }

    pub fn disabled() -> Self {
        Self {
            state: "disabled".to_owned(),
            running: false,
            last_sync_at: None,
            last_error: None,
            uploaded_items: 0,
            received_items: 0,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            state: "error".to_owned(),
            running: true,
            last_sync_at: None,
            last_error: Some(message.into()),
            uploaded_items: 0,
            received_items: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudSyncHostEvent {
    Status(CloudSyncRunStatus),
    HistoryChanged,
    SettingsChanged,
}

/// Host operations that must remain outside the shared sync runner.
///
/// Implementations may keep a persistent database connection or open one per
/// call. A runner never receives a Tauri/WinUI window handle through this port.
pub trait CloudSyncHost: Send {
    fn now_ms(&self) -> i64;
    fn is_cancelled(&self) -> bool;

    fn load_runtime_state(&mut self) -> CloudSyncHostResult<CloudSyncRuntimeState>;
    fn save_runtime_state(&mut self, state: &CloudSyncRuntimeState) -> CloudSyncHostResult<()>;

    fn collect_local_items(
        &mut self,
        preferences: &CloudSyncContentPrefs,
    ) -> CloudSyncHostResult<Vec<CloudSyncItem>>;

    fn collect_local_delta(
        &mut self,
        local_items: &[CloudSyncItem],
    ) -> CloudSyncHostResult<CloudSyncLocalDelta>;

    fn replace_local_index(
        &mut self,
        collapsed_index: &BTreeMap<String, CloudSyncItem>,
    ) -> CloudSyncHostResult<()>;

    fn apply_remote_items(
        &mut self,
        remote_items: &[CloudSyncItem],
        preferences: &CloudSyncContentPrefs,
    ) -> CloudSyncHostResult<usize>;

    /// Resolve local image paths and rich-text resources before blob offload.
    fn prepare_upload_items(&mut self, items: &mut [CloudSyncItem]) -> CloudSyncHostResult<()>;

    /// Store a validated remote image payload and return its local content value.
    fn materialize_remote_image(&mut self, data_url: &str) -> CloudSyncHostResult<String>;

    fn collect_syncable_settings(&mut self) -> CloudSyncHostResult<HashMap<String, String>>;
    fn apply_synced_settings(
        &mut self,
        incoming: &HashMap<String, String>,
    ) -> CloudSyncHostResult<usize>;

    fn next_emoji_operation(&mut self) -> CloudSyncHostResult<Option<CloudSyncItem>>;
    fn mark_emoji_uploaded(&mut self, content_hash: i64);
    fn emit(&mut self, event: CloudSyncHostEvent);
}

/// Run one WebDAV synchronization pass without depending on a desktop UI runtime.
///
/// Operation files and snapshots become locally committed only after the updated
/// head document is published. This preserves the retry/discoverability contract
/// used by existing Tauri peers.
pub async fn run_webdav_once<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    force_snapshot: bool,
) -> CloudSyncRunnerResult<CloudSyncRunStatus> {
    if host.is_cancelled() {
        let status = CloudSyncRunStatus::disabled();
        host.emit(CloudSyncHostEvent::Status(status.clone()));
        return Ok(status);
    }

    host.emit(CloudSyncHostEvent::Status(CloudSyncRunStatus::syncing()));
    match run_webdav_once_inner(host, config, force_snapshot).await {
        Ok(status) => {
            host.emit(CloudSyncHostEvent::Status(status.clone()));
            Ok(status)
        }
        Err(error) => {
            host.emit(CloudSyncHostEvent::Status(CloudSyncRunStatus::failed(
                error.to_string(),
            )));
            Err(error)
        }
    }
}

async fn run_webdav_once_inner<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    force_snapshot: bool,
) -> CloudSyncRunnerResult<CloudSyncRunStatus> {
    if normalize_device_id(&config.device_id).is_none() {
        return Err(CloudSyncRunnerError::Protocol(
            "invalid local device id".to_owned(),
        ));
    }

    let now = host.now_ms();
    let mut state = host.load_runtime_state()?;
    let local_items = host.collect_local_items(&config.content_prefs)?;
    let local_delta = host.collect_local_delta(&local_items)?;
    let transport = WebDavTransport::new(
        &config.webdav_url,
        &config.webdav_username,
        &config.webdav_password,
        &config.device_id,
    )?;
    let layout = transport.ensure_layout(&config.webdav_base_path).await?;
    let mut head = resolve_sync_head(host, config, &transport, &layout, &mut state, now).await?;
    let should_pull = should_pull_snapshot(
        force_snapshot,
        state.last_snapshot_pull_at,
        now,
        !state.op_cursors.is_empty(),
        config.snapshot_interval_secs,
    );
    let should_push = force_snapshot
        || should_run_periodic_snapshot(
            state.last_snapshot_push_at,
            now,
            config.snapshot_interval_secs,
        );

    let mut uploaded_items = 0usize;
    if !local_delta.items.is_empty() {
        let mut items = local_delta.items.clone();
        host.prepare_upload_items(&mut items)?;
        process_outgoing_blobs(host, config, &transport, &layout, &mut state, &mut items).await?;
        if publish_ops(
            host, config, &transport, &layout, &mut state, &mut head, &items,
        )
        .await?
        .is_none()
        {
            return finish_cancelled(host, &mut state);
        }
        host.replace_local_index(&local_delta.collapsed_index)?;
        uploaded_items = uploaded_items.saturating_add(local_delta.items.len());
    }

    if host.is_cancelled() {
        return finish_cancelled(host, &mut state);
    }

    let (mut received_items, head_stale) =
        pull_remote_ops(host, config, &transport, &layout, &mut state, &head).await?;
    if host.is_cancelled() {
        return finish_cancelled(host, &mut state);
    }

    let mut pulled_recovery_snapshots = false;
    if head_stale {
        head = rebuild_sync_head(host, config, &transport, &layout).await?;
        head.updated_at = host.now_ms();
        transport
            .put_json_atomic(&layout.head_path, &head, "sync head")
            .await?;
        state.last_head_rebuild_at = now;
        host.save_runtime_state(&state)?;

        let (recovered, _) =
            pull_remote_ops(host, config, &transport, &layout, &mut state, &head).await?;
        received_items = received_items.saturating_add(recovered);
        received_items = received_items
            .saturating_add(pull_remote_snapshots(host, config, &transport, &layout, &head).await?);
        pulled_recovery_snapshots = true;
    }

    if host.is_cancelled() {
        return finish_cancelled(host, &mut state);
    }

    if let Ok(Some(mut emoji)) = host.next_emoji_operation() {
        let emoji_hash = emoji.content_hash;
        host.prepare_upload_items(std::slice::from_mut(&mut emoji))?;
        process_outgoing_blobs(
            host,
            config,
            &transport,
            &layout,
            &mut state,
            std::slice::from_mut(&mut emoji),
        )
        .await?;
        if publish_ops(
            host,
            config,
            &transport,
            &layout,
            &mut state,
            &mut head,
            &[emoji],
        )
        .await?
        .is_none()
        {
            return finish_cancelled(host, &mut state);
        }
        host.mark_emoji_uploaded(emoji_hash);
        uploaded_items = uploaded_items.saturating_add(1);
    }

    let mut settings_changed = 0usize;
    if should_pull {
        if !pulled_recovery_snapshots {
            received_items = received_items.saturating_add(
                pull_remote_snapshots(host, config, &transport, &layout, &head).await?,
            );
        }
        settings_changed =
            pull_remote_settings(host, config, &transport, &layout, &mut state, &head).await?;
        state.last_snapshot_pull_at = now;
        host.save_runtime_state(&state)?;
    }

    if should_push {
        if host.is_cancelled() {
            return finish_cancelled(host, &mut state);
        }

        let mut snapshot_items = host.collect_local_items(&config.content_prefs)?;
        let snapshot_item_count = snapshot_items.len();
        host.prepare_upload_items(&mut snapshot_items)?;
        process_outgoing_blobs(
            host,
            config,
            &transport,
            &layout,
            &mut state,
            &mut snapshot_items,
        )
        .await?;
        let snapshot_updated_at = host.now_ms();
        let snapshot = WebDavDeviceSnapshot {
            device_id: config.device_id.clone(),
            updated_at: snapshot_updated_at,
            latest_op_seq: state.local_op_seq,
            entries: snapshot_items,
        };
        transport
            .put_json_atomic(
                &device_document_path(&layout.devices_path, &config.device_id),
                &snapshot,
                "snapshot",
            )
            .await?;
        update_head_device(&mut head, &config.device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(state.local_op_seq);
            device.snapshot_updated_at = device.snapshot_updated_at.max(snapshot_updated_at);
            device.snapshot_op_seq = device.snapshot_op_seq.max(state.local_op_seq);
        });

        let settings = host.collect_syncable_settings()?;
        let settings_updated_at = host.now_ms();
        let settings_snapshot = WebDavSettingsSnapshot {
            device_id: config.device_id.clone(),
            updated_at: settings_updated_at,
            settings,
        };
        let settings_count = settings_snapshot.settings.len();
        transport
            .put_json_atomic(
                &device_document_path(&layout.settings_path, &config.device_id),
                &settings_snapshot,
                "settings snapshot",
            )
            .await?;
        update_head_device(&mut head, &config.device_id, |device| {
            device.settings_updated_at = device.settings_updated_at.max(settings_updated_at);
        });

        // The head is the publication commit point for both snapshot documents.
        head.updated_at = host.now_ms();
        transport
            .put_json_atomic(&layout.head_path, &head, "sync head")
            .await?;
        state.last_snapshot_push_at = now;
        prune_blob_cache(&mut state.blob_cache);
        host.save_runtime_state(&state)?;
        uploaded_items = uploaded_items
            .saturating_add(snapshot_item_count)
            .saturating_add(settings_count);
        let _ = cleanup_local_ops(config, &transport, &layout, state.local_op_seq).await;
    }

    prune_blob_cache(&mut state.blob_cache);
    state.cursor = now;
    host.save_runtime_state(&state)?;
    if received_items > 0 {
        host.emit(CloudSyncHostEvent::HistoryChanged);
    }
    if settings_changed > 0 {
        host.emit(CloudSyncHostEvent::SettingsChanged);
    }
    Ok(CloudSyncRunStatus::idle(
        now,
        uploaded_items,
        received_items,
    ))
}

fn finish_cancelled<H: CloudSyncHost>(
    host: &mut H,
    state: &mut CloudSyncRuntimeState,
) -> CloudSyncRunnerResult<CloudSyncRunStatus> {
    prune_blob_cache(&mut state.blob_cache);
    host.save_runtime_state(state)?;
    Ok(CloudSyncRunStatus::disabled())
}

async fn resolve_sync_head<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    state: &mut CloudSyncRuntimeState,
    now: i64,
) -> CloudSyncRunnerResult<WebDavSyncHead> {
    let fetched = transport
        .get_json::<WebDavSyncHead>(
            &layout.head_path,
            404,
            "webdav GET head failed",
            "parse head json failed",
        )
        .await?;
    let needs_rebuild = fetched.is_none()
        || should_run_periodic_snapshot(
            state.last_head_rebuild_at,
            now,
            WEBDAV_HEAD_REBUILD_INTERVAL_SECS,
        );
    if !needs_rebuild {
        return Ok(fetched.unwrap_or_default());
    }

    match rebuild_sync_head(host, config, transport, layout).await {
        Ok(mut rebuilt) => {
            rebuilt.updated_at = host.now_ms();
            if fetched.as_ref() != Some(&rebuilt) {
                transport
                    .put_json_atomic(&layout.head_path, &rebuilt, "sync head")
                    .await?;
            }
            state.last_head_rebuild_at = now;
            host.save_runtime_state(state)?;
            Ok(rebuilt)
        }
        Err(error) => fetched.ok_or(error),
    }
}

async fn rebuild_sync_head<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
) -> CloudSyncRunnerResult<WebDavSyncHead> {
    let mut head = WebDavSyncHead {
        updated_at: host.now_ms(),
        devices: BTreeMap::new(),
    };

    for reference in transport.list_op_references(&layout.ops_path).await? {
        if !is_safe_device_id(&reference.device_id) {
            continue;
        }
        update_head_device(&mut head, &reference.device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(reference.seq);
        });
    }

    let snapshot_limit = config
        .max_remote_snapshots
        .min(DEFAULT_MAX_REMOTE_SNAPSHOTS);
    for device_id in transport
        .list_snapshot_ids(&layout.devices_path)
        .await?
        .into_iter()
        .take(snapshot_limit)
    {
        if !is_safe_device_id(&device_id) {
            continue;
        }
        let Some(snapshot) = fetch_snapshot(transport, layout, &device_id).await? else {
            continue;
        };
        if !same_device_id(&snapshot.device_id, &device_id) {
            continue;
        }
        update_head_device(&mut head, &device_id, |device| {
            device.latest_op_seq = device.latest_op_seq.max(snapshot.latest_op_seq);
            device.snapshot_updated_at = device.snapshot_updated_at.max(snapshot.updated_at);
            device.snapshot_op_seq = device.snapshot_op_seq.max(snapshot.latest_op_seq);
        });
    }

    for device_id in transport
        .list_snapshot_ids(&layout.settings_path)
        .await?
        .into_iter()
        .take(snapshot_limit)
    {
        if !is_safe_device_id(&device_id) {
            continue;
        }
        let Some(snapshot) = fetch_settings_snapshot(transport, layout, &device_id).await? else {
            continue;
        };
        if !same_device_id(&snapshot.device_id, &device_id) {
            continue;
        }
        update_head_device(&mut head, &device_id, |device| {
            device.settings_updated_at = device.settings_updated_at.max(snapshot.updated_at);
        });
    }
    Ok(head)
}

async fn publish_ops<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    state: &mut CloudSyncRuntimeState,
    head: &mut WebDavSyncHead,
    items: &[CloudSyncItem],
) -> CloudSyncRunnerResult<Option<i64>> {
    if items.is_empty() {
        return Ok(Some(state.local_op_seq));
    }
    let published_seq = head
        .devices
        .get(&config.device_id)
        .map(|device| device.latest_op_seq)
        .unwrap_or(0);
    let mut next_seq = state.local_op_seq.max(published_seq);
    for chunk in items.chunks(WEBDAV_OP_BATCH_SIZE) {
        if host.is_cancelled() {
            return Ok(None);
        }
        next_seq = next_seq.checked_add(1).ok_or_else(|| {
            CloudSyncRunnerError::Protocol("local WebDAV op sequence exhausted".to_owned())
        })?;
        let batch = WebDavOpsBatch {
            device_id: config.device_id.clone(),
            seq: next_seq,
            updated_at: host.now_ms(),
            entries: chunk.to_vec(),
        };
        transport
            .put_json_atomic(
                &op_document_path(&layout.ops_path, &config.device_id, next_seq),
                &batch,
                "ops batch",
            )
            .await?;
    }

    update_head_device(head, &config.device_id, |device| {
        device.latest_op_seq = device.latest_op_seq.max(next_seq);
    });
    head.updated_at = host.now_ms();
    transport
        .put_json_atomic(&layout.head_path, head, "sync head")
        .await?;
    state.local_op_seq = next_seq;
    host.save_runtime_state(state)?;
    Ok(Some(next_seq))
}

async fn pull_remote_ops<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    state: &mut CloudSyncRuntimeState,
    head: &WebDavSyncHead,
) -> CloudSyncRunnerResult<(usize, bool)> {
    let plan = plan_remote_ops_from_head(
        head,
        &state.op_cursors,
        &config.device_id,
        config.max_ops_per_run.min(DEFAULT_MAX_OPS_PER_RUN),
    );
    let mut received = 0usize;
    let mut head_stale = false;
    let mut stale_devices = HashSet::new();
    for reference in plan.references {
        if host.is_cancelled() {
            break;
        }
        if stale_devices.contains(&reference.device_id) {
            continue;
        }
        let Some(mut batch) = fetch_ops_batch(transport, layout, &reference).await? else {
            stale_devices.insert(reference.device_id);
            head_stale = true;
            continue;
        };
        if !same_device_id(&batch.device_id, &reference.device_id) || batch.seq != reference.seq {
            stale_devices.insert(reference.device_id);
            head_stale = true;
            continue;
        }
        hydrate_incoming_blobs(host, transport, layout, &mut batch.entries).await?;
        received = received
            .saturating_add(host.apply_remote_items(&batch.entries, &config.content_prefs)?);
        state.op_cursors.insert(reference.device_id, reference.seq);
        host.save_runtime_state(state)?;
    }
    Ok((received, head_stale))
}

async fn pull_remote_snapshots<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    head: &WebDavSyncHead,
) -> CloudSyncRunnerResult<usize> {
    let mut items = Vec::new();
    for device_id in remote_snapshot_candidates(
        head,
        &config.device_id,
        config
            .max_remote_snapshots
            .min(DEFAULT_MAX_REMOTE_SNAPSHOTS),
    ) {
        if host.is_cancelled() {
            break;
        }
        let Some(mut snapshot) = fetch_snapshot(transport, layout, &device_id).await? else {
            continue;
        };
        if !same_device_id(&snapshot.device_id, &device_id) {
            continue;
        }
        hydrate_incoming_blobs(host, transport, layout, &mut snapshot.entries).await?;
        items.extend(snapshot.entries);
    }
    items.sort_by_key(item_revision);
    Ok(host.apply_remote_items(&items, &config.content_prefs)?)
}

async fn pull_remote_settings<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    state: &mut CloudSyncRuntimeState,
    head: &WebDavSyncHead,
) -> CloudSyncRunnerResult<usize> {
    let Some((device_id, latest_at)) = newest_settings_source(head, &config.device_id) else {
        return Ok(0);
    };
    if latest_at <= state.settings_applied_at {
        return Ok(0);
    }
    let Some(snapshot) = fetch_settings_snapshot(transport, layout, &device_id).await? else {
        return Ok(0);
    };
    if !same_device_id(&snapshot.device_id, &device_id)
        || snapshot.updated_at <= state.settings_applied_at
    {
        return Ok(0);
    }
    let changed = host.apply_synced_settings(&snapshot.settings)?;
    state.settings_applied_at = snapshot.updated_at;
    host.save_runtime_state(state)?;
    Ok(changed)
}

async fn process_outgoing_blobs<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    state: &mut CloudSyncRuntimeState,
    items: &mut [CloudSyncItem],
) -> CloudSyncRunnerResult<()> {
    for item in items {
        if item.deleted_at > 0 {
            continue;
        }
        if item.content_type == "image" {
            if !item.content.starts_with("data:image/") {
                return Err(CloudSyncRunnerError::Protocol(
                    "image upload was not materialized as a data URL".to_owned(),
                ));
            }
            if !item.content.is_empty() {
                let bytes = item.content.as_bytes();
                let hash = upload_blob_cached(
                    host,
                    config,
                    transport,
                    layout,
                    state,
                    BLOB_KIND_IMAGE,
                    bytes,
                )
                .await?;
                item.content_blob_hash = Some(hash);
                item.content.clear();
            }
            continue;
        }

        if item.content.len() > BLOB_THRESHOLD_CONTENT {
            let bytes = item.content.as_bytes();
            let hash = upload_blob_cached(
                host,
                config,
                transport,
                layout,
                state,
                BLOB_KIND_CONTENT,
                bytes,
            )
            .await?;
            item.content_blob_hash = Some(hash);
            item.content.clear();
        }
        if item
            .html_content
            .as_ref()
            .is_some_and(|html| html.len() > BLOB_THRESHOLD_HTML)
        {
            let html = item.html_content.take().unwrap_or_default();
            let hash = upload_blob_cached(
                host,
                config,
                transport,
                layout,
                state,
                BLOB_KIND_HTML,
                html.as_bytes(),
            )
            .await?;
            item.html_blob_hash = Some(hash);
        }
    }
    Ok(())
}

async fn upload_blob_cached<H: CloudSyncHost>(
    host: &mut H,
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    state: &mut CloudSyncRuntimeState,
    kind: &str,
    bytes: &[u8],
) -> CloudSyncRunnerResult<String> {
    let hash = sha256_hex(bytes);
    let cache_key = blob_cache_key(config, kind, &hash);
    if !state.blob_cache.contains_key(&cache_key) {
        let uploaded_hash = transport
            .upload_blob(&layout.blobs_path, kind, bytes)
            .await?;
        if uploaded_hash != hash {
            return Err(CloudSyncRunnerError::Protocol(
                "uploaded blob hash mismatch".to_owned(),
            ));
        }
    }
    state.blob_cache.insert(cache_key, host.now_ms());
    Ok(hash)
}

async fn hydrate_incoming_blobs<H: CloudSyncHost>(
    host: &mut H,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    items: &mut [CloudSyncItem],
) -> CloudSyncRunnerResult<()> {
    for item in items {
        if item.deleted_at > 0 {
            continue;
        }
        if let Some(hash) = item.content_blob_hash.as_deref() {
            let kind = if item.content_type == "image" {
                BLOB_KIND_IMAGE
            } else {
                BLOB_KIND_CONTENT
            };
            let bytes = download_verified_blob(transport, &layout.blobs_path, kind, hash).await?;
            if item.content_type == "image" {
                let data_url = image_data_url_from_blob_bytes(&bytes).ok_or_else(|| {
                    CloudSyncRunnerError::Protocol(format!(
                        "unsupported image blob payload: {hash}"
                    ))
                })?;
                item.content = host.materialize_remote_image(&data_url)?;
            } else {
                item.content = String::from_utf8(bytes).map_err(|_| {
                    CloudSyncRunnerError::Protocol(format!(
                        "content blob is not valid UTF-8: {hash}"
                    ))
                })?;
            }
        }
        if let Some(hash) = item.html_blob_hash.as_deref() {
            let bytes =
                download_verified_blob(transport, &layout.blobs_path, BLOB_KIND_HTML, hash).await?;
            item.html_content = Some(String::from_utf8(bytes).map_err(|_| {
                CloudSyncRunnerError::Protocol(format!("HTML blob is not valid UTF-8: {hash}"))
            })?);
        }
    }
    Ok(())
}

async fn download_verified_blob(
    transport: &WebDavTransport,
    blobs_path: &str,
    kind: &str,
    expected_hash: &str,
) -> CloudSyncRunnerResult<Vec<u8>> {
    let bytes = transport
        .download_blob(blobs_path, kind, expected_hash)
        .await?;
    if bytes.len() > MAX_REMOTE_BLOB_BYTES {
        return Err(CloudSyncRunnerError::Protocol(format!(
            "remote blob exceeds the {} byte limit",
            MAX_REMOTE_BLOB_BYTES
        )));
    }
    if sha256_hex(&bytes) != expected_hash.to_ascii_lowercase() {
        return Err(CloudSyncRunnerError::Protocol(
            "downloaded blob hash mismatch".to_owned(),
        ));
    }
    Ok(bytes)
}

async fn cleanup_local_ops(
    config: &CloudSyncRunnerConfig,
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    max_seq: i64,
) -> CloudSyncRunnerResult<usize> {
    if max_seq <= 0 {
        return Ok(0);
    }
    let mut deleted = 0usize;
    for reference in transport.list_op_references(&layout.ops_path).await? {
        if !same_device_id(&reference.device_id, &config.device_id) || reference.seq > max_seq {
            continue;
        }
        transport
            .delete_if_exists(&op_document_path(
                &layout.ops_path,
                &reference.device_id,
                reference.seq,
            ))
            .await?;
        deleted = deleted.saturating_add(1);
    }
    Ok(deleted)
}

async fn fetch_snapshot(
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    device_id: &str,
) -> CloudSyncRunnerResult<Option<WebDavDeviceSnapshot>> {
    Ok(transport
        .get_json(
            &device_document_path(&layout.devices_path, device_id),
            404,
            "webdav GET snapshot failed",
            "parse snapshot json failed",
        )
        .await?)
}

async fn fetch_settings_snapshot(
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    device_id: &str,
) -> CloudSyncRunnerResult<Option<WebDavSettingsSnapshot>> {
    Ok(transport
        .get_json(
            &device_document_path(&layout.settings_path, device_id),
            404,
            "webdav GET settings snapshot failed",
            "parse settings snapshot json failed",
        )
        .await?)
}

async fn fetch_ops_batch(
    transport: &WebDavTransport,
    layout: &WebDavLayout,
    reference: &WebDavOpReference,
) -> CloudSyncRunnerResult<Option<WebDavOpsBatch>> {
    Ok(transport
        .get_json(
            &op_document_path(&layout.ops_path, &reference.device_id, reference.seq),
            404,
            "webdav GET ops batch failed",
            "parse ops batch json failed",
        )
        .await?)
}

fn device_document_path(collection: &str, device_id: &str) -> String {
    format!("{}/{}.json", collection.trim_end_matches('/'), device_id)
}

fn op_document_path(collection: &str, device_id: &str, seq: i64) -> String {
    format!(
        "{}/{}__{:020}.json",
        collection.trim_end_matches('/'),
        device_id,
        seq.max(0)
    )
}

fn blob_cache_key(config: &CloudSyncRunnerConfig, kind: &str, hash: &str) -> String {
    format!(
        "{}|{}|{}|{}",
        config.webdav_url.trim_end_matches('/'),
        config.webdav_base_path.trim_matches('/'),
        kind,
        hash
    )
}

fn prune_blob_cache(cache: &mut HashMap<String, i64>) {
    if cache.len() <= MAX_BLOB_CACHE_ENTRIES {
        return;
    }
    let mut entries = cache
        .iter()
        .map(|(key, timestamp)| (key.clone(), *timestamp))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    cache.clear();
    cache.extend(entries.into_iter().take(MAX_BLOB_CACHE_ENTRIES));
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn image_data_url_from_blob_bytes(bytes: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        let trimmed = text.trim();
        if trimmed.starts_with("data:image/") {
            return Some(trimmed.to_owned());
        }
    }
    let mime = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        image::ImageFormat::Bmp => "image/bmp",
        _ => return None,
    };
    let payload = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{payload}"))
}

fn is_safe_device_id(device_id: &str) -> bool {
    normalize_device_id(device_id).is_some()
        && device_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteOpPlan {
    pub references: Vec<WebDavOpReference>,
    pub truncated: bool,
}

pub fn plan_remote_ops_from_head(
    head: &WebDavSyncHead,
    cursors: &BTreeMap<String, i64>,
    local_device_id: &str,
    max_ops: usize,
) -> RemoteOpPlan {
    let mut plan = RemoteOpPlan::default();
    if max_ops == 0 {
        plan.truncated = head.devices.iter().any(|(device_id, device)| {
            is_safe_device_id(device_id)
                && !same_device_id(device_id, local_device_id)
                && device.latest_op_seq > cursors.get(device_id).copied().unwrap_or(0)
        });
        return plan;
    }

    for (device_id, device) in &head.devices {
        if !is_safe_device_id(device_id)
            || same_device_id(device_id, local_device_id)
            || device.latest_op_seq <= 0
        {
            continue;
        }
        let cursor = cursors.get(device_id).copied().unwrap_or(0).max(0);
        if device.latest_op_seq <= cursor {
            continue;
        }
        for seq in (cursor + 1)..=device.latest_op_seq {
            if plan.references.len() == max_ops {
                plan.truncated = true;
                return plan;
            }
            plan.references.push(WebDavOpReference {
                device_id: device_id.clone(),
                seq,
            });
        }
    }
    plan
}

pub fn remote_snapshot_candidates(
    head: &WebDavSyncHead,
    local_device_id: &str,
    max_snapshots: usize,
) -> Vec<String> {
    let mut candidates = head
        .devices
        .iter()
        .filter_map(|(device_id, device)| {
            (is_safe_device_id(device_id)
                && !same_device_id(device_id, local_device_id)
                && device.snapshot_updated_at > 0)
                .then(|| (device_id.clone(), device.snapshot_updated_at))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    candidates
        .into_iter()
        .take(max_snapshots)
        .map(|(device_id, _)| device_id)
        .collect()
}

pub fn newest_settings_source(
    head: &WebDavSyncHead,
    local_device_id: &str,
) -> Option<(String, i64)> {
    head.devices
        .iter()
        .filter(|(device_id, device)| {
            is_safe_device_id(device_id)
                && !same_device_id(device_id, local_device_id)
                && device.settings_updated_at > 0
        })
        .map(|(device_id, device)| (device_id.clone(), device.settings_updated_at))
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)))
}

pub fn update_head_device<F>(head: &mut WebDavSyncHead, device_id: &str, update: F)
where
    F: FnOnce(&mut WebDavDeviceHead),
{
    let device = head.devices.entry(device_id.to_owned()).or_default();
    update(device);
}

pub fn should_run_periodic_snapshot(last_ts: i64, now: i64, interval_secs: i64) -> bool {
    if last_ts <= 0 {
        return true;
    }
    now.saturating_sub(last_ts) >= interval_secs.saturating_mul(1_000)
}

pub fn should_pull_snapshot(
    force: bool,
    last_pull_at: i64,
    now: i64,
    has_remote_cursor: bool,
    snapshot_interval_secs: i64,
) -> bool {
    if force {
        return true;
    }
    let interval = if has_remote_cursor {
        snapshot_interval_secs
    } else {
        (5 * 60).min(snapshot_interval_secs)
    };
    should_run_periodic_snapshot(last_pull_at, now, interval)
}

pub fn normalize_device_id(device_id: &str) -> Option<String> {
    let trimmed = device_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    let short = trimmed.split('-').next()?;
    ((short.len() == 8 || short.len() == 9)
        && short.chars().all(|character| character.is_ascii_hexdigit()))
    .then(|| short.to_owned())
}

pub fn same_device_id(left: &str, right: &str) -> bool {
    let left = normalize_device_id(left);
    let right = normalize_device_id(right);
    left.is_some() && left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_sync_protocol::{
        collapse_items_by_sync_key, compute_sync_content_hash, HASH_VERSION_WHITESPACE,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    struct TestWebDav {
        endpoint: String,
        resources: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        stopping: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }

    impl TestWebDav {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let resources = Arc::new(Mutex::new(BTreeMap::new()));
            let stopping = Arc::new(AtomicBool::new(false));
            let worker_resources = resources.clone();
            let worker_stopping = stopping.clone();
            let worker = thread::spawn(move || {
                while !worker_stopping.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            handle_webdav_request(&mut stream, &worker_resources);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                endpoint: format!("http://{address}"),
                resources,
                stopping,
                worker: Some(worker),
            }
        }

        fn insert_json<T: Serialize>(&self, path: &str, value: &T) {
            self.resources
                .lock()
                .unwrap()
                .insert(path.to_owned(), serde_json::to_vec(value).unwrap());
        }

        fn insert_bytes(&self, path: &str, bytes: &[u8]) {
            self.resources
                .lock()
                .unwrap()
                .insert(path.to_owned(), bytes.to_vec());
        }

        fn json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> T {
            let bytes = self.resources.lock().unwrap().get(path).cloned().unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }

        fn contains(&self, path: &str) -> bool {
            self.resources.lock().unwrap().contains_key(path)
        }

        fn has_temporary_resources(&self) -> bool {
            self.resources
                .lock()
                .unwrap()
                .keys()
                .any(|path| path.contains(".uploading."))
        }
    }

    impl Drop for TestWebDav {
        fn drop(&mut self) {
            self.stopping.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(self.endpoint.trim_start_matches("http://"));
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn handle_webdav_request(
        stream: &mut TcpStream,
        resources: &Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    ) {
        let Some((method, path, headers, body)) = read_http_request(stream) else {
            return;
        };
        match method.as_str() {
            "MKCOL" => write_http_response(stream, 201, "Created", &[], "text/plain"),
            "PUT" => {
                resources.lock().unwrap().insert(path, body);
                write_http_response(stream, 201, "Created", &[], "text/plain");
            }
            "MOVE" => {
                let destination = headers
                    .get("destination")
                    .and_then(|value| absolute_url_path(value));
                let moved = destination.and_then(|destination| {
                    let mut guard = resources.lock().unwrap();
                    guard
                        .remove(&path)
                        .map(|bytes| guard.insert(destination, bytes))
                });
                if moved.is_some() {
                    write_http_response(stream, 201, "Created", &[], "text/plain");
                } else {
                    write_http_response(stream, 404, "Not Found", &[], "text/plain");
                }
            }
            "GET" => {
                let body = resources.lock().unwrap().get(&path).cloned();
                if let Some(body) = body {
                    write_http_response(stream, 200, "OK", &body, "application/json");
                } else {
                    write_http_response(stream, 404, "Not Found", &[], "text/plain");
                }
            }
            "DELETE" => {
                resources.lock().unwrap().remove(&path);
                write_http_response(stream, 204, "No Content", &[], "text/plain");
            }
            "PROPFIND" => {
                let collection = format!("{}/", path.trim_end_matches('/'));
                let mut hrefs = vec![collection.clone()];
                hrefs.extend(
                    resources
                        .lock()
                        .unwrap()
                        .keys()
                        .filter(|resource| {
                            resource
                                .strip_prefix(&collection)
                                .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
                        })
                        .cloned(),
                );
                let body = format!(
                    "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">{}</d:multistatus>",
                    hrefs
                        .into_iter()
                        .map(|href| format!("<d:response><d:href>{href}</d:href></d:response>"))
                        .collect::<String>()
                );
                write_http_response(
                    stream,
                    207,
                    "Multi-Status",
                    body.as_bytes(),
                    "application/xml",
                );
            }
            _ => write_http_response(stream, 405, "Method Not Allowed", &[], "text/plain"),
        }
    }

    fn read_http_request(
        stream: &mut TcpStream,
    ) -> Option<(String, String, HashMap<String, String>, Vec<u8>)> {
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut buffer).ok()?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let header = std::str::from_utf8(&bytes[..header_end]).ok()?;
        let mut lines = header.split("\r\n");
        let mut request_line = lines.next()?.split_whitespace();
        let method = request_line.next()?.to_owned();
        let path = request_line.next()?.to_owned();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect::<HashMap<_, _>>();
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while bytes.len().saturating_sub(header_end) < content_length {
            let read = stream.read(&mut buffer).ok()?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        let body_end = header_end.saturating_add(content_length).min(bytes.len());
        Some((method, path, headers, bytes[header_end..body_end].to_vec()))
    }

    fn absolute_url_path(value: &str) -> Option<String> {
        let after_scheme = value.split_once("://")?.1;
        let slash = after_scheme.find('/').unwrap_or(after_scheme.len());
        Some(if slash == after_scheme.len() {
            "/".to_owned()
        } else {
            after_scheme[slash..].to_owned()
        })
    }

    fn write_http_response(
        stream: &mut TcpStream,
        status: u16,
        reason: &str,
        body: &[u8],
        content_type: &str,
    ) {
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    #[derive(Default)]
    struct MemoryHost {
        now: i64,
        cancelled: bool,
        state: CloudSyncRuntimeState,
        local_items: Vec<CloudSyncItem>,
        delta_items: Vec<CloudSyncItem>,
        local_index: BTreeMap<String, CloudSyncItem>,
        applied_items: Vec<CloudSyncItem>,
        local_settings: HashMap<String, String>,
        applied_settings: HashMap<String, String>,
        emoji: Option<CloudSyncItem>,
        uploaded_emoji_hash: Option<i64>,
        events: Vec<CloudSyncHostEvent>,
    }

    impl CloudSyncHost for MemoryHost {
        fn now_ms(&self) -> i64 {
            self.now
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled
        }

        fn load_runtime_state(&mut self) -> CloudSyncHostResult<CloudSyncRuntimeState> {
            Ok(self.state.clone())
        }

        fn save_runtime_state(&mut self, state: &CloudSyncRuntimeState) -> CloudSyncHostResult<()> {
            self.state = state.clone();
            Ok(())
        }

        fn collect_local_items(
            &mut self,
            _preferences: &CloudSyncContentPrefs,
        ) -> CloudSyncHostResult<Vec<CloudSyncItem>> {
            Ok(self.local_items.clone())
        }

        fn collect_local_delta(
            &mut self,
            _local_items: &[CloudSyncItem],
        ) -> CloudSyncHostResult<CloudSyncLocalDelta> {
            Ok(CloudSyncLocalDelta {
                items: self.delta_items.clone(),
                collapsed_index: collapse_items_by_sync_key(&self.local_items),
            })
        }

        fn replace_local_index(
            &mut self,
            collapsed_index: &BTreeMap<String, CloudSyncItem>,
        ) -> CloudSyncHostResult<()> {
            self.local_index = collapsed_index.clone();
            Ok(())
        }

        fn apply_remote_items(
            &mut self,
            remote_items: &[CloudSyncItem],
            _preferences: &CloudSyncContentPrefs,
        ) -> CloudSyncHostResult<usize> {
            self.applied_items.extend_from_slice(remote_items);
            Ok(remote_items.len())
        }

        fn prepare_upload_items(
            &mut self,
            _items: &mut [CloudSyncItem],
        ) -> CloudSyncHostResult<()> {
            Ok(())
        }

        fn materialize_remote_image(&mut self, data_url: &str) -> CloudSyncHostResult<String> {
            Ok(data_url.to_owned())
        }

        fn collect_syncable_settings(&mut self) -> CloudSyncHostResult<HashMap<String, String>> {
            Ok(self.local_settings.clone())
        }

        fn apply_synced_settings(
            &mut self,
            incoming: &HashMap<String, String>,
        ) -> CloudSyncHostResult<usize> {
            let changed = incoming
                .iter()
                .filter(|(key, value)| self.applied_settings.get(*key) != Some(*value))
                .count();
            self.applied_settings.extend(incoming.clone());
            Ok(changed)
        }

        fn next_emoji_operation(&mut self) -> CloudSyncHostResult<Option<CloudSyncItem>> {
            Ok(self.emoji.take())
        }

        fn mark_emoji_uploaded(&mut self, content_hash: i64) {
            self.uploaded_emoji_hash = Some(content_hash);
        }

        fn emit(&mut self, event: CloudSyncHostEvent) {
            self.events.push(event);
        }
    }

    fn runner_config(endpoint: &str, device_id: &str) -> CloudSyncRunnerConfig {
        CloudSyncRunnerConfig::new(
            device_id,
            endpoint,
            "user",
            "secret",
            "tiez-sync",
            120,
            43_200,
            CloudSyncContentPrefs::default(),
        )
    }

    fn text_item(content: &str, updated_at: i64, updated_by: &str) -> CloudSyncItem {
        CloudSyncItem {
            content_type: "text".to_owned(),
            content: content.to_owned(),
            content_hash: compute_sync_content_hash("text", content),
            hash_version: HASH_VERSION_WHITESPACE,
            deleted_at: 0,
            html_content: None,
            content_blob_hash: None,
            html_blob_hash: None,
            source_app: "Tests".to_owned(),
            timestamp: updated_at,
            updated_at,
            updated_by: updated_by.to_owned(),
            preview: content.chars().take(80).collect(),
            is_pinned: false,
            tags: Vec::new(),
            use_count: 1,
            pinned_order: 0,
        }
    }

    fn head_with(devices: &[(&str, i64, i64, i64)]) -> WebDavSyncHead {
        let devices = devices
            .iter()
            .map(
                |(id, latest_op_seq, snapshot_updated_at, settings_updated_at)| {
                    (
                        (*id).to_owned(),
                        WebDavDeviceHead {
                            latest_op_seq: *latest_op_seq,
                            snapshot_updated_at: *snapshot_updated_at,
                            snapshot_op_seq: 0,
                            settings_updated_at: *settings_updated_at,
                        },
                    )
                },
            )
            .collect();
        WebDavSyncHead {
            updated_at: 0,
            devices,
        }
    }

    #[tokio::test]
    async fn runner_publishes_local_delta_only_after_head_commit() {
        let server = TestWebDav::start();
        let now = 1_700_000_000_000;
        let item = text_item("保留首尾空格  ", now - 10, "aaaaaaaa");
        let mut host = MemoryHost {
            now,
            state: CloudSyncRuntimeState {
                last_snapshot_push_at: now,
                last_snapshot_pull_at: now,
                ..CloudSyncRuntimeState::default()
            },
            local_items: vec![item.clone()],
            delta_items: vec![item],
            ..MemoryHost::default()
        };

        let status = run_webdav_once(
            &mut host,
            &runner_config(&server.endpoint, "aaaaaaaa"),
            false,
        )
        .await
        .unwrap();

        assert_eq!(status, CloudSyncRunStatus::idle(now, 1, 0));
        assert_eq!(host.state.local_op_seq, 1);
        assert_eq!(host.state.cursor, now);
        assert_eq!(host.local_index.len(), 1);
        assert!(server.contains("/tiez-sync/ops/aaaaaaaa__00000000000000000001.json"));
        let head: WebDavSyncHead = server.json("/tiez-sync/head.json");
        assert_eq!(head.devices["aaaaaaaa"].latest_op_seq, 1);
        assert!(!server.has_temporary_resources());
        assert!(matches!(
            host.events.first(),
            Some(CloudSyncHostEvent::Status(status)) if status.state == "syncing"
        ));
        assert!(matches!(
            host.events.last(),
            Some(CloudSyncHostEvent::Status(status)) if status.state == "idle"
        ));
    }

    #[tokio::test]
    async fn forced_snapshot_publishes_settings_then_cleans_covered_ops() {
        let server = TestWebDav::start();
        let now = 1_700_000_000_000;
        let item = text_item("快照正文", now - 10, "aaaaaaaa");
        let mut host = MemoryHost {
            now,
            local_items: vec![item.clone()],
            delta_items: vec![item],
            local_settings: HashMap::from([("app.language".to_owned(), "zh-CN".to_owned())]),
            ..MemoryHost::default()
        };

        let status = run_webdav_once(
            &mut host,
            &runner_config(&server.endpoint, "aaaaaaaa"),
            true,
        )
        .await
        .unwrap();

        assert_eq!(status.uploaded_items, 3);
        assert_eq!(host.state.last_snapshot_push_at, now);
        assert_eq!(host.state.last_snapshot_pull_at, now);
        assert!(!server.contains("/tiez-sync/ops/aaaaaaaa__00000000000000000001.json"));
        let snapshot: WebDavDeviceSnapshot = server.json("/tiez-sync/devices/aaaaaaaa.json");
        assert_eq!(snapshot.latest_op_seq, 1);
        assert_eq!(snapshot.entries.len(), 1);
        let settings: WebDavSettingsSnapshot = server.json("/tiez-sync/settings/aaaaaaaa.json");
        assert_eq!(settings.settings["app.language"], "zh-CN");
        let head: WebDavSyncHead = server.json("/tiez-sync/head.json");
        assert_eq!(head.devices["aaaaaaaa"].snapshot_op_seq, 1);
        assert_eq!(
            head.devices["aaaaaaaa"].settings_updated_at,
            settings.updated_at
        );
    }

    #[tokio::test]
    async fn cold_start_pulls_remote_snapshot_and_newest_settings() {
        let server = TestWebDav::start();
        let now = 1_700_000_000_000;
        let remote_item = text_item("远端快照", now - 20, "bbbbbbbb");
        let snapshot = WebDavDeviceSnapshot {
            device_id: "bbbbbbbb".to_owned(),
            updated_at: now - 10,
            latest_op_seq: 0,
            entries: vec![remote_item.clone()],
        };
        let settings = WebDavSettingsSnapshot {
            device_id: "bbbbbbbb".to_owned(),
            updated_at: now - 5,
            settings: HashMap::from([("app.language".to_owned(), "zh-CN".to_owned())]),
        };
        let head = head_with(&[("bbbbbbbb", 0, snapshot.updated_at, settings.updated_at)]);
        server.insert_json("/tiez-sync/head.json", &head);
        server.insert_json("/tiez-sync/devices/bbbbbbbb.json", &snapshot);
        server.insert_json("/tiez-sync/settings/bbbbbbbb.json", &settings);
        let mut host = MemoryHost {
            now,
            state: CloudSyncRuntimeState {
                last_snapshot_push_at: now,
                last_head_rebuild_at: now,
                ..CloudSyncRuntimeState::default()
            },
            ..MemoryHost::default()
        };

        let status = run_webdav_once(
            &mut host,
            &runner_config(&server.endpoint, "aaaaaaaa"),
            false,
        )
        .await
        .unwrap();

        assert_eq!(status.received_items, 1);
        assert_eq!(host.applied_items, vec![remote_item]);
        assert_eq!(host.applied_settings["app.language"], "zh-CN");
        assert_eq!(host.state.settings_applied_at, settings.updated_at);
        assert_eq!(host.state.last_snapshot_pull_at, now);
        assert!(host
            .events
            .iter()
            .any(|event| matches!(event, CloudSyncHostEvent::SettingsChanged)));
    }

    #[tokio::test]
    async fn runner_pulls_verified_blob_and_advances_remote_cursor() {
        let server = TestWebDav::start();
        let now = 1_700_000_000_000;
        let content = "云端正文".repeat(2_000);
        let blob_hash = sha256_hex(content.as_bytes());
        let mut remote_item = text_item(&content, now - 20, "bbbbbbbb");
        remote_item.content.clear();
        remote_item.content_blob_hash = Some(blob_hash.clone());
        let batch = WebDavOpsBatch {
            device_id: "bbbbbbbb".to_owned(),
            seq: 1,
            updated_at: now - 10,
            entries: vec![remote_item],
        };
        let head = head_with(&[("bbbbbbbb", 1, 0, 0)]);
        server.insert_json("/tiez-sync/head.json", &head);
        server.insert_json("/tiez-sync/ops/bbbbbbbb__00000000000000000001.json", &batch);
        server.insert_bytes(
            &format!(
                "/tiez-sync/blobs/{}/content_{}.blob",
                &blob_hash[..2],
                blob_hash
            ),
            content.as_bytes(),
        );
        let mut host = MemoryHost {
            now,
            state: CloudSyncRuntimeState {
                last_snapshot_push_at: now,
                last_snapshot_pull_at: now,
                last_head_rebuild_at: now,
                ..CloudSyncRuntimeState::default()
            },
            ..MemoryHost::default()
        };

        let status = run_webdav_once(
            &mut host,
            &runner_config(&server.endpoint, "aaaaaaaa"),
            false,
        )
        .await
        .unwrap();

        assert_eq!(status.received_items, 1);
        assert_eq!(host.state.op_cursors["bbbbbbbb"], 1);
        assert_eq!(host.applied_items[0].content, content);
        assert!(host
            .events
            .iter()
            .any(|event| matches!(event, CloudSyncHostEvent::HistoryChanged)));
    }

    #[tokio::test]
    async fn runner_rejects_tampered_blob_without_advancing_cursor() {
        let server = TestWebDav::start();
        let now = 1_700_000_000_000;
        let expected = b"expected remote body";
        let blob_hash = sha256_hex(expected);
        let mut remote_item = text_item("expected remote body", now - 20, "bbbbbbbb");
        remote_item.content.clear();
        remote_item.content_blob_hash = Some(blob_hash.clone());
        server.insert_json("/tiez-sync/head.json", &head_with(&[("bbbbbbbb", 1, 0, 0)]));
        server.insert_json(
            "/tiez-sync/ops/bbbbbbbb__00000000000000000001.json",
            &WebDavOpsBatch {
                device_id: "bbbbbbbb".to_owned(),
                seq: 1,
                updated_at: now - 10,
                entries: vec![remote_item],
            },
        );
        server.insert_bytes(
            &format!(
                "/tiez-sync/blobs/{}/content_{}.blob",
                &blob_hash[..2],
                blob_hash
            ),
            b"tampered remote body",
        );
        let mut host = MemoryHost {
            now,
            state: CloudSyncRuntimeState {
                last_snapshot_push_at: now,
                last_snapshot_pull_at: now,
                last_head_rebuild_at: now,
                ..CloudSyncRuntimeState::default()
            },
            ..MemoryHost::default()
        };

        let error = run_webdav_once(
            &mut host,
            &runner_config(&server.endpoint, "aaaaaaaa"),
            false,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, CloudSyncRunnerError::Protocol(_)));
        assert!(!host.state.op_cursors.contains_key("bbbbbbbb"));
        assert!(host.applied_items.is_empty());
        assert!(matches!(
            host.events.last(),
            Some(CloudSyncHostEvent::Status(status)) if status.state == "error"
        ));
    }

    #[tokio::test]
    async fn cancelled_runner_does_not_contact_webdav() {
        let mut host = MemoryHost {
            cancelled: true,
            ..MemoryHost::default()
        };
        let status = run_webdav_once(
            &mut host,
            &runner_config("http://127.0.0.1:1", "aaaaaaaa"),
            false,
        )
        .await
        .unwrap();
        assert_eq!(status, CloudSyncRunStatus::disabled());
        assert_eq!(host.events.len(), 1);
    }

    #[test]
    fn runtime_state_deserializes_older_empty_documents() {
        let state: CloudSyncRuntimeState = serde_json::from_str("{}").unwrap();
        assert_eq!(state, CloudSyncRuntimeState::default());
    }

    #[test]
    fn device_aliases_match_the_existing_short_id_contract() {
        assert!(same_device_id("abcdef12", "abcdef12-peer-name"));
        assert!(same_device_id("abcdef123", "abcdef123-legacy"));
        assert!(!same_device_id("abcdef12", "ABCDEF12-peer-name"));
        assert!(!same_device_id("device-a", "device-a"));
        assert!(!same_device_id("", ""));
    }

    #[test]
    fn remote_op_plan_resumes_cursors_skips_local_and_is_bounded() {
        let head = head_with(&[
            ("aaaaaaaa", 9_000_000, 0, 0),
            ("bbbbbbbb", 4, 0, 0),
            ("cccccccc", 2, 0, 0),
        ]);
        let cursors = BTreeMap::from([("bbbbbbbb".to_owned(), 2)]);
        let plan = plan_remote_ops_from_head(&head, &cursors, "aaaaaaaa-local", 3);
        assert_eq!(
            plan.references,
            vec![
                WebDavOpReference {
                    device_id: "bbbbbbbb".to_owned(),
                    seq: 3,
                },
                WebDavOpReference {
                    device_id: "bbbbbbbb".to_owned(),
                    seq: 4,
                },
                WebDavOpReference {
                    device_id: "cccccccc".to_owned(),
                    seq: 1,
                },
            ]
        );
        assert!(plan.truncated);

        let malicious = head_with(&[("bbbbbbbb/../../head", 99, 99, 99)]);
        assert!(
            plan_remote_ops_from_head(&malicious, &BTreeMap::new(), "aaaaaaaa", 10)
                .references
                .is_empty()
        );
        assert!(remote_snapshot_candidates(&malicious, "aaaaaaaa", 10).is_empty());
        assert_eq!(newest_settings_source(&malicious, "aaaaaaaa"), None);
    }

    #[test]
    fn snapshot_and_settings_plans_use_stable_timestamp_ordering() {
        let head = head_with(&[
            ("aaaaaaaa", 0, 999, 999),
            ("bbbbbbbb", 0, 20, 30),
            ("cccccccc", 0, 20, 30),
            ("dddddddd", 0, 10, 40),
        ]);
        assert_eq!(
            remote_snapshot_candidates(&head, "aaaaaaaa-local", 2),
            vec!["bbbbbbbb", "cccccccc"]
        );
        assert_eq!(
            newest_settings_source(&head, "aaaaaaaa-local"),
            Some(("dddddddd".to_owned(), 40))
        );
    }

    #[test]
    fn snapshot_schedule_keeps_cold_start_fallback() {
        let now = 1_000_000;
        assert!(should_pull_snapshot(false, 0, now, false, 43_200));
        assert!(!should_pull_snapshot(
            false,
            now - 100_000,
            now,
            false,
            43_200
        ));
        assert!(should_pull_snapshot(true, now, now, true, 43_200));
    }

    #[test]
    fn status_values_preserve_the_existing_frontend_contract() {
        assert_eq!(CloudSyncRunStatus::syncing().state, "syncing");
        assert_eq!(CloudSyncRunStatus::disabled().state, "disabled");
        assert_eq!(CloudSyncRunStatus::failed("network").state, "error");
        assert_eq!(CloudSyncRunStatus::idle(42, 2, 3).last_sync_at, Some(42));
    }
}
