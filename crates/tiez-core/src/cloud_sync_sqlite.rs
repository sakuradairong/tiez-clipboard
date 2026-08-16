//! Writable SQLite host for the shared cloud-sync runner.
//!
//! This adapter is used by native desktop runtimes that own the production
//! TieZ database directly. It contains no Tauri handles or WinUI objects.

use crate::cloud_sync_protocol::{
    collapse_items_by_sync_key, compute_legacy_sync_content_hash, compute_sync_content_hash,
    is_cloud_clipboard_content_type, is_cloud_sync_setting_eligible, item_revision,
    item_updated_at, resolved_content_hash, sync_digest_for_item, sync_key_for_item,
    CloudSyncContentPrefs, CloudSyncItem, HASH_VERSION_LEGACY, HASH_VERSION_WHITESPACE,
};
use crate::cloud_sync_runner::{
    CloudSyncHost, CloudSyncHostError, CloudSyncHostErrorKind, CloudSyncHostEvent,
    CloudSyncHostResult, CloudSyncLocalDelta, CloudSyncRuntimeState,
};
use crate::encryption::{decrypt_value, encrypt_value, ENCRYPT_PREFIX};
use base64::Engine;
use regex::Regex;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const KEY_LOCAL_SEQ: &str = "cloud_sync_webdav_local_seq";
const KEY_OP_CURSORS: &str = "cloud_sync_webdav_op_cursor_map";
const KEY_BLOB_CACHE: &str = "cloud_sync_webdav_blob_cache";
const KEY_LAST_SNAPSHOT_PUSH: &str = "cloud_sync_webdav_last_snapshot_push_at";
const KEY_LAST_SNAPSHOT_PULL: &str = "cloud_sync_webdav_last_snapshot_pull_at";
const KEY_LAST_HEAD_REBUILD: &str = "cloud_sync_webdav_last_head_rebuild_at";
const KEY_SETTINGS_APPLIED: &str = "cloud_sync_settings_applied_at";
const KEY_CURSOR: &str = "cloud_sync_cursor";
const KEY_EMOJI_FAVORITES: &str = "app.emoji_favorites";
const KEY_LAST_EMOJI_HASH: &str = "cloud_sync_webdav_last_emoji_hash";
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const RICH_IMAGE_FALLBACK_PREFIX: &str = "<!--TIEZ_RICH_IMAGE:";
const RICH_IMAGE_FALLBACK_SUFFIX: &str = "-->";

const SENSITIVE_TAGS: &[&str] = &["sensitive", "密码", "password"];

pub struct SqliteCloudSyncHost {
    database_path: PathBuf,
    data_dir: PathBuf,
    cancelled: Arc<AtomicBool>,
    event_sink: Option<Box<dyn FnMut(CloudSyncHostEvent) + Send>>,
    last_emoji_hash: i64,
}

impl SqliteCloudSyncHost {
    pub fn new(
        database_path: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        cancelled: Arc<AtomicBool>,
    ) -> CloudSyncHostResult<Self> {
        let database_path = database_path.into();
        let data_dir = data_dir.into();
        let connection = open_connection(&database_path)?;
        connection
            .query_row("SELECT 1 FROM settings LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(storage_error)?;
        Ok(Self {
            database_path,
            data_dir,
            cancelled,
            event_sink: None,
            last_emoji_hash: setting_i64(&connection, KEY_LAST_EMOJI_HASH),
        })
    }

    pub fn with_event_sink(
        mut self,
        event_sink: impl FnMut(CloudSyncHostEvent) + Send + 'static,
    ) -> Self {
        self.event_sink = Some(Box::new(event_sink));
        self
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    fn connection(&self) -> CloudSyncHostResult<Connection> {
        open_connection(&self.database_path)
    }
}

pub fn ensure_cloud_sync_device_id(database_path: impl AsRef<Path>) -> CloudSyncHostResult<String> {
    let connection = open_connection(database_path.as_ref())?;
    if let Some(existing) = setting_value(&connection, "app.anon_id") {
        let trimmed = existing.trim();
        let short = trimmed.split('-').next().unwrap_or_default();
        if (short.len() == 8 || short.len() == 9)
            && short.chars().all(|character| character.is_ascii_hexdigit())
        {
            return Ok(short.to_owned());
        }
    }

    let identity = format!(
        "{}|{}|{}",
        std::env::var("COMPUTERNAME").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default(),
        database_path.as_ref().to_string_lossy()
    );
    let digest = sha256_hex(identity.as_bytes());
    let device_id = digest[..8].to_owned();
    upsert_setting(&connection, "app.anon_id", &device_id)?;
    Ok(device_id)
}

impl CloudSyncHost for SqliteCloudSyncHost {
    fn now_ms(&self) -> i64 {
        now_ms()
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn load_runtime_state(&mut self) -> CloudSyncHostResult<CloudSyncRuntimeState> {
        let connection = self.connection()?;
        Ok(CloudSyncRuntimeState {
            local_op_seq: setting_i64(&connection, KEY_LOCAL_SEQ),
            op_cursors: setting_json(&connection, KEY_OP_CURSORS),
            blob_cache: setting_json(&connection, KEY_BLOB_CACHE),
            last_snapshot_push_at: setting_i64(&connection, KEY_LAST_SNAPSHOT_PUSH),
            last_snapshot_pull_at: setting_i64(&connection, KEY_LAST_SNAPSHOT_PULL),
            last_head_rebuild_at: setting_i64(&connection, KEY_LAST_HEAD_REBUILD),
            settings_applied_at: setting_i64(&connection, KEY_SETTINGS_APPLIED),
            cursor: setting_i64(&connection, KEY_CURSOR),
        })
    }

    fn save_runtime_state(&mut self, state: &CloudSyncRuntimeState) -> CloudSyncHostResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let values = [
            (KEY_LOCAL_SEQ, state.local_op_seq.to_string()),
            (
                KEY_OP_CURSORS,
                serde_json::to_string(&state.op_cursors).map_err(internal_error)?,
            ),
            (
                KEY_BLOB_CACHE,
                serde_json::to_string(&state.blob_cache).map_err(internal_error)?,
            ),
            (
                KEY_LAST_SNAPSHOT_PUSH,
                state.last_snapshot_push_at.to_string(),
            ),
            (
                KEY_LAST_SNAPSHOT_PULL,
                state.last_snapshot_pull_at.to_string(),
            ),
            (
                KEY_LAST_HEAD_REBUILD,
                state.last_head_rebuild_at.to_string(),
            ),
            (KEY_SETTINGS_APPLIED, state.settings_applied_at.to_string()),
            (KEY_CURSOR, state.cursor.to_string()),
        ];
        for (key, value) in values {
            upsert_setting(&transaction, key, &value)?;
        }
        transaction.commit().map_err(storage_error)
    }

    fn collect_local_items(
        &mut self,
        preferences: &CloudSyncContentPrefs,
    ) -> CloudSyncHostResult<Vec<CloudSyncItem>> {
        let connection = self.connection()?;
        collect_local_items(&connection, preferences)
    }

    fn collect_local_delta(
        &mut self,
        local_items: &[CloudSyncItem],
    ) -> CloudSyncHostResult<CloudSyncLocalDelta> {
        let connection = self.connection()?;
        let previous = load_local_index(&connection)?;
        let collapsed_index = collapse_items_by_sync_key(local_items);
        let mut items = collapsed_index
            .iter()
            .filter_map(|(key, item)| {
                let digest = sync_digest_for_item(item);
                (previous.get(key) != Some(&digest)).then(|| item.clone())
            })
            .collect::<Vec<_>>();
        items.sort_by_key(item_revision);
        Ok(CloudSyncLocalDelta {
            items,
            collapsed_index,
        })
    }

    fn replace_local_index(
        &mut self,
        collapsed_index: &BTreeMap<String, CloudSyncItem>,
    ) -> CloudSyncHostResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        transaction
            .execute("DELETE FROM cloud_sync_local_index", [])
            .map_err(storage_error)?;
        for (key, item) in collapsed_index {
            transaction
                .execute(
                    "INSERT INTO cloud_sync_local_index (sync_key, digest) VALUES (?1, ?2)",
                    params![key, sync_digest_for_item(item)],
                )
                .map_err(storage_error)?;
        }
        transaction.commit().map_err(storage_error)
    }

    fn apply_remote_items(
        &mut self,
        remote_items: &[CloudSyncItem],
        preferences: &CloudSyncContentPrefs,
    ) -> CloudSyncHostResult<usize> {
        let connection = self.connection()?;
        let mut applied = 0usize;
        for remote in remote_items {
            let mut item = remote.clone();
            canonicalize_live_hash(&mut item);
            if item.content_type == "emoji_sync" {
                if preferences.emoji {
                    let outcome = merge_remote_emojis(&connection, &self.data_dir, &item.content)?;
                    if let Some(content_hash) = outcome.suppression_hash {
                        self.last_emoji_hash = content_hash;
                        upsert_setting(
                            &connection,
                            KEY_LAST_EMOJI_HASH,
                            &content_hash.to_string(),
                        )?;
                    }
                    if outcome.changed {
                        applied = applied.saturating_add(1);
                    }
                }
                continue;
            }
            if !is_cloud_clipboard_content_type(&item.content_type)
                || !preferences.includes_content_type(&item.content_type)
            {
                continue;
            }
            applied = applied.saturating_add(if item.deleted_at > 0 {
                apply_tombstone(&connection, &self.data_dir, &item)?
            } else if item.content.is_empty() {
                0
            } else {
                apply_live_item(&connection, &item, now_ms())?
            });
        }
        Ok(applied)
    }

    fn prepare_upload_items(&mut self, items: &mut [CloudSyncItem]) -> CloudSyncHostResult<()> {
        for item in items {
            if item.deleted_at > 0 {
                continue;
            }
            if item.content_type == "image" && !item.content.starts_with("data:image/") {
                item.content = image_path_to_data_url(&item.content)?;
            }
            if item.content_type == "rich_text" {
                if let Some(html) = item.html_content.as_mut() {
                    *html = rewrite_rich_html_resources_for_sync(html);
                }
            }
        }
        Ok(())
    }

    fn materialize_remote_image(&mut self, data_url: &str) -> CloudSyncHostResult<String> {
        persist_image_attachment(&self.data_dir, data_url)
    }

    fn collect_syncable_settings(&mut self) -> CloudSyncHostResult<HashMap<String, String>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT key, value FROM settings")
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage_error)?;
        let mut settings = HashMap::new();
        for row in rows {
            let (key, value) = row.map_err(storage_error)?;
            if setting_is_syncable(&key) {
                settings.insert(key, value);
            }
        }
        Ok(settings)
    }

    fn apply_synced_settings(
        &mut self,
        incoming: &HashMap<String, String>,
    ) -> CloudSyncHostResult<usize> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let mut changed = 0usize;
        for (key, value) in incoming {
            if !setting_is_syncable(key) {
                continue;
            }
            let current = transaction
                .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                    row.get::<_, String>(0)
                })
                .optional()
                .map_err(storage_error)?;
            if current.as_deref() == Some(value) {
                continue;
            }
            upsert_setting(&transaction, key, value)?;
            changed = changed.saturating_add(1);
        }
        transaction.commit().map_err(storage_error)?;
        Ok(changed)
    }

    fn next_emoji_operation(&mut self) -> CloudSyncHostResult<Option<CloudSyncItem>> {
        let connection = self.connection()?;
        let stored = setting_value(&connection, KEY_EMOJI_FAVORITES).unwrap_or_default();
        let paths = serde_json::from_str::<Vec<String>>(&stored).unwrap_or_default();
        let Some((content, content_hash)) = emoji_sync_payload(&paths)? else {
            return Ok(None);
        };
        if content_hash == self.last_emoji_hash {
            return Ok(None);
        }
        let timestamp = now_ms();
        Ok(Some(CloudSyncItem {
            content_type: "emoji_sync".to_owned(),
            content: content.clone(),
            content_hash,
            hash_version: HASH_VERSION_WHITESPACE,
            deleted_at: 0,
            html_content: None,
            content_blob_hash: None,
            html_blob_hash: None,
            source_app: "TieZ Emoji".to_owned(),
            timestamp,
            updated_at: timestamp,
            updated_by: setting_value(&connection, "app.anon_id").unwrap_or_default(),
            preview: "Emoji favorites sync".to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            use_count: 0,
            pinned_order: 0,
        }))
    }

    fn mark_emoji_uploaded(&mut self, content_hash: i64) {
        self.last_emoji_hash = content_hash;
        if let Ok(connection) = self.connection() {
            let _ = upsert_setting(&connection, KEY_LAST_EMOJI_HASH, &content_hash.to_string());
        }
    }

    fn emit(&mut self, event: CloudSyncHostEvent) {
        if let Some(sink) = self.event_sink.as_mut() {
            sink(event);
        }
    }
}

fn collect_local_items(
    connection: &Connection,
    preferences: &CloudSyncContentPrefs,
) -> CloudSyncHostResult<Vec<CloudSyncItem>> {
    let mut statement = connection
        .prepare(
            "SELECT content_type, content, content_hash, content_hash_version, html_content,
                    source_app, timestamp, sync_updated_at, sync_updated_by, preview,
                    is_pinned, tags, use_count, pinned_order
             FROM clipboard_history ORDER BY id ASC",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok(CloudSyncItem {
                content_type: row.get(0)?,
                content: row.get(1)?,
                content_hash: row.get(2)?,
                hash_version: row.get(3)?,
                deleted_at: 0,
                html_content: row.get(4)?,
                content_blob_hash: None,
                html_blob_hash: None,
                source_app: row.get(5)?,
                timestamp: row.get(6)?,
                updated_at: row.get(7)?,
                updated_by: row.get(8)?,
                preview: row.get(9)?,
                is_pinned: row.get::<_, i32>(10)? == 1,
                tags: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
                use_count: row.get(12)?,
                pinned_order: row.get(13)?,
            })
        })
        .map_err(storage_error)?;
    let mut items = Vec::new();
    for row in rows {
        let mut item = row.map_err(storage_error)?;
        if !preferences.includes_content_type(&item.content_type) {
            continue;
        }
        let Some(content) = decode_stored(&item.content) else {
            continue;
        };
        item.content = content;
        item.html_content = item.html_content.and_then(|value| decode_stored(&value));
        item.preview = decode_stored(&item.preview).unwrap_or_default();
        if item.content_hash == 0 {
            item.content_hash = compute_sync_content_hash(&item.content_type, &item.content);
        }
        items.push(item);
    }

    let mut statement = connection
        .prepare(
            "SELECT content_type, content_hash, hash_version, deleted_at
             FROM cloud_sync_tombstones ORDER BY deleted_at ASC",
        )
        .map_err(storage_error)?;
    let tombstones = statement
        .query_map([], |row| {
            let deleted_at = row.get(3)?;
            Ok(CloudSyncItem {
                content_type: row.get(0)?,
                content: String::new(),
                content_hash: row.get(1)?,
                hash_version: row.get(2)?,
                deleted_at,
                html_content: None,
                content_blob_hash: None,
                html_blob_hash: None,
                source_app: "sync".to_owned(),
                timestamp: deleted_at,
                updated_at: deleted_at,
                updated_by: String::new(),
                preview: String::new(),
                is_pinned: false,
                tags: Vec::new(),
                use_count: 0,
                pinned_order: 0,
            })
        })
        .map_err(storage_error)?;
    for row in tombstones {
        let item = row.map_err(storage_error)?;
        if preferences.includes_content_type(&item.content_type) {
            items.push(item);
        }
    }
    items.sort_by_key(item_revision);
    Ok(items)
}

fn apply_live_item(
    connection: &Connection,
    item: &CloudSyncItem,
    fallback_timestamp: i64,
) -> CloudSyncHostResult<usize> {
    with_savepoint(connection, || {
        let remote_hash = resolved_content_hash(item);
        if remote_hash != 0 && matching_tombstone(connection, item)? >= item_updated_at(item) {
            return Ok(0);
        }
        if let Some(id) = find_existing(connection, item)? {
            return update_existing(connection, id, item);
        }

        let sensitive = has_sensitive_tag(&item.tags);
        let content = encode_stored(&item.content, sensitive)?;
        let html = item
            .html_content
            .as_deref()
            .map(|value| encode_stored(value, sensitive))
            .transpose()?;
        let preview_plain = if item.preview.is_empty() {
            if item.content_type == "image" {
                "[Image Content]".to_owned()
            } else {
                item.content.chars().take(200).collect()
            }
        } else {
            item.preview.clone()
        };
        let preview = encode_stored(&preview_plain, sensitive)?;
        let tags = clean_tags(&item.tags);
        let tags_json = serde_json::to_string(&tags).map_err(internal_error)?;
        let timestamp = if item.timestamp > 0 {
            item.timestamp
        } else {
            fallback_timestamp
        };
        connection
            .execute(
                "INSERT INTO clipboard_history
                    (content_type, content, html_content, source_app, timestamp, preview,
                     is_pinned, content_hash, content_hash_version, tags, use_count,
                     is_external, pinned_order, sync_updated_at, sync_updated_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    item.content_type,
                    content,
                    html,
                    item.source_app,
                    timestamp,
                    preview,
                    i32::from(item.is_pinned),
                    remote_hash,
                    item.hash_version,
                    tags_json,
                    item.use_count,
                    i32::from(matches!(
                        item.content_type.as_str(),
                        "image" | "file" | "video"
                    )),
                    item.pinned_order,
                    item_updated_at(item),
                    item.updated_by,
                ],
            )
            .map_err(storage_error)?;
        let id = connection.last_insert_rowid();
        sync_tags(connection, id, &tags)?;
        clear_matching_tombstones(connection, item)?;
        let persisted = load_sync_item(connection, id)?;
        record_digest(connection, &persisted)?;
        Ok(1)
    })
}

fn update_existing(
    connection: &Connection,
    id: i64,
    item: &CloudSyncItem,
) -> CloudSyncHostResult<usize> {
    let local = connection
        .query_row(
            "SELECT content, html_content, timestamp, is_pinned, pinned_order, preview,
                    source_app, use_count, tags, sync_updated_at, sync_updated_by
             FROM clipboard_history WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i32>(3)? == 1,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i32>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .map_err(storage_error)?;
    let local_content = decode_stored(&local.0).unwrap_or(local.0.clone());
    let local_html = local.1.as_deref().and_then(decode_stored);
    let local_preview = decode_stored(&local.5).unwrap_or_default();
    let remote_tags = clean_tags(&item.tags);
    let remote_tags_json = serde_json::to_string(&remote_tags).map_err(internal_error)?;
    let remote_version = (item_updated_at(item), item.updated_by.as_str());
    let local_version = (local.9, local.10.as_str());
    let remote_key = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        item.timestamp,
        item.is_pinned,
        item.pinned_order,
        item.preview,
        item.source_app,
        item.use_count,
        remote_tags_json,
        item.content,
        item.html_content.as_deref().unwrap_or_default()
    );
    let local_key = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        local.2,
        local.3,
        local.4,
        local_preview,
        local.6,
        local.7,
        local.8,
        local_content,
        local_html.as_deref().unwrap_or_default()
    );
    let remote_wins = remote_version > local_version
        || (remote_version == local_version && remote_key > local_key);
    let remote_accepted = remote_version > local_version
        || (remote_version == local_version && remote_key >= local_key);
    let max_use_count = local.7.max(item.use_count);
    if remote_wins {
        let sensitive = has_sensitive_tag(&remote_tags);
        let content = encode_stored(&item.content, sensitive)?;
        let html = item
            .html_content
            .as_deref()
            .map(|value| encode_stored(value, sensitive))
            .transpose()?;
        let preview = encode_stored(
            if item.preview.is_empty() {
                &local_preview
            } else {
                &item.preview
            },
            sensitive,
        )?;
        connection
            .execute(
                "UPDATE clipboard_history SET content = ?1, html_content = ?2, timestamp = ?3,
                    is_pinned = ?4, pinned_order = ?5, preview = ?6, source_app = ?7,
                    use_count = ?8, tags = ?9, is_external = ?10, sync_updated_at = ?11,
                    sync_updated_by = ?12, content_hash = ?13, content_hash_version = ?14,
                    source_app_path = NULL WHERE id = ?15",
                params![
                    content,
                    html,
                    item.timestamp,
                    i32::from(item.is_pinned),
                    item.pinned_order,
                    preview,
                    item.source_app,
                    max_use_count,
                    remote_tags_json,
                    i32::from(matches!(
                        item.content_type.as_str(),
                        "image" | "file" | "video"
                    )),
                    item_updated_at(item),
                    item.updated_by,
                    resolved_content_hash(item),
                    item.hash_version,
                    id,
                ],
            )
            .map_err(storage_error)?;
        sync_tags(connection, id, &remote_tags)?;
    } else if max_use_count > local.7 {
        connection
            .execute(
                "UPDATE clipboard_history SET use_count = ?1 WHERE id = ?2",
                params![max_use_count, id],
            )
            .map_err(storage_error)?;
    }
    if resolved_content_hash(item) != 0 {
        clear_matching_tombstones(connection, item)?;
    }
    if remote_accepted {
        let persisted = load_sync_item(connection, id)?;
        record_digest(connection, &persisted)?;
    }
    Ok(usize::from(remote_wins || max_use_count > local.7))
}

fn apply_tombstone(
    connection: &Connection,
    data_dir: &Path,
    item: &CloudSyncItem,
) -> CloudSyncHostResult<usize> {
    let remote_hash = resolved_content_hash(item);
    if remote_hash == 0 || item.deleted_at <= 0 {
        return Ok(0);
    }
    let attachments_dir = data_dir.join("attachments");
    let (applied, cleanup_paths) = with_savepoint(connection, || {
        let version = item.hash_version.max(HASH_VERSION_LEGACY);
        connection
            .execute(
                "INSERT INTO cloud_sync_tombstones
                    (content_type, content_hash, hash_version, deleted_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(content_type, content_hash, hash_version)
                 DO UPDATE SET deleted_at = MAX(cloud_sync_tombstones.deleted_at, excluded.deleted_at)",
                params![item.content_type, remote_hash, version, item.deleted_at],
            )
            .map_err(storage_error)?;
        let mut statement = connection
            .prepare(
                "SELECT id, content, content_hash, content_hash_version,
                        sync_updated_at, sync_updated_by, html_content, is_external
                 FROM clipboard_history WHERE content_type = ?1",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([&item.content_type], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i32>(7)? == 1,
                ))
            })
            .map_err(storage_error)?;
        let mut matching = Vec::new();
        for row in rows {
            let row = row.map_err(storage_error)?;
            if tombstone_matches(item, &row.1, row.2, row.3) {
                matching.push(row);
            }
        }
        drop(statement);
        let mut applied = 0usize;
        let mut accepted = true;
        let mut cleanup_paths = HashSet::new();
        for (id, content, _, _, updated_at, updated_by, html, is_external) in matching {
            if (item.deleted_at, item.updated_by.as_str()) < (updated_at, updated_by.as_str()) {
                accepted = false;
                continue;
            }
            cleanup_paths.extend(collect_attachment_paths_for_cleanup(
                &content,
                html.as_deref(),
                is_external,
                &attachments_dir,
            ));
            connection
                .execute("DELETE FROM entry_tags WHERE entry_id = ?1", [id])
                .map_err(storage_error)?;
            connection
                .execute("DELETE FROM clipboard_history WHERE id = ?1", [id])
                .map_err(storage_error)?;
            applied = applied.saturating_add(1);
        }
        if accepted {
            record_digest(connection, item)?;
        }
        Ok((applied, cleanup_paths))
    })?;
    cleanup_unreferenced_attachment_paths(connection, cleanup_paths, &attachments_dir);
    Ok(applied)
}

fn find_existing(
    connection: &Connection,
    item: &CloudSyncItem,
) -> CloudSyncHostResult<Option<i64>> {
    let hash = resolved_content_hash(item);
    if hash != 0 {
        return connection
            .query_row(
                "SELECT id FROM clipboard_history
                 WHERE content_type = ?1 AND content_hash = ?2
                   AND ((?3 >= 2 AND content_hash_version >= 2)
                     OR (?3 <= 1 AND content_hash_version <= 1))
                 ORDER BY id DESC LIMIT 1",
                params![item.content_type, hash, item.hash_version],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error);
    }
    connection
        .query_row(
            "SELECT id FROM clipboard_history WHERE content_type = ?1 AND content = ?2
             ORDER BY id DESC LIMIT 1",
            params![item.content_type, item.content],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)
}

fn matching_tombstone(connection: &Connection, item: &CloudSyncItem) -> CloudSyncHostResult<i64> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(deleted_at), 0) FROM cloud_sync_tombstones
             WHERE content_type = ?1
               AND ((hash_version <= 1 AND content_hash IN (?2, ?3))
                 OR (?4 >= 2 AND hash_version >= 2 AND content_hash = ?3))",
            params![
                item.content_type,
                compute_legacy_sync_content_hash(&item.content_type, &item.content),
                resolved_content_hash(item),
                item.hash_version,
            ],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

fn clear_matching_tombstones(
    connection: &Connection,
    item: &CloudSyncItem,
) -> CloudSyncHostResult<()> {
    connection
        .execute(
            "DELETE FROM cloud_sync_tombstones
             WHERE content_type = ?1 AND deleted_at <= ?4
               AND ((hash_version <= 1 AND content_hash IN (?2, ?3))
                 OR (?5 >= 2 AND hash_version >= 2 AND content_hash = ?3))",
            params![
                item.content_type,
                compute_legacy_sync_content_hash(&item.content_type, &item.content),
                resolved_content_hash(item),
                item_updated_at(item),
                item.hash_version,
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn tombstone_matches(
    item: &CloudSyncItem,
    stored_content: &str,
    stored_hash: i64,
    stored_version: i64,
) -> bool {
    let remote_hash = resolved_content_hash(item);
    if item.hash_version <= HASH_VERSION_LEGACY {
        stored_hash == remote_hash
            || decode_stored(stored_content).is_some_and(|plain| {
                compute_legacy_sync_content_hash(&item.content_type, &plain) == remote_hash
            })
    } else {
        stored_version >= HASH_VERSION_WHITESPACE && stored_hash == remote_hash
    }
}

fn canonicalize_live_hash(item: &mut CloudSyncItem) {
    if item.deleted_at > 0 {
        return;
    }
    if matches!(
        item.content_type.as_str(),
        "text" | "code" | "url" | "rich_text" | "file" | "video" | "emoji_sync"
    ) {
        item.content_hash = compute_sync_content_hash(&item.content_type, &item.content);
        item.hash_version = HASH_VERSION_WHITESPACE;
    }
}

fn record_digest(connection: &Connection, item: &CloudSyncItem) -> CloudSyncHostResult<()> {
    let Some(key) = sync_key_for_item(item) else {
        return Ok(());
    };
    connection
        .execute(
            "INSERT INTO cloud_sync_local_index (sync_key, digest) VALUES (?1, ?2)
             ON CONFLICT(sync_key) DO UPDATE SET digest = excluded.digest",
            params![key, sync_digest_for_item(item)],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn load_sync_item(connection: &Connection, id: i64) -> CloudSyncHostResult<CloudSyncItem> {
    let mut item = connection
        .query_row(
            "SELECT content_type, content, content_hash, content_hash_version, html_content,
                    source_app, timestamp, sync_updated_at, sync_updated_by, preview,
                    is_pinned, tags, use_count, pinned_order
             FROM clipboard_history WHERE id = ?1",
            [id],
            |row| {
                Ok(CloudSyncItem {
                    content_type: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get(2)?,
                    hash_version: row.get(3)?,
                    deleted_at: 0,
                    html_content: row.get(4)?,
                    content_blob_hash: None,
                    html_blob_hash: None,
                    source_app: row.get(5)?,
                    timestamp: row.get(6)?,
                    updated_at: row.get(7)?,
                    updated_by: row.get(8)?,
                    preview: row.get(9)?,
                    is_pinned: row.get::<_, i32>(10)? == 1,
                    tags: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
                    use_count: row.get(12)?,
                    pinned_order: row.get(13)?,
                })
            },
        )
        .map_err(storage_error)?;
    item.content = decode_stored(&item.content).ok_or_else(|| {
        CloudSyncHostError::new(
            CloudSyncHostErrorKind::Storage,
            "failed to decrypt persisted cloud-sync content",
        )
    })?;
    item.html_content = item.html_content.and_then(|value| decode_stored(&value));
    item.preview = decode_stored(&item.preview).unwrap_or_default();
    Ok(item)
}

fn load_local_index(connection: &Connection) -> CloudSyncHostResult<HashMap<String, String>> {
    let mut statement = connection
        .prepare("SELECT sync_key, digest FROM cloud_sync_local_index")
        .map_err(storage_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(storage_error)?;
    let mut index = HashMap::new();
    for row in rows {
        let (key, digest) = row.map_err(storage_error)?;
        index.insert(key, digest);
    }
    Ok(index)
}

struct EmojiMergeOutcome {
    changed: bool,
    suppression_hash: Option<i64>,
}

fn merge_remote_emojis(
    connection: &Connection,
    data_dir: &Path,
    remote_json: &str,
) -> CloudSyncHostResult<EmojiMergeOutcome> {
    let local_json = setting_value(connection, KEY_EMOJI_FAVORITES).unwrap_or_default();
    let local = serde_json::from_str::<Vec<String>>(&local_json).unwrap_or_default();
    let remote_payload = serde_json::from_str::<Vec<String>>(remote_json).map_err(|error| {
        CloudSyncHostError::new(
            CloudSyncHostErrorKind::Payload,
            format!("invalid emoji sync payload: {error}"),
        )
    })?;
    let mut remote = remote_payload
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            if item.starts_with("data:image/") {
                persist_image_attachment(data_dir, item).ok()
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    remote.sort();
    remote.dedup();
    let mut merged = local
        .iter()
        .chain(remote.iter())
        .cloned()
        .collect::<HashSet<_>>();
    let mut merged = merged.drain().collect::<Vec<_>>();
    merged.sort();
    let changed = merged != local;
    if changed {
        upsert_setting(
            connection,
            KEY_EMOJI_FAVORITES,
            &serde_json::to_string(&merged).map_err(internal_error)?,
        )?;
    }
    let suppression_hash = if remote == merged {
        emoji_sync_payload(&merged)?.map(|(_, content_hash)| content_hash)
    } else {
        None
    };
    Ok(EmojiMergeOutcome {
        changed,
        suppression_hash,
    })
}

fn emoji_sync_payload(paths: &[String]) -> CloudSyncHostResult<Option<(String, i64)>> {
    let encoded = paths
        .iter()
        .filter_map(|path| image_path_to_data_url(path).ok())
        .collect::<Vec<_>>();
    if encoded.is_empty() {
        return Ok(None);
    }
    let content = serde_json::to_string(&encoded).map_err(internal_error)?;
    let content_hash = compute_sync_content_hash("emoji_sync", &content);
    Ok(Some((content, content_hash)))
}

fn sync_tags(connection: &Connection, id: i64, tags: &[String]) -> CloudSyncHostResult<()> {
    connection
        .execute("DELETE FROM entry_tags WHERE entry_id = ?1", [id])
        .map_err(storage_error)?;
    for tag in tags {
        connection
            .execute(
                "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                params![id, tag],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.iter()
        .filter_map(|tag| {
            let tag = tag.trim();
            (!tag.is_empty() && seen.insert(tag.to_owned())).then(|| tag.to_owned())
        })
        .collect()
}

fn has_sensitive_tag(tags: &[String]) -> bool {
    tags.iter().any(|tag| {
        SENSITIVE_TAGS
            .iter()
            .any(|sensitive| sensitive.eq_ignore_ascii_case(tag))
    })
}

fn encode_stored(value: &str, sensitive: bool) -> CloudSyncHostResult<String> {
    if !sensitive {
        return Ok(value.to_owned());
    }
    encrypt_value(value).ok_or_else(|| {
        CloudSyncHostError::new(
            CloudSyncHostErrorKind::Storage,
            "failed to encrypt sensitive cloud-sync value",
        )
    })
}

fn decode_stored(value: &str) -> Option<String> {
    if value.starts_with(ENCRYPT_PREFIX) {
        decrypt_value(value)
    } else {
        Some(value.to_owned())
    }
}

fn local_rich_resource_path(raw: &str) -> Option<PathBuf> {
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

    let path_raw = value.strip_prefix("file://").unwrap_or(value);
    let path_without_drive_prefix =
        if path_raw.starts_with('/') && path_raw.chars().nth(2) == Some(':') {
            &path_raw[1..]
        } else {
            path_raw
        };
    let decoded = urlencoding::decode(path_without_drive_prefix)
        .map(|path| path.into_owned())
        .unwrap_or_else(|_| path_without_drive_prefix.to_owned());
    let clean = decoded
        .split('?')
        .next()
        .unwrap_or(&decoded)
        .split('#')
        .next()
        .unwrap_or(&decoded)
        .trim();
    if clean.is_empty() {
        return None;
    }

    let path = PathBuf::from(clean);
    path.is_absolute().then_some(path)
}

fn rich_fallback_payload(html: &str) -> Option<&str> {
    let start = html.rfind(RICH_IMAGE_FALLBACK_PREFIX)? + RICH_IMAGE_FALLBACK_PREFIX.len();
    let end = start + html[start..].find(RICH_IMAGE_FALLBACK_SUFFIX)?;
    let payload = html[start..end].trim();
    (!payload.is_empty()).then_some(payload)
}

fn rewrite_rich_html_resources_for_sync(html: &str) -> String {
    static IMAGE_SOURCE: OnceLock<Regex> = OnceLock::new();
    let expression = IMAGE_SOURCE.get_or_init(|| {
        Regex::new(r#"(?is)(<img\b[^>]*\bsrc=["'])([^"']+)(["'][^>]*>)"#)
            .expect("valid rich image source expression")
    });
    let with_inline_sources = expression
        .replace_all(html, |captures: &regex::Captures| {
            let Some(path) = local_rich_resource_path(&captures[2]) else {
                return captures[0].to_owned();
            };
            let Ok(data_url) = image_path_to_data_url(path.to_string_lossy().as_ref()) else {
                return captures[0].to_owned();
            };
            format!("{}{}{}", &captures[1], data_url, &captures[3])
        })
        .into_owned();

    let Some(payload) = rich_fallback_payload(&with_inline_sources) else {
        return with_inline_sources;
    };
    let Some(path) = local_rich_resource_path(payload) else {
        return with_inline_sources;
    };
    let Ok(data_url) = image_path_to_data_url(path.to_string_lossy().as_ref()) else {
        return with_inline_sources;
    };
    let start = with_inline_sources
        .rfind(RICH_IMAGE_FALLBACK_PREFIX)
        .expect("fallback marker was found")
        + RICH_IMAGE_FALLBACK_PREFIX.len();
    let end = start
        + with_inline_sources[start..]
            .find(RICH_IMAGE_FALLBACK_SUFFIX)
            .expect("fallback suffix was found");
    format!(
        "{}{}{}",
        &with_inline_sources[..start],
        data_url,
        &with_inline_sources[end..]
    )
}

fn rich_html_local_paths(html: &str) -> Vec<PathBuf> {
    static IMAGE_SOURCE: OnceLock<Regex> = OnceLock::new();
    let expression = IMAGE_SOURCE.get_or_init(|| {
        Regex::new(r#"(?is)<img\b[^>]*\bsrc=["']([^"']+)["'][^>]*>"#)
            .expect("valid rich image source expression")
    });
    let mut paths = expression
        .captures_iter(html)
        .filter_map(|captures| local_rich_resource_path(&captures[1]))
        .collect::<Vec<_>>();
    if let Some(path) = rich_fallback_payload(html).and_then(local_rich_resource_path) {
        paths.push(path);
    }
    paths
}

fn canonical_attachment_path(path: &Path, attachments_dir: &Path) -> Option<PathBuf> {
    let attachments = std::fs::canonicalize(attachments_dir).ok()?;
    let candidate = std::fs::canonicalize(path).ok()?;
    candidate.starts_with(&attachments).then_some(candidate)
}

fn collect_attachment_paths_for_cleanup(
    content_raw: &str,
    html_raw: Option<&str>,
    is_external: bool,
    attachments_dir: &Path,
) -> HashSet<PathBuf> {
    let mut paths = HashSet::new();
    if is_external {
        if let Some(content) = decode_stored(content_raw) {
            if let Some(path) = canonical_attachment_path(Path::new(&content), attachments_dir) {
                paths.insert(path);
            }
        }
    }
    if let Some(html) = html_raw.and_then(decode_stored) {
        for path in rich_html_local_paths(&html) {
            if let Some(path) = canonical_attachment_path(&path, attachments_dir) {
                paths.insert(path);
            }
        }
    }
    paths
}

fn cleanup_unreferenced_attachment_paths(
    connection: &Connection,
    cleanup_paths: HashSet<PathBuf>,
    attachments_dir: &Path,
) {
    if cleanup_paths.is_empty() {
        return;
    }
    let cleanup_paths = cleanup_paths
        .into_iter()
        .filter_map(|path| canonical_attachment_path(&path, attachments_dir))
        .collect::<HashSet<_>>();
    if cleanup_paths.is_empty() {
        return;
    }

    let mut statement = match connection
        .prepare("SELECT content, html_content, is_external FROM clipboard_history")
    {
        Ok(statement) => statement,
        Err(_) => return,
    };
    let rows = match statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i32>(2)? == 1,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return,
    };
    let mut referenced = HashSet::new();
    for row in rows {
        let Ok((content_raw, html_raw, is_external)) = row else {
            return;
        };
        if is_external {
            let Some(content) = decode_stored(&content_raw) else {
                return;
            };
            if let Some(path) = canonical_attachment_path(Path::new(&content), attachments_dir) {
                referenced.insert(path);
            }
        }
        if let Some(html_raw) = html_raw {
            let Some(html) = decode_stored(&html_raw) else {
                return;
            };
            for path in rich_html_local_paths(&html) {
                if let Some(path) = canonical_attachment_path(&path, attachments_dir) {
                    referenced.insert(path);
                }
            }
        }
    }
    if let Some(emoji_favorites) = setting_value(connection, KEY_EMOJI_FAVORITES) {
        let Ok(paths) = serde_json::from_str::<Vec<String>>(&emoji_favorites) else {
            return;
        };
        for path in paths {
            if let Some(path) = canonical_attachment_path(Path::new(&path), attachments_dir) {
                referenced.insert(path);
            }
        }
    }

    for path in cleanup_paths.difference(&referenced) {
        let _ = std::fs::remove_file(path);
    }
}

fn image_path_to_data_url(value: &str) -> CloudSyncHostResult<String> {
    let bytes = std::fs::read(value).map_err(|error| {
        CloudSyncHostError::new(
            CloudSyncHostErrorKind::Payload,
            format!("failed to read image payload: {error}"),
        )
    })?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(CloudSyncHostError::new(
            CloudSyncHostErrorKind::Payload,
            "image payload is empty or too large",
        ));
    }
    let mime = image_mime(&bytes).ok_or_else(|| {
        CloudSyncHostError::new(CloudSyncHostErrorKind::Payload, "unsupported image payload")
    })?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn persist_image_attachment(data_dir: &Path, data_url: &str) -> CloudSyncHostResult<String> {
    let (metadata, payload) = data_url.split_once(',').ok_or_else(|| {
        CloudSyncHostError::new(CloudSyncHostErrorKind::Payload, "invalid image data URL")
    })?;
    if !metadata.starts_with("data:image/") || !metadata.contains(";base64") {
        return Err(CloudSyncHostError::new(
            CloudSyncHostErrorKind::Payload,
            "unsupported image data URL",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|error| {
            CloudSyncHostError::new(
                CloudSyncHostErrorKind::Payload,
                format!("invalid image base64: {error}"),
            )
        })?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(CloudSyncHostError::new(
            CloudSyncHostErrorKind::Payload,
            "remote image is empty or too large",
        ));
    }
    let extension = match image::guess_format(&bytes).ok() {
        Some(image::ImageFormat::Jpeg) => "jpg",
        Some(image::ImageFormat::Gif) => "gif",
        Some(image::ImageFormat::WebP) => "webp",
        Some(image::ImageFormat::Bmp) => "bmp",
        Some(image::ImageFormat::Png) => "png",
        _ => {
            return Err(CloudSyncHostError::new(
                CloudSyncHostErrorKind::Payload,
                "unsupported remote image format",
            ))
        }
    };
    let attachments = data_dir.join("attachments");
    std::fs::create_dir_all(&attachments).map_err(storage_error)?;
    let hash = sha256_hex(&bytes);
    let path = attachments.join(format!("sync_{hash}.{extension}"));
    if !path.exists() {
        std::fs::write(&path, bytes).map_err(storage_error)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::Gif => Some("image/gif"),
        image::ImageFormat::WebP => Some("image/webp"),
        image::ImageFormat::Bmp => Some("image/bmp"),
        _ => None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn setting_is_syncable(key: &str) -> bool {
    is_cloud_sync_setting_eligible(key)
}

fn setting_value(connection: &Connection, key: &str) -> Option<String> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .ok()
        .flatten()
}

fn setting_i64(connection: &Connection, key: &str) -> i64 {
    setting_value(connection, key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn setting_json<T: serde::de::DeserializeOwned + Default>(connection: &Connection, key: &str) -> T {
    setting_value(connection, key)
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn upsert_setting(connection: &Connection, key: &str, value: &str) -> CloudSyncHostResult<()> {
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn open_connection(path: &Path) -> CloudSyncHostResult<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(storage_error)
}

fn with_savepoint<T>(
    connection: &Connection,
    operation: impl FnOnce() -> CloudSyncHostResult<T>,
) -> CloudSyncHostResult<T> {
    const SAVEPOINT: &str = "tiez_native_cloud_sync";
    connection
        .execute_batch(&format!("SAVEPOINT {SAVEPOINT}"))
        .map_err(storage_error)?;
    match operation() {
        Ok(value) => {
            connection
                .execute_batch(&format!("RELEASE SAVEPOINT {SAVEPOINT}"))
                .map_err(storage_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = connection.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}"
            ));
            Err(error)
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn storage_error(error: impl std::fmt::Display) -> CloudSyncHostError {
    CloudSyncHostError::new(CloudSyncHostErrorKind::Storage, error.to_string())
}

fn internal_error(error: impl std::fmt::Display) -> CloudSyncHostError {
    CloudSyncHostError::new(CloudSyncHostErrorKind::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database_bootstrap::open_database_with_decrypt;
    use std::fs;

    struct TestDatabase {
        root: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "tiez-cloud-sqlite-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            let path = root.join("clipboard.db");
            let connection = open_database_with_decrypt(&path, decrypt_value).unwrap();
            connection
                .execute(
                    "INSERT INTO settings (key, value) VALUES ('app.anon_id', 'aaaaaaaa')
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [],
                )
                .unwrap();
            drop(connection);
            Self { root, path }
        }

        fn host(&self) -> SqliteCloudSyncHost {
            SqliteCloudSyncHost::new(&self.path, &self.root, Arc::new(AtomicBool::new(false)))
                .unwrap()
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn text_item(content: &str, revision: i64, updated_by: &str) -> CloudSyncItem {
        CloudSyncItem {
            content_type: "text".to_owned(),
            content: content.to_owned(),
            content_hash: compute_sync_content_hash("text", content),
            hash_version: HASH_VERSION_WHITESPACE,
            deleted_at: 0,
            html_content: None,
            content_blob_hash: None,
            html_blob_hash: None,
            source_app: "Remote".to_owned(),
            timestamp: revision,
            updated_at: revision,
            updated_by: updated_by.to_owned(),
            preview: content.to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            use_count: 1,
            pinned_order: 0,
        }
    }

    #[test]
    fn remote_metadata_and_tombstones_follow_revision_order() {
        let database = TestDatabase::new("conflicts");
        let mut host = database.host();
        let preferences = CloudSyncContentPrefs::default();
        let original = text_item("保留尾部空格  ", 100, "bbbbbbbb");
        assert_eq!(
            host.apply_remote_items(&[original.clone()], &preferences)
                .unwrap(),
            1
        );

        let mut newer = original.clone();
        newer.updated_at = 200;
        newer.updated_by = "cccccccc".to_owned();
        newer.is_pinned = true;
        newer.tags = vec!["工作".to_owned()];
        newer.use_count = 4;
        assert_eq!(
            host.apply_remote_items(&[newer.clone()], &preferences)
                .unwrap(),
            1
        );
        let connection = Connection::open(&database.path).unwrap();
        let stored: (String, i32, String, i32, i64) = connection
            .query_row(
                "SELECT content, is_pinned, tags, use_count, sync_updated_at
                 FROM clipboard_history LIMIT 1",
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
            .unwrap();
        assert_eq!(stored.0, original.content);
        assert_eq!(stored.1, 1);
        assert_eq!(stored.2, "[\"工作\"]");
        assert_eq!(stored.3, 4);
        assert_eq!(stored.4, 200);
        drop(connection);

        let mut older = original.clone();
        older.updated_at = 150;
        older.tags = vec!["过期".to_owned()];
        assert_eq!(host.apply_remote_items(&[older], &preferences).unwrap(), 0);

        let mut tombstone = original.clone();
        tombstone.content.clear();
        tombstone.deleted_at = 300;
        tombstone.updated_at = 300;
        tombstone.updated_by = "dddddddd".to_owned();
        assert_eq!(
            host.apply_remote_items(&[tombstone], &preferences).unwrap(),
            1
        );
        let connection = Connection::open(&database.path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn runtime_state_delta_index_and_settings_round_trip() {
        let database = TestDatabase::new("state");
        let mut host = database.host();
        let state = CloudSyncRuntimeState {
            local_op_seq: 7,
            op_cursors: BTreeMap::from([("bbbbbbbb".to_owned(), 9)]),
            blob_cache: HashMap::from([("blob".to_owned(), 11)]),
            last_snapshot_push_at: 12,
            last_snapshot_pull_at: 13,
            last_head_rebuild_at: 14,
            settings_applied_at: 15,
            cursor: 16,
        };
        host.save_runtime_state(&state).unwrap();
        assert_eq!(host.load_runtime_state().unwrap(), state);

        let item = text_item("delta", 100, "bbbbbbbb");
        host.apply_remote_items(&[item], &CloudSyncContentPrefs::default())
            .unwrap();
        let local = host
            .collect_local_items(&CloudSyncContentPrefs::default())
            .unwrap();
        let first = host.collect_local_delta(&local).unwrap();
        assert!(
            first.items.is_empty(),
            "accepted remote rows suppress echoes"
        );
        host.replace_local_index(&first.collapsed_index).unwrap();
        assert!(host.collect_local_delta(&local).unwrap().items.is_empty());

        assert_eq!(
            host.apply_synced_settings(&HashMap::from([
                ("app.theme".to_owned(), "acrylic".to_owned()),
                ("cloud_sync_enabled".to_owned(), "false".to_owned()),
                ("mqtt_password".to_owned(), "must-not-sync".to_owned()),
                ("ai_profiles".to_owned(), "must-not-sync".to_owned()),
            ]))
            .unwrap(),
            1
        );
        let settings = host.collect_syncable_settings().unwrap();
        assert_eq!(
            settings.get("app.theme").map(String::as_str),
            Some("acrylic")
        );
        assert!(!settings.contains_key("cloud_sync_enabled"));
        assert!(!settings.contains_key("mqtt_password"));
        assert!(!settings.contains_key("ai_profiles"));
    }

    #[test]
    fn sensitive_remote_content_is_encrypted_at_rest_and_decrypted_for_sync() {
        let database = TestDatabase::new("privacy");
        let mut host = database.host();
        let mut item = text_item("口令：TieZ-秘密", 100, "bbbbbbbb");
        item.tags = vec!["密码".to_owned()];
        host.apply_remote_items(&[item.clone()], &CloudSyncContentPrefs::default())
            .unwrap();

        let connection = Connection::open(&database.path).unwrap();
        let stored = connection
            .query_row("SELECT content FROM clipboard_history LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        #[cfg(windows)]
        assert!(stored.starts_with(ENCRYPT_PREFIX));
        drop(connection);

        let collected = host
            .collect_local_items(&CloudSyncContentPrefs::default())
            .unwrap();
        assert_eq!(collected[0].content, item.content);
        assert_eq!(collected[0].tags, item.tags);
    }

    #[test]
    fn rich_html_local_images_are_inlined_before_upload() {
        let database = TestDatabase::new("rich-upload");
        let image_path = database.root.join("rich source.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([80, 140, 220, 255]))
            .save(&image_path)
            .unwrap();
        let path = image_path.to_string_lossy();
        let mut item = text_item("rich-content", 100, "bbbbbbbb");
        item.content_type = "rich_text".to_owned();
        item.content_hash = compute_sync_content_hash("rich_text", &item.content);
        item.html_content = Some(format!(
            "<p><img src=\"{path}\"></p>{RICH_IMAGE_FALLBACK_PREFIX}{path}{RICH_IMAGE_FALLBACK_SUFFIX}"
        ));

        database
            .host()
            .prepare_upload_items(std::slice::from_mut(&mut item))
            .unwrap();

        let html = item.html_content.unwrap();
        assert_eq!(html.matches("data:image/png;base64,").count(), 2);
        assert!(!html.contains(path.as_ref()));
        assert!(html.contains(RICH_IMAGE_FALLBACK_PREFIX));
        assert!(html.contains(RICH_IMAGE_FALLBACK_SUFFIX));
    }

    #[test]
    fn remote_tombstones_remove_only_unreferenced_managed_attachments() {
        let database = TestDatabase::new("attachment-cleanup");
        let attachments = database.root.join("attachments");
        fs::create_dir_all(&attachments).unwrap();
        let owned_image = attachments.join("sync_owned.png");
        let owned_rich_image = attachments.join("sync_rich_owned.png");
        let shared_rich_image = attachments.join("sync_rich_shared.png");
        let emoji_shared_image = attachments.join("sync_emoji_shared.png");
        let outside_image = database.root.join("outside.png");
        for (path, color) in [
            (&owned_image, [220, 20, 20, 255]),
            (&owned_rich_image, [20, 220, 20, 255]),
            (&shared_rich_image, [20, 20, 220, 255]),
            (&emoji_shared_image, [220, 20, 220, 255]),
            (&outside_image, [120, 120, 120, 255]),
        ] {
            image::RgbaImage::from_pixel(1, 1, image::Rgba(color))
                .save(path)
                .unwrap();
        }

        let mut owned = text_item(owned_image.to_string_lossy().as_ref(), 100, "bbbbbbbb");
        owned.content_type = "image".to_owned();
        owned.content_hash = 4_101;
        owned.preview = "owned image".to_owned();

        let mut outside = text_item(outside_image.to_string_lossy().as_ref(), 110, "bbbbbbbb");
        outside.content_type = "image".to_owned();
        outside.content_hash = 4_102;
        outside.preview = "outside image".to_owned();

        let mut emoji_shared = text_item(
            emoji_shared_image.to_string_lossy().as_ref(),
            115,
            "bbbbbbbb",
        );
        emoji_shared.content_type = "image".to_owned();
        emoji_shared.content_hash = 4_103;
        emoji_shared.preview = "emoji shared image".to_owned();

        let mut rich_owned = text_item("rich-owned", 120, "bbbbbbbb");
        rich_owned.content_type = "rich_text".to_owned();
        rich_owned.content_hash = compute_sync_content_hash("rich_text", &rich_owned.content);
        rich_owned.html_content = Some(format!(
            "<img src=\"{}\">",
            owned_rich_image.to_string_lossy()
        ));

        let mut rich_shared_a = text_item("rich-shared-a", 130, "bbbbbbbb");
        rich_shared_a.content_type = "rich_text".to_owned();
        rich_shared_a.content_hash = compute_sync_content_hash("rich_text", &rich_shared_a.content);
        rich_shared_a.html_content = Some(format!(
            "{RICH_IMAGE_FALLBACK_PREFIX}{}{RICH_IMAGE_FALLBACK_SUFFIX}",
            shared_rich_image.to_string_lossy()
        ));
        let mut rich_shared_b = text_item("rich-shared-b", 140, "bbbbbbbb");
        rich_shared_b.content_type = "rich_text".to_owned();
        rich_shared_b.content_hash = compute_sync_content_hash("rich_text", &rich_shared_b.content);
        rich_shared_b.html_content = rich_shared_a.html_content.clone();

        let mut host = database.host();
        let preferences = CloudSyncContentPrefs::default();
        let connection = Connection::open(&database.path).unwrap();
        upsert_setting(
            &connection,
            KEY_EMOJI_FAVORITES,
            &serde_json::to_string(&vec![emoji_shared_image.to_string_lossy()]).unwrap(),
        )
        .unwrap();
        drop(connection);
        assert_eq!(
            host.apply_remote_items(
                &[
                    owned.clone(),
                    outside.clone(),
                    emoji_shared.clone(),
                    rich_owned.clone(),
                    rich_shared_a.clone(),
                    rich_shared_b,
                ],
                &preferences,
            )
            .unwrap(),
            6
        );

        let tombstone = |mut item: CloudSyncItem, revision: i64| {
            item.content.clear();
            item.html_content = None;
            item.deleted_at = revision;
            item.updated_at = revision;
            item.updated_by = "cccccccc".to_owned();
            item
        };
        assert_eq!(
            host.apply_remote_items(
                &[
                    tombstone(owned, 300),
                    tombstone(outside, 310),
                    tombstone(emoji_shared, 315),
                    tombstone(rich_owned, 320),
                    tombstone(rich_shared_a, 330),
                ],
                &preferences,
            )
            .unwrap(),
            5
        );

        assert!(!owned_image.exists());
        assert!(!owned_rich_image.exists());
        assert!(shared_rich_image.exists());
        assert!(emoji_shared_image.exists());
        assert!(outside_image.exists());
        let connection = Connection::open(&database.path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn device_id_reuses_valid_values_and_persists_a_stable_fallback() {
        let database = TestDatabase::new("device-id");
        assert_eq!(
            ensure_cloud_sync_device_id(&database.path).unwrap(),
            "aaaaaaaa"
        );

        let connection = Connection::open(&database.path).unwrap();
        connection
            .execute(
                "UPDATE settings SET value = 'not-a-device-id' WHERE key = 'app.anon_id'",
                [],
            )
            .unwrap();
        drop(connection);

        let generated = ensure_cloud_sync_device_id(&database.path).unwrap();
        assert_eq!(generated.len(), 8);
        assert!(generated
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert_eq!(
            ensure_cloud_sync_device_id(&database.path).unwrap(),
            generated
        );
    }

    #[test]
    fn emoji_images_cross_the_host_boundary_as_data_urls() {
        let source = TestDatabase::new("emoji-source");
        let image_path = source.root.join("favorite.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([20, 120, 220, 255]))
            .save(&image_path)
            .unwrap();
        let connection = Connection::open(&source.path).unwrap();
        upsert_setting(
            &connection,
            KEY_EMOJI_FAVORITES,
            &serde_json::to_string(&vec![image_path.to_string_lossy()]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(
                &setting_value(&connection, KEY_EMOJI_FAVORITES).unwrap()
            )
            .unwrap(),
            vec![image_path.to_string_lossy().into_owned()]
        );
        assert!(image_path_to_data_url(&image_path.to_string_lossy())
            .unwrap()
            .starts_with("data:image/png;base64,"));
        drop(connection);

        let mut source_host = source.host();
        let operation = source_host.next_emoji_operation().unwrap().unwrap();
        assert!(operation.content.contains("data:image/png;base64,"));
        source_host.mark_emoji_uploaded(operation.content_hash);
        assert!(source_host.next_emoji_operation().unwrap().is_none());
        drop(source_host);
        assert!(source.host().next_emoji_operation().unwrap().is_none());

        let destination = TestDatabase::new("emoji-destination");
        let mut destination_host = destination.host();
        assert_eq!(
            destination_host
                .apply_remote_items(&[operation], &CloudSyncContentPrefs::default())
                .unwrap(),
            1
        );
        let connection = Connection::open(&destination.path).unwrap();
        let stored = setting_value(&connection, KEY_EMOJI_FAVORITES).unwrap();
        let paths = serde_json::from_str::<Vec<String>>(&stored).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(Path::new(&paths[0]).is_file());
        assert!(Path::new(&paths[0]).starts_with(destination.root.join("attachments")));
        drop(connection);
        assert!(destination.host().next_emoji_operation().unwrap().is_none());
    }
}
