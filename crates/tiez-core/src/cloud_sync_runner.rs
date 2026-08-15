//! Runtime ports and deterministic scheduling decisions for native cloud sync.
//!
//! Network behavior lives in `cloud_sync_webdav`; wire models and conflict
//! identity live in `cloud_sync_protocol`. This module defines the remaining
//! boundary between the shared runner and a desktop host's database, files,
//! settings, cancellation signal, and UI notifications.

use crate::cloud_sync_protocol::{
    CloudSyncContentPrefs, CloudSyncItem, WebDavDeviceHead, WebDavSyncHead,
};
use crate::cloud_sync_webdav::WebDavOpReference;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;

pub const DEFAULT_MAX_OPS_PER_RUN: usize = 2_000;
pub const DEFAULT_MAX_REMOTE_SNAPSHOTS: usize = 24;

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
pub trait CloudSyncHost {
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
            !same_device_id(device_id, local_device_id)
                && device.latest_op_seq > cursors.get(device_id).copied().unwrap_or(0)
        });
        return plan;
    }

    for (device_id, device) in &head.devices {
        if same_device_id(device_id, local_device_id) || device.latest_op_seq <= 0 {
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
            (!same_device_id(device_id, local_device_id) && device.snapshot_updated_at > 0)
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
            !same_device_id(device_id, local_device_id) && device.settings_updated_at > 0
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
