//! Stable cloud-sync wire models and deterministic conflict identity rules.
//!
//! The types in this module intentionally retain the existing snake_case JSON
//! contract. They contain no Tauri handles, database connections, or frontend
//! state, so both desktop runtimes can use the same remote representation.

use crate::content_identity::{
    calc_image_hash, calc_legacy_text_hash, calc_text_hash, uses_text_content_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const HASH_VERSION_LEGACY: i64 = 1;
pub const HASH_VERSION_WHITESPACE: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSyncContentPrefs {
    #[serde(default = "default_true")]
    pub text: bool,
    #[serde(default = "default_true")]
    pub image: bool,
    #[serde(rename = "file_path", default = "default_true")]
    pub file_path: bool,
    #[serde(default = "default_true")]
    pub emoji: bool,
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
    pub fn includes_content_type(&self, content_type: &str) -> bool {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSyncItem {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavDeviceSnapshot {
    pub device_id: String,
    pub updated_at: i64,
    #[serde(default)]
    pub latest_op_seq: i64,
    pub entries: Vec<CloudSyncItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavSettingsSnapshot {
    pub device_id: String,
    pub updated_at: i64,
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavOpsBatch {
    pub device_id: String,
    pub seq: i64,
    pub updated_at: i64,
    pub entries: Vec<CloudSyncItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavDeviceHead {
    #[serde(default)]
    pub latest_op_seq: i64,
    #[serde(default)]
    pub snapshot_updated_at: i64,
    #[serde(default)]
    pub snapshot_op_seq: i64,
    #[serde(default)]
    pub settings_updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavSyncHead {
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub devices: BTreeMap<String, WebDavDeviceHead>,
}

pub fn is_cloud_clipboard_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "text" | "code" | "url" | "rich_text" | "image" | "file" | "video" | "emoji_sync"
    )
}

pub fn uses_text_sync_hash(content_type: &str) -> bool {
    uses_text_content_hash(content_type)
}

pub fn compute_sync_content_hash(content_type: &str, content: &str) -> i64 {
    match content_type {
        "image" => calc_image_hash(content).unwrap_or(0),
        content_type if uses_text_sync_hash(content_type) => calc_text_hash(content) as i64,
        _ => 0,
    }
}

pub fn compute_legacy_sync_content_hash(content_type: &str, content: &str) -> i64 {
    if uses_text_sync_hash(content_type) {
        calc_legacy_text_hash(content) as i64
    } else {
        compute_sync_content_hash(content_type, content)
    }
}

pub fn resolved_content_hash(item: &CloudSyncItem) -> i64 {
    if item.content_hash != 0 {
        item.content_hash
    } else {
        compute_sync_content_hash(&item.content_type, &item.content)
    }
}

pub fn item_updated_at(item: &CloudSyncItem) -> i64 {
    if item.updated_at > 0 {
        item.updated_at
    } else {
        item.timestamp
    }
}

pub fn item_revision(item: &CloudSyncItem) -> i64 {
    item.deleted_at.max(item_updated_at(item))
}

pub fn sync_key_for_item(item: &CloudSyncItem) -> Option<String> {
    let hash = resolved_content_hash(item);
    if hash == 0 {
        return None;
    }
    Some(format!(
        "{}:{}:{}",
        item.content_type, item.hash_version, hash
    ))
}

pub fn sync_digest_for_item(item: &CloudSyncItem) -> String {
    let tags_json = serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".to_owned());
    let html_hash = item
        .html_content
        .as_ref()
        .map(|value| calc_text_hash(value))
        .unwrap_or(0);
    let preview_hash = calc_text_hash(&item.preview);
    let source_hash = calc_text_hash(&item.source_app);
    let metadata = format!(
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
        calc_text_hash(&tags_json)
    );
    calc_text_hash(&metadata).to_string()
}

pub fn collapse_items_by_sync_key(items: &[CloudSyncItem]) -> BTreeMap<String, CloudSyncItem> {
    let mut collapsed = BTreeMap::new();
    for item in items {
        let Some(key) = sync_key_for_item(item) else {
            continue;
        };
        let mut normalized = item.clone();
        normalized.content_hash = resolved_content_hash(item);
        let replace = collapsed
            .get(&key)
            .map(|old: &CloudSyncItem| {
                (item_revision(&normalized), normalized.updated_by.as_str())
                    >= (item_revision(old), old.updated_by.as_str())
            })
            .unwrap_or(true);
        if replace {
            collapsed.insert(key, normalized);
        }
    }
    collapsed
}

const fn default_true() -> bool {
    true
}

const fn default_hash_version() -> i64 {
    HASH_VERSION_LEGACY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(content: &str, updated_at: i64, updated_by: &str) -> CloudSyncItem {
        CloudSyncItem {
            content_type: "text".to_owned(),
            content: content.to_owned(),
            content_hash: compute_sync_content_hash("text", content),
            hash_version: HASH_VERSION_WHITESPACE,
            deleted_at: 0,
            html_content: None,
            content_blob_hash: None,
            html_blob_hash: None,
            source_app: "tests".to_owned(),
            timestamp: 10,
            updated_at,
            updated_by: updated_by.to_owned(),
            preview: content.to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            use_count: 0,
            pinned_order: 0,
        }
    }

    #[test]
    fn preferences_keep_the_existing_file_path_json_contract() {
        let preferences: CloudSyncContentPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(preferences, CloudSyncContentPrefs::default());
        assert!(preferences.includes_content_type("rich_text"));
        assert!(!preferences.includes_content_type("unsupported"));
        assert_eq!(
            serde_json::to_value(&preferences).unwrap()["file_path"],
            true
        );
    }

    #[test]
    fn legacy_item_json_defaults_match_existing_peers() {
        let value = serde_json::json!({
            "content_type": "text",
            "content": "hello",
            "source_app": "peer",
            "timestamp": 42,
            "preview": "hello"
        });
        let parsed: CloudSyncItem = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.hash_version, HASH_VERSION_LEGACY);
        assert_eq!(parsed.updated_at, 0);
        assert!(parsed.tags.is_empty());
        assert_eq!(item_updated_at(&parsed), 42);
    }

    #[test]
    fn whitespace_identity_and_legacy_identity_remain_distinct() {
        let exact = compute_sync_content_hash("text", "hello ");
        let trimmed = compute_sync_content_hash("text", "hello");
        assert_ne!(exact, trimmed);
        assert_eq!(
            compute_legacy_sync_content_hash("text", " hello \r\n"),
            compute_legacy_sync_content_hash("text", "hello")
        );
    }

    #[test]
    fn collapse_prefers_revision_then_device_id_and_materializes_hash() {
        let old = item("same", 20, "z-device");
        let mut winning = old.clone();
        winning.content_hash = 0;
        winning.updated_by = "zz-device".to_owned();
        let collapsed = collapse_items_by_sync_key(&[old, winning]);
        let value = collapsed.values().next().unwrap();
        assert_eq!(value.updated_by, "zz-device");
        assert_ne!(value.content_hash, 0);
    }

    #[test]
    fn digest_tracks_sync_metadata_but_not_blob_transport_fields() {
        let base = item("payload", 20, "device");
        let mut metadata = base.clone();
        metadata.is_pinned = true;
        assert_ne!(sync_digest_for_item(&base), sync_digest_for_item(&metadata));

        let mut blob_backed = base.clone();
        blob_backed.content_blob_hash = Some("a".repeat(64));
        assert_eq!(
            sync_digest_for_item(&base),
            sync_digest_for_item(&blob_backed)
        );
    }

    #[test]
    fn webdav_head_defaults_allow_older_head_documents() {
        let parsed: WebDavSyncHead = serde_json::from_str(r#"{"devices":{"peer":{}}}"#).unwrap();
        assert_eq!(parsed.updated_at, 0);
        assert_eq!(parsed.devices["peer"], WebDavDeviceHead::default());
    }
}
