use crate::database::{
    calc_image_hash, calc_text_hash, has_sensitive_tag, is_text_type, save_image_to_file,
    uses_text_content_hash, ENCRYPT_PREFIX,
};
use crate::domain::models::ClipboardEntry;
use crate::infrastructure::encryption;
use crate::infrastructure::repository::settings_repo::SqliteSettingsRepository;
use rusqlite::params;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tiez_core::database_mutation::{
    delete_record, load_stored_record, save_prepared_record, set_pinned, DeleteRecordPlan,
    PreparedClipboardRecord,
};
use urlencoding::decode;

const RICH_IMAGE_FALLBACK_PREFIX: &str = "<!--TIEZ_RICH_IMAGE:";
const RICH_IMAGE_FALLBACK_SUFFIX: &str = "-->";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub trait ClipboardRepository {
    fn save(
        &self,
        entry: &ClipboardEntry,
        data_dir: Option<&std::path::Path>,
    ) -> Result<i64, String>;
    fn get_history(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String>;
    fn search(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String>;
    fn delete(&self, id: i64, data_dir: Option<&std::path::Path>) -> Result<(), String>;
    fn clear(&self, data_dir: Option<&std::path::Path>) -> Result<(), String>;
    fn get_count(&self) -> Result<i64, String>;
    fn increment_use_count(&self, id: i64) -> Result<(), String>;
    fn touch_entry(&self, id: i64, timestamp: i64) -> Result<(), String>;
    fn toggle_pin(&self, id: i64, is_pinned: bool) -> Result<(), String>;
    fn update_pinned_order(&self, orders: Vec<(i64, i64)>) -> Result<(), String>;
    fn get_entry_by_id(&self, id: i64) -> Result<Option<ClipboardEntry>, String>;
    fn get_entry_by_content(
        &self,
        content: &str,
        content_type: Option<&str>,
    ) -> Result<Option<i64>, String>;
    fn update_entry_content(&self, id: i64, content: &str, preview: &str) -> Result<(), String>;
    fn get_entry_content(&self, id: i64) -> Result<Option<String>, String>;
    fn get_entry_content_full(&self, id: i64) -> Result<Option<(String, String)>, String>;
    fn get_entry_content_with_html(
        &self,
        id: i64,
    ) -> Result<Option<(String, String, Option<String>)>, String>;
}

pub struct SqliteClipboardRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteClipboardRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn encrypt_entry_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        let (content_raw, preview_raw, html_raw, content_type, content_hash, hash_version): (
            String,
            String,
            Option<String>,
            String,
            i64,
            i64,
        ) = conn
            .query_row(
                "SELECT content, preview, html_content, content_type, content_hash,
                        content_hash_version
                 FROM clipboard_history WHERE id = ?",
                params![id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2).ok(),
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        let already_encrypted = content_raw.starts_with(ENCRYPT_PREFIX)
            && preview_raw.starts_with(ENCRYPT_PREFIX)
            && html_raw
                .as_ref()
                .map(|h| h.starts_with(ENCRYPT_PREFIX))
                .unwrap_or(true);
        if already_encrypted {
            return Ok(());
        }

        let content_plain = self
            .try_decrypt_text(&content_raw)
            .ok_or_else(|| "failed to decrypt clipboard content; row was preserved".to_string())?;
        let preview_plain = self
            .try_decrypt_text(&preview_raw)
            .ok_or_else(|| "failed to decrypt clipboard preview; row was preserved".to_string())?;
        let html_plain = html_raw
            .as_deref()
            .map(|html| {
                self.try_decrypt_text(html).ok_or_else(|| {
                    "failed to decrypt clipboard HTML; row was preserved".to_string()
                })
            })
            .transpose()?;

        let content_enc = self.maybe_encrypt_text(&content_plain);
        let preview_enc = self.maybe_encrypt_text(&preview_plain);
        let html_enc = html_plain.as_ref().map(|h| self.maybe_encrypt_text(h));
        let (new_hash, new_hash_version) = if uses_text_content_hash(&content_type) {
            (calc_text_hash(&content_plain) as i64, 2)
        } else {
            (content_hash, hash_version)
        };

        conn.execute(
            "UPDATE clipboard_history
             SET content = ?, preview = ?, html_content = ?, content_hash = ?, content_hash_version = ?
             WHERE id = ?",
            params![
                content_enc,
                preview_enc,
                html_enc,
                new_hash,
                new_hash_version,
                id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn decrypt_entry_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        let (content_raw, preview_raw, html_raw, content_type, content_hash, hash_version): (
            String,
            String,
            Option<String>,
            String,
            i64,
            i64,
        ) = conn
            .query_row(
                "SELECT content, preview, html_content, content_type, content_hash,
                        content_hash_version
                 FROM clipboard_history WHERE id = ?",
                params![id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2).ok(),
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        let any_encrypted = content_raw.starts_with(ENCRYPT_PREFIX)
            || preview_raw.starts_with(ENCRYPT_PREFIX)
            || html_raw
                .as_ref()
                .map(|h| h.starts_with(ENCRYPT_PREFIX))
                .unwrap_or(false);
        if !any_encrypted {
            return Ok(());
        }

        let content_plain = self
            .try_decrypt_text(&content_raw)
            .ok_or_else(|| "failed to decrypt clipboard content; row was preserved".to_string())?;
        let preview_plain = self
            .try_decrypt_text(&preview_raw)
            .ok_or_else(|| "failed to decrypt clipboard preview; row was preserved".to_string())?;
        let html_plain = html_raw
            .as_deref()
            .map(|html| {
                self.try_decrypt_text(html).ok_or_else(|| {
                    "failed to decrypt clipboard HTML; row was preserved".to_string()
                })
            })
            .transpose()?;
        let (new_hash, new_hash_version) = if uses_text_content_hash(&content_type) {
            (calc_text_hash(&content_plain) as i64, 2)
        } else {
            (content_hash, hash_version)
        };

        conn.execute(
            "UPDATE clipboard_history
             SET content = ?, preview = ?, html_content = ?, content_hash = ?, content_hash_version = ?
             WHERE id = ?",
            params![
                content_plain,
                preview_plain,
                html_plain,
                new_hash,
                new_hash_version,
                id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn maybe_encrypt_text(&self, value: &str) -> String {
        #[cfg(not(feature = "portable"))]
        {
            if value.starts_with(ENCRYPT_PREFIX) {
                return value.to_string();
            }
            encryption::encrypt_value(value).unwrap_or_else(|| value.to_string())
        }
        #[cfg(feature = "portable")]
        {
            value.to_string()
        }
    }

    fn try_decrypt_text(&self, value: &str) -> Option<String> {
        if value.starts_with(ENCRYPT_PREFIX) {
            encryption::decrypt_value(value).filter(|plain| plain != value)
        } else {
            Some(value.to_string())
        }
    }

    fn maybe_decrypt_text(&self, value: &str) -> String {
        self.try_decrypt_text(value)
            .unwrap_or_else(|| value.to_string())
    }

    fn extract_rich_image_fallback_payload(html: &str) -> Option<String> {
        if let Some(start) = html.rfind(RICH_IMAGE_FALLBACK_PREFIX) {
            let marker_start = start + RICH_IMAGE_FALLBACK_PREFIX.len();
            if let Some(end_rel) = html[marker_start..].find(RICH_IMAGE_FALLBACK_SUFFIX) {
                let marker_end = marker_start + end_rel;
                let payload = html[marker_start..marker_end].trim();
                if !payload.is_empty() {
                    return Some(payload.to_string());
                }
            }
        }
        None
    }

    fn fallback_payload_to_path(payload: &str) -> Option<PathBuf> {
        let value = payload.trim();
        if value.is_empty() || value.starts_with("data:image/") {
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

        if decoded_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(decoded_path))
        }
    }

    pub(crate) fn collect_attachment_paths_for_cleanup(
        &self,
        content_raw: &str,
        html_raw: Option<&str>,
        is_external: bool,
        attachments_dir: &std::path::Path,
    ) -> Vec<PathBuf> {
        let mut paths = HashSet::new();

        if is_external {
            if let Some(content) = self.try_decrypt_text(content_raw) {
                let content_path = PathBuf::from(content);
                if content_path.starts_with(attachments_dir) {
                    paths.insert(content_path);
                }
            }
        }

        if let Some(html_raw_value) = html_raw {
            if let Some(html) = self.try_decrypt_text(html_raw_value) {
                if let Some(payload) = Self::extract_rich_image_fallback_payload(&html) {
                    if let Some(path) = Self::fallback_payload_to_path(&payload) {
                        if path.starts_with(attachments_dir) {
                            paths.insert(path);
                        }
                    }
                }
            }
        }

        paths.into_iter().collect()
    }

    pub(crate) fn cleanup_unreferenced_attachment_paths_with_conn(
        &self,
        conn: &Connection,
        cleanup_paths: impl IntoIterator<Item = PathBuf>,
        attachments_dir: &std::path::Path,
    ) {
        let cleanup_paths: HashSet<PathBuf> = cleanup_paths
            .into_iter()
            .filter(|path| path.starts_with(attachments_dir))
            .collect();
        if cleanup_paths.is_empty() {
            return;
        }

        let mut stmt = match conn.prepare(
            "SELECT content, html_content, is_external
             FROM clipboard_history",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return,
        };
        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i32>(2)? == 1,
            ))
        }) {
            Ok(rows) => rows,
            Err(_) => return,
        };

        let mut referenced_paths = HashSet::new();
        for row in rows {
            let Ok((content_raw, html_raw, is_external)) = row else {
                return;
            };

            if is_external {
                let Some(content) = self.try_decrypt_text(&content_raw) else {
                    // An unreadable encrypted field may contain any candidate path.
                    // Preserve all files rather than risk deleting a live attachment.
                    return;
                };
                let path = PathBuf::from(content);
                referenced_paths.insert(std::fs::canonicalize(&path).unwrap_or(path));
            }

            if let Some(html_raw) = html_raw {
                let Some(html) = self.try_decrypt_text(&html_raw) else {
                    // Rich-text fallbacks can also be persisted encrypted. As above,
                    // inability to inspect a surviving row must fail closed.
                    return;
                };
                if let Some(payload) = Self::extract_rich_image_fallback_payload(&html) {
                    if let Some(path) = Self::fallback_payload_to_path(&payload) {
                        referenced_paths.insert(std::fs::canonicalize(&path).unwrap_or(path));
                    }
                }
            }
        }

        for path in cleanup_paths {
            let path_identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !referenced_paths.contains(&path_identity) && path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    pub fn save_with_conn(
        &self,
        conn: &Connection,
        entry: &ClipboardEntry,
        data_dir: Option<&std::path::Path>,
    ) -> Result<i64, String> {
        // Encrypt only when explicitly marked as sensitive
        let should_encrypt = has_sensitive_tag(&entry.tags);

        let mut final_content = entry.content.clone();
        let mut final_is_external = entry.is_external;

        // Externalize image if possible
        if entry.content_type == "image" && entry.content.starts_with("data:image/") {
            if let Some(dir) = data_dir {
                if let Some(path) = save_image_to_file(&entry.content, dir) {
                    final_content = path;
                    final_is_external = true;
                }
            }
        }

        let calculated_hash = if entry.content_type == "image" {
            if entry.content.starts_with("data:") {
                calc_image_hash(&entry.content).unwrap_or(0)
            } else {
                if let Ok(img) = image::open(&entry.content) {
                    let thumb = img.resize_exact(32, 32, image::imageops::FilterType::Nearest);
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    thumb.as_bytes().hash(&mut hasher);
                    hasher.finish() as i64
                } else {
                    0
                }
            }
        } else {
            calc_text_hash(&final_content) as i64
        };

        let (content, preview, content_hash, html_content) = if should_encrypt {
            let encrypted_content = self.maybe_encrypt_text(&final_content);
            let encrypted_preview = self.maybe_encrypt_text(&entry.preview);
            let encrypted_html = entry
                .html_content
                .as_ref()
                .map(|html| self.maybe_encrypt_text(html));
            (
                encrypted_content,
                encrypted_preview,
                calculated_hash,
                encrypted_html,
            )
        } else {
            (
                final_content.clone(),
                entry.preview.clone(),
                calculated_hash,
                entry.html_content.clone(),
            )
        };

        save_prepared_record(
            conn,
            &PreparedClipboardRecord {
                id: entry.id,
                content_type: &entry.content_type,
                content: &content,
                identity_content: &final_content,
                html_content: html_content.as_deref(),
                source_app: &entry.source_app,
                source_app_path: entry.source_app_path.as_deref(),
                timestamp: entry.timestamp,
                preview: &preview,
                is_pinned: entry.is_pinned,
                content_hash,
                tags: &entry.tags,
                is_external: final_is_external,
                pinned_order: entry.pinned_order,
            },
        )
    }

    pub fn delete_with_conn(
        &self,
        conn: &Connection,
        id: i64,
        data_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let entry = load_stored_record(conn, id)?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.to_string())?;

        let cleanup_paths = data_dir
            .map(|dir| {
                self.collect_attachment_paths_for_cleanup(
                    &entry.content,
                    entry.html_content.as_deref(),
                    entry.is_external,
                    &dir.join("attachments"),
                )
            })
            .unwrap_or_default();
        let may_cleanup_after_savepoint = conn.is_autocommit();

        let readable_content = self.try_decrypt_text(&entry.content);
        let (tombstone_hash, tombstone_hash_version) =
            if uses_text_content_hash(&entry.content_type) {
                if let Some(content_plain) = readable_content.as_deref() {
                    (calc_text_hash(content_plain) as i64, 2)
                } else {
                    (entry.content_hash, entry.content_hash_version)
                }
            } else {
                (entry.content_hash, entry.content_hash_version)
            };

        // Tombstone, history, and normalized tags are one database unit. If
        // legacy ciphertext cannot be decrypted, retain its stored hash
        // semantics rather than manufacturing a v2 hash from ciphertext bytes.
        if !delete_record(
            conn,
            DeleteRecordPlan {
                id,
                content_type: &entry.content_type,
                content_hash: tombstone_hash,
                content_hash_version: tombstone_hash_version,
                deleted_at: now_ms(),
            },
        )? {
            return Err(rusqlite::Error::QueryReturnedNoRows.to_string());
        }

        // Releasing the outermost savepoint commits the database operation. When
        // called inside an owner transaction (the global-tag path), that owner must
        // keep cleanup deferred until its own commit and therefore passes no data dir.
        if may_cleanup_after_savepoint {
            if let Some(dir) = data_dir {
                self.cleanup_unreferenced_attachment_paths_with_conn(
                    conn,
                    cleanup_paths,
                    &dir.join("attachments"),
                );
            }
        }
        Ok(())
    }

    pub fn delete_metadata_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        if conn.is_autocommit() {
            return Err(
                "metadata deletion requires an active owner transaction or savepoint".to_string(),
            );
        }
        conn.execute("DELETE FROM clipboard_history WHERE id = ?", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM entry_tags WHERE entry_id = ?", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn find_by_content_with_conn(
        &self,
        conn: &Connection,
        content: &str,
        content_type: Option<&str>,
    ) -> Result<Option<i64>, String> {
        if content_type == Some("image") {
            if let Some(hash) = calc_image_hash(content) {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM clipboard_history \
                     WHERE (content_type = 'image' AND content_hash = ?) OR content = ?",
                    )
                    .map_err(|e| e.to_string())?;
                let mut rows = stmt
                    .query(params![hash, content])
                    .map_err(|e| e.to_string())?;
                if let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    return Ok(Some(row.get(0).map_err(|e| e.to_string())?));
                }
                return Ok(None);
            }
        }

        let hash = calc_text_hash(content) as i64;

        if let Some(ct) = content_type {
            let mut stmt = conn.prepare(
                "SELECT id FROM clipboard_history \
                 WHERE (content_type = ? AND content_hash = ?) OR (content_type = ? AND content = ?)",
            ).map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(params![ct, hash, ct, content])
                .map_err(|e| e.to_string())?;
            if let Some(row) = rows.next().map_err(|e| e.to_string())? {
                Ok(Some(row.get(0).map_err(|e| e.to_string())?))
            } else {
                Ok(None)
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id FROM clipboard_history \
                 WHERE ((content_type IN ('text', 'rich_text', 'code', 'url')) AND content_hash = ?) OR content = ?",
            ).map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(params![hash, content])
                .map_err(|e| e.to_string())?;
            if let Some(row) = rows.next().map_err(|e| e.to_string())? {
                Ok(Some(row.get(0).map_err(|e| e.to_string())?))
            } else {
                Ok(None)
            }
        }
    }

    pub fn enforce_limit_with_conn(
        &self,
        conn: &Connection,
        data_dir: Option<&std::path::Path>,
    ) -> Result<Vec<i64>, String> {
        // Check if storage limit is enabled
        if let Ok(Some(limit_enabled_str)) =
            SqliteSettingsRepository::get_raw(conn, "app.persistent_limit_enabled")
        {
            if limit_enabled_str == "false" {
                return Ok(Vec::new());
            }
        }

        // Get the storage limit
        if let Ok(Some(limit_str)) = SqliteSettingsRepository::get_raw(conn, "app.persistent_limit")
        {
            if let Ok(limit) = limit_str.parse::<i32>() {
                // Count non-pinned entries
                let count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM clipboard_history WHERE is_pinned = 0 AND (tags = '[]' OR tags IS NULL)",
                    [],
                    |row| row.get(0)
                ).map_err(|e| e.to_string())?;

                if count > limit {
                    // First, get the IDs that will be deleted
                    let to_delete = count - limit;
                    let deleted_ids: Vec<i64> = {
                        let mut stmt = conn
                            .prepare(
                                "SELECT id FROM clipboard_history 
                             WHERE is_pinned = 0 AND (tags = '[]' OR tags IS NULL)
                             ORDER BY timestamp ASC 
                             LIMIT ?",
                            )
                            .map_err(|e| e.to_string())?;

                        let rows = stmt
                            .query_map([to_delete], |row| row.get(0))
                            .map_err(|e| e.to_string())?;
                        rows.filter_map(|r| r.ok()).collect()
                    };
                    // Actually delete records (and files if needed)
                    for id in &deleted_ids {
                        self.delete_with_conn(conn, *id, data_dir)?;
                    }
                    return Ok(deleted_ids);
                }
            }
        }

        Ok(Vec::new())
    }
    pub fn toggle_pin_with_conn(
        &self,
        conn: &Connection,
        id: i64,
        is_pinned: bool,
    ) -> Result<(), String> {
        let _ = set_pinned(conn, id, is_pinned, now_ms())?;
        Ok(())
    }

    pub fn update_pinned_order_with_conn(
        &self,
        conn: &Connection,
        orders: Vec<(i64, i64)>,
    ) -> Result<(), String> {
        let updated_at = now_ms();
        for (id, order) in orders {
            conn.execute(
                "UPDATE clipboard_history
                 SET pinned_order = ?1,
                     sync_updated_at = ?3,
                     sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                 WHERE id = ?2",
                params![order, id, updated_at],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn get_entry_by_id_with_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<ClipboardEntry>, String> {
        let mut stmt = conn.prepare(
            "SELECT id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, tags, use_count, is_external, pinned_order, source_app_path 
             FROM clipboard_history 
             WHERE id = ? 
             LIMIT 1",
        ).map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let tags_str: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
            let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();

            let content_raw: String = row.get(2).map_err(|e| e.to_string())?;
            let html_raw: Option<String> = row.get(3).map_err(|e| e.to_string()).unwrap_or(None);
            let preview_raw: String = row.get(6).map_err(|e| e.to_string())?;
            let content = self.maybe_decrypt_text(&content_raw);
            let preview = self.maybe_decrypt_text(&preview_raw);
            let html_content = html_raw.map(|v| self.maybe_decrypt_text(&v));

            Ok(Some(ClipboardEntry {
                id: row.get(0).map_err(|e| e.to_string())?,
                content_type: row.get(1).map_err(|e| e.to_string())?,
                content,
                html_content,
                source_app: row.get(4).map_err(|e| e.to_string())?,
                timestamp: row.get(5).map_err(|e| e.to_string())?,
                preview,
                is_pinned: row.get::<_, i32>(7).map_err(|e| e.to_string())? == 1,
                tags,
                use_count: row.get(9).unwrap_or(0),
                is_external: row.get::<_, i32>(10).unwrap_or(0) == 1,
                pinned_order: row.get(11).unwrap_or(0),
                source_app_path: row.get(12).unwrap_or(None),
                file_preview_exists: true,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn update_entry_content_with_conn(
        &self,
        conn: &Connection,
        id: i64,
        content: &str,
        preview: &str,
    ) -> Result<(), String> {
        let (old_content_raw, content_type, tags_json, has_html) = conn
            .query_row(
                "SELECT content, content_type, tags, (html_content IS NOT NULL) FROM clipboard_history WHERE id = ?",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        let old_content = self.maybe_decrypt_text(&old_content_raw);
        // Procceed if content changed, OR if content is same but we need to transition away from rich text/clear HTML
        if old_content == content && content_type != "rich_text" && !has_html {
            return Ok(());
        }

        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let should_encrypt = has_sensitive_tag(&tags);
        let sync_updated_at = now_ms();

        if is_text_type(&content_type) {
            let hash = calc_text_hash(content) as i64;
            let new_type = if content_type == "rich_text" {
                "text"
            } else {
                &content_type
            };
            if should_encrypt {
                let encrypted_content = self.maybe_encrypt_text(content);
                let encrypted_preview = self.maybe_encrypt_text(preview);
                conn.execute(
                    "UPDATE clipboard_history
                     SET content = ?1,
                         preview = ?2,
                         content_hash = ?3,
                         content_hash_version = 2,
                         html_content = NULL,
                         content_type = ?4,
                         sync_updated_at = ?5,
                         sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                     WHERE id = ?6",
                    params![encrypted_content, encrypted_preview, hash, new_type, sync_updated_at, id],
                ).map_err(|e| e.to_string())?;
            } else {
                conn.execute(
                    "UPDATE clipboard_history
                     SET content = ?1,
                         preview = ?2,
                         content_hash = ?3,
                         content_hash_version = 2,
                         html_content = NULL,
                         content_type = ?4,
                         sync_updated_at = ?5,
                         sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                     WHERE id = ?6",
                    params![content, preview, hash, new_type, sync_updated_at, id],
                ).map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        if should_encrypt {
            let encrypted_content = self.maybe_encrypt_text(content);
            let encrypted_preview = self.maybe_encrypt_text(preview);
            conn.execute(
                "UPDATE clipboard_history
                 SET content = ?1,
                     preview = ?2,
                     html_content = NULL,
                     sync_updated_at = ?3,
                     sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                 WHERE id = ?4",
                params![encrypted_content, encrypted_preview, sync_updated_at, id],
            ).map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE clipboard_history
                 SET content = ?1,
                     preview = ?2,
                     html_content = NULL,
                     sync_updated_at = ?3,
                     sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                 WHERE id = ?4",
                params![content, preview, sync_updated_at, id],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn get_entry_content_full_with_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<(String, String)>, String> {
        let mut stmt = conn
            .prepare("SELECT content, content_type FROM clipboard_history WHERE id = ?")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let content: String = row.get(0).map_err(|e| e.to_string())?;
            let content_type: String = row.get(1).map_err(|e| e.to_string())?;
            Ok(Some((self.maybe_decrypt_text(&content), content_type)))
        } else {
            Ok(None)
        }
    }

    pub fn get_entry_content_with_html_with_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<(String, String, Option<String>)>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT content, content_type, html_content FROM clipboard_history WHERE id = ?",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let content: String = row.get(0).map_err(|e| e.to_string())?;
            let content_type: String = row.get(1).map_err(|e| e.to_string())?;
            let html_raw: Option<String> = row.get(2).map_err(|e| e.to_string()).unwrap_or(None);
            let html_content = html_raw.map(|v| self.maybe_decrypt_text(&v));
            Ok(Some((
                self.maybe_decrypt_text(&content),
                content_type,
                html_content,
            )))
        } else {
            Ok(None)
        }
    }
}

impl ClipboardRepository for SqliteClipboardRepository {
    fn save(
        &self,
        entry: &ClipboardEntry,
        data_dir: Option<&std::path::Path>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.save_with_conn(&conn, entry, data_dir)
    }

    fn get_history(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let map_row = |row: &rusqlite::Row| {
            let tags_str: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
            let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
            let content_type: String = row.get(1)?;
            let content_raw: String = row.get(2)?;
            let html_raw: Option<String> = row.get(3).ok();
            let preview_raw: String = row.get(6)?;
            let content = self.maybe_decrypt_text(&content_raw);
            let preview = self.maybe_decrypt_text(&preview_raw);
            let html_content = html_raw.as_ref().map(|v| self.maybe_decrypt_text(v));

            Ok((
                ClipboardEntry {
                    id: row.get(0)?,
                    content_type,
                    content,
                    html_content,
                    source_app: row.get(4)?,
                    timestamp: row.get(5)?,
                    preview,
                    is_pinned: row.get::<_, i32>(7)? == 1,
                    tags,
                    use_count: row.get(9).unwrap_or(0),
                    is_external: row.get::<_, i32>(10)? == 1,
                    pinned_order: row.get(11).unwrap_or(0),
                    source_app_path: row.get(12).unwrap_or(None),
                    // Avoid synchronous filesystem existence checks in history query.
                    // Missing files are still handled by frontend image/file preview error fallback.
                    file_preview_exists: true,
                },
                content_raw,
                preview_raw,
                html_raw,
            ))
        };

        let mut mapped_rows = Vec::new();
        if let Some(ct) = content_type {
            let mut stmt = conn.prepare(
                "SELECT id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, tags, use_count, is_external, pinned_order, source_app_path 
                 FROM clipboard_history 
                 WHERE content_type = ? 
                 ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC 
                 LIMIT ? OFFSET ?",
            ).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![ct, limit, offset], map_row)
                .map_err(|e| e.to_string())?;
            for row in rows {
                mapped_rows.push(row.map_err(|e| e.to_string())?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, content_type, content, html_content, source_app, timestamp, preview, is_pinned, tags, use_count, is_external, pinned_order, source_app_path 
                 FROM clipboard_history 
                 ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC 
                 LIMIT ? OFFSET ?",
            ).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([limit, offset], map_row)
                .map_err(|e| e.to_string())?;
            for row in rows {
                mapped_rows.push(row.map_err(|e| e.to_string())?);
            }
        }

        let mut history = Vec::new();
        for (entry, content_raw, preview_raw, html_raw) in mapped_rows {
            #[cfg(not(feature = "portable"))]
            {
                let is_sensitive = has_sensitive_tag(&entry.tags);
                let content_encrypted = content_raw.starts_with(ENCRYPT_PREFIX);
                let preview_encrypted = preview_raw.starts_with(ENCRYPT_PREFIX);
                let html_encrypted = html_raw
                    .as_ref()
                    .map(|h| h.starts_with(ENCRYPT_PREFIX))
                    .unwrap_or(false);
                let html_needs_encrypt = html_raw
                    .as_ref()
                    .map(|h| !h.starts_with(ENCRYPT_PREFIX))
                    .unwrap_or(false);

                if is_sensitive && (!content_encrypted || !preview_encrypted || html_needs_encrypt)
                {
                    let _ = self.encrypt_entry_with_conn(&conn, entry.id);
                } else if !is_sensitive
                    && (content_encrypted || preview_encrypted || html_encrypted)
                {
                    let _ = self.decrypt_entry_with_conn(&conn, entry.id);
                }
            }

            history.push(entry);
        }
        Ok(history)
    }

    fn search(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let term = query.trim().to_lowercase();
        if term.is_empty() {
            return Ok(Vec::new());
        }

        #[cfg(feature = "portable")]
        {
            // Portable version: Data is NOT encrypted, use conventional SQL LIKE search (fastest)
            let sql = if tag_only {
                "SELECT ch.id, ch.content_type, ch.content, ch.html_content, ch.source_app, ch.timestamp, ch.preview, ch.is_pinned, ch.tags, ch.use_count, ch.is_external, ch.pinned_order, ch.source_app_path
                 FROM clipboard_history ch
                 WHERE EXISTS (
                     SELECT 1 FROM entry_tags et
                     WHERE et.entry_id = ch.id
                       AND et.tag LIKE '%' || ?1 || '%'
                 )
                 ORDER BY ch.timestamp DESC
                 LIMIT ?2"
            } else {
                "SELECT ch.id, ch.content_type, ch.content, ch.html_content, ch.source_app, ch.timestamp, ch.preview, ch.is_pinned, ch.tags, ch.use_count, ch.is_external, ch.pinned_order, ch.source_app_path
                 FROM clipboard_history ch
                 WHERE ch.content LIKE '%' || ?1 || '%'
                    OR ch.source_app LIKE '%' || ?1 || '%'
                    OR EXISTS (
                        SELECT 1 FROM entry_tags et
                        WHERE et.entry_id = ch.id
                          AND et.tag LIKE '%' || ?1 || '%'
                    )
                    OR EXISTS (
                        SELECT 1 FROM clipboard_image_analysis ia
                        WHERE ia.entry_id = ch.id
                          AND ia.content_hash = ch.content_hash
                          AND (
                              ia.ocr_text LIKE '%' || ?1 || '%'
                              OR ia.qr_codes LIKE '%' || ?1 || '%'
                          )
                    )
                 ORDER BY ch.timestamp DESC
                 LIMIT ?2"
            };

            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

            let rows = stmt
                .query_map(params![term, limit], |row| {
                    let tags_str: String =
                        row.get::<_, String>(8).unwrap_or_else(|_| "[]".to_string());
                    Ok(ClipboardEntry {
                        id: row.get(0)?,
                        content_type: row.get(1)?,
                        content: row.get(2)?,
                        html_content: row.get(3).ok(),
                        source_app: row.get(4)?,
                        timestamp: row.get(5)?,
                        preview: row.get(6)?,
                        is_pinned: row.get::<_, i32>(7)? == 1,
                        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                        use_count: row.get(9).unwrap_or(0),
                        is_external: row.get::<_, i32>(10)? == 1,
                        pinned_order: row.get(11).unwrap_or(0),
                        source_app_path: row.get(12).unwrap_or(None),
                        file_preview_exists: true, // Simplified for search
                    })
                })
                .map_err(|e| e.to_string())?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row.map_err(|e| e.to_string())?);
            }
            Ok(results)
        }

        #[cfg(not(feature = "portable"))]
        {
            let mut results: Vec<ClipboardEntry> = Vec::new();
            let mut seen: HashSet<i64> = HashSet::new();

            let sensitive_tags_sql = {
                let tags = crate::database::SENSITIVE_TAGS;
                let parts: Vec<String> = tags
                    .iter()
                    .map(|t| format!("'{}'", t.replace('\'', "''")))
                    .collect();
                format!("({})", parts.join(","))
            };

            // 1) SQL search for non-sensitive (plaintext) entries
            let sql_non_sensitive = if tag_only {
                format!(
                    "SELECT ch.id, ch.content_type, ch.content, ch.html_content, ch.source_app, ch.timestamp, ch.preview, ch.is_pinned, ch.tags, ch.use_count, ch.is_external, ch.pinned_order, ch.source_app_path
                     FROM clipboard_history ch
                     WHERE NOT EXISTS (
                         SELECT 1 FROM entry_tags se
                         WHERE se.entry_id = ch.id
                           AND se.tag COLLATE NOCASE IN {}
                     )
                       AND EXISTS (
                           SELECT 1 FROM entry_tags et
                           WHERE et.entry_id = ch.id
                             AND et.tag LIKE '%' || ?1 || '%'
                       )
                     ORDER BY ch.timestamp DESC, ch.id DESC
                     LIMIT ?2",
                    sensitive_tags_sql
                )
            } else {
                format!(
                    "SELECT ch.id, ch.content_type, ch.content, ch.html_content, ch.source_app, ch.timestamp, ch.preview, ch.is_pinned, ch.tags, ch.use_count, ch.is_external, ch.pinned_order, ch.source_app_path
                     FROM clipboard_history ch
                     WHERE NOT EXISTS (
                         SELECT 1 FROM entry_tags se
                         WHERE se.entry_id = ch.id
                           AND se.tag COLLATE NOCASE IN {}
                     )
                       AND (
                         ch.content LIKE '%' || ?1 || '%'
                         OR ch.source_app LIKE '%' || ?1 || '%'
                         OR EXISTS (
                             SELECT 1 FROM entry_tags et
                             WHERE et.entry_id = ch.id
                               AND et.tag LIKE '%' || ?1 || '%'
                         )
                         OR EXISTS (
                             SELECT 1 FROM clipboard_image_analysis ia
                             WHERE ia.entry_id = ch.id
                               AND ia.content_hash = ch.content_hash
                               AND (
                                   ia.ocr_text LIKE '%' || ?1 || '%'
                                   OR ia.qr_codes LIKE '%' || ?1 || '%'
                               )
                         )
                       )
                     ORDER BY ch.timestamp DESC, ch.id DESC
                     LIMIT ?2",
                    sensitive_tags_sql
                )
            };

            let mut stmt = conn
                .prepare(&sql_non_sensitive)
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![term, limit], |row| {
                    let tags_str: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
                    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
                    let content_raw: String = row.get(2)?;
                    let preview_raw: String = row.get(6)?;
                    let html_raw: Option<String> = row.get(3).ok();
                    let content = self.maybe_decrypt_text(&content_raw);
                    let preview = self.maybe_decrypt_text(&preview_raw);
                    let html_content = html_raw.map(|v| self.maybe_decrypt_text(&v));

                    Ok(ClipboardEntry {
                        id: row.get(0)?,
                        content_type: row.get(1)?,
                        content,
                        html_content,
                        source_app: row.get(4)?,
                        timestamp: row.get(5)?,
                        preview,
                        is_pinned: row.get::<_, i32>(7)? == 1,
                        tags,
                        use_count: row.get(9).unwrap_or(0),
                        is_external: row.get::<_, i32>(10)? == 1,
                        pinned_order: row.get(11).unwrap_or(0),
                        source_app_path: row.get(12).unwrap_or(None),
                        file_preview_exists: true,
                    })
                })
                .map_err(|e| e.to_string())?;

            for row in rows {
                if let Ok(entry) = row {
                    if seen.insert(entry.id) {
                        results.push(entry);
                    }
                }
            }

            // 2) Decrypt-scan sensitive or encrypted entries (only if needed)
            if results.len() < limit as usize {
                let mut cursor_ts = i64::MAX;
                let mut cursor_id = i64::MAX;
                let batch_size = 500;
                let enc_like = format!("{}%", ENCRYPT_PREFIX);
                let sql_sensitive = format!(
                    "SELECT ch.id, ch.content_type, ch.content, ch.html_content, ch.source_app, ch.timestamp, ch.preview, ch.is_pinned, ch.tags, ch.use_count, ch.is_external, ch.pinned_order, ch.source_app_path 
                     FROM clipboard_history ch
                     WHERE (
                         EXISTS (
                             SELECT 1 FROM entry_tags se 
                             WHERE se.entry_id = ch.id 
                               AND se.tag COLLATE NOCASE IN {}
                         )
                         OR ch.content LIKE ?1 
                         OR ch.preview LIKE ?1 
                         OR ch.html_content LIKE ?1
                     )
                       AND ((ch.timestamp < ?2) OR (ch.timestamp = ?2 AND ch.id < ?3))
                     ORDER BY ch.timestamp DESC, ch.id DESC
                     LIMIT ?4",
                    sensitive_tags_sql
                );

                loop {
                    let mut stmt = conn.prepare(&sql_sensitive).map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map(params![enc_like, cursor_ts, cursor_id, batch_size], |row| {
                            let tags_str: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
                            Ok(ClipboardEntry {
                                id: row.get(0)?,
                                content_type: row.get(1)?,
                                content: row.get(2)?, // Encrypted
                                html_content: row.get(3).ok(),
                                source_app: row.get(4)?,
                                timestamp: row.get(5)?,
                                preview: row.get(6)?, // Encrypted
                                is_pinned: row.get::<_, i32>(7)? == 1,
                                tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                                use_count: row.get(9).unwrap_or(0),
                                is_external: row.get::<_, i32>(10)? == 1,
                                pinned_order: row.get(11).unwrap_or(0),
                                source_app_path: row.get(12).unwrap_or(None),
                                file_preview_exists: true,
                            })
                        })
                        .map_err(|e| e.to_string())?;

                    let mut batch: Vec<ClipboardEntry> = Vec::new();
                    for row in rows {
                        if let Ok(mut entry) = row {
                            entry.content = self.maybe_decrypt_text(&entry.content);
                            entry.preview = self.maybe_decrypt_text(&entry.preview);
                            if let Some(html) = entry.html_content.take() {
                                entry.html_content = Some(self.maybe_decrypt_text(&html));
                            }
                            batch.push(entry);
                        }
                    }

                    if batch.is_empty() {
                        break;
                    }

                    for entry in batch.iter() {
                        let matches = if tag_only {
                            entry.tags.iter().any(|t| t.to_lowercase().contains(&term))
                        } else {
                            entry.content.to_lowercase().contains(&term)
                                || entry.source_app.to_lowercase().contains(&term)
                                || entry.tags.iter().any(|t| t.to_lowercase().contains(&term))
                        };

                        if matches && seen.insert(entry.id) {
                            results.push(entry.clone());
                            if results.len() >= limit as usize {
                                break;
                            }
                        }
                    }

                    if results.len() >= limit as usize {
                        break;
                    }

                    if let Some(last) = batch.last() {
                        cursor_ts = last.timestamp;
                        cursor_id = last.id;
                    } else {
                        break;
                    }
                }
            }

            results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(b.id.cmp(&a.id)));
            if results.len() > limit as usize {
                results.truncate(limit as usize);
            }
            Ok(results)
        }
    }

    fn delete(&self, id: i64, data_dir: Option<&std::path::Path>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.delete_with_conn(&conn, id, data_dir)
    }

    fn clear(&self, data_dir: Option<&std::path::Path>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // Get IDs of unpinned items without tags.
        let mut stmt = conn
            .prepare(
                "SELECT id FROM clipboard_history 
             WHERE is_pinned = 0 
               AND NOT EXISTS (SELECT 1 FROM entry_tags WHERE entry_id = clipboard_history.id)",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let ids: Vec<i64> = rows.filter_map(Result::ok).collect();

        // Delete one-by-one so tombstones are recorded for cloud deletion sync.
        for id in &ids {
            self.delete_with_conn(&conn, *id, data_dir)?;
        }

        // VACUUM to reclaim space
        let _ = conn.execute_batch("VACUUM;");
        Ok(())
    }

    fn get_count(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM clipboard_history")
            .map_err(|e| e.to_string())?;
        let count: i64 = stmt
            .query_row([], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    fn increment_use_count(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE clipboard_history SET use_count = use_count + 1 WHERE id = ?",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn touch_entry(&self, id: i64, timestamp: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE clipboard_history
             SET timestamp = ?1,
                 sync_updated_at = ?1,
                 sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
             WHERE id = ?2",
            params![timestamp, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn toggle_pin(&self, id: i64, is_pinned: bool) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.toggle_pin_with_conn(&conn, id, is_pinned)
    }

    fn update_pinned_order(&self, orders: Vec<(i64, i64)>) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        self.update_pinned_order_with_conn(&tx, orders)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_entry_by_id(&self, id: i64) -> Result<Option<ClipboardEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_entry_by_id_with_conn(&conn, id)
    }

    fn get_entry_by_content(
        &self,
        content: &str,
        content_type: Option<&str>,
    ) -> Result<Option<i64>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.find_by_content_with_conn(&conn, content, content_type)
    }

    fn update_entry_content(&self, id: i64, content: &str, preview: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.update_entry_content_with_conn(&conn, id, content, preview)
    }

    fn get_entry_content(&self, id: i64) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT content FROM clipboard_history WHERE id = ?")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let content: String = row.get(0).map_err(|e| e.to_string())?;
            Ok(Some(self.maybe_decrypt_text(&content)))
        } else {
            Ok(None)
        }
    }

    fn get_entry_content_full(&self, id: i64) -> Result<Option<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_entry_content_full_with_conn(&conn, id)
    }

    fn get_entry_content_with_html(
        &self,
        id: i64,
    ) -> Result<Option<(String, String, Option<String>)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_entry_content_with_html_with_conn(&conn, id)
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardRepository, SqliteClipboardRepository};
    use crate::domain::models::ClipboardEntry;
    use crate::infrastructure::repository::migrations::run_migrations;
    use rusqlite::{params, Connection};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn search_matches_current_ocr_and_qr_index() {
        let conn = Connection::open_in_memory().expect("open repository test db");
        run_migrations(&conn).expect("run migrations");
        conn.execute(
            "INSERT INTO clipboard_history
                (id, content_type, content, source_app, timestamp, preview, content_hash)
             VALUES (1, 'image', '/tmp/image.png', 'test', 100, 'image', 42)",
            [],
        )
        .expect("insert image entry");
        conn.execute(
            "INSERT INTO clipboard_image_analysis
                (entry_id, content_hash, ocr_text, qr_codes, analyzed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                1,
                42,
                "invoice number 8675",
                "[\"https://tiez.app/qr\"]",
                100
            ],
        )
        .expect("insert image analysis");

        let repo = SqliteClipboardRepository::new(Arc::new(Mutex::new(conn)));
        assert_eq!(repo.search("8675", 10, false).expect("search OCR").len(), 1);
        assert_eq!(
            repo.search("tiez.app", 10, false).expect("search QR").len(),
            1
        );

        {
            let conn = repo.conn.lock().expect("lock repository db");
            conn.execute(
                "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'invoice')",
                [],
            )
            .expect("insert first matching tag");
            conn.execute(
                "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'invoice-archive')",
                [],
            )
            .expect("insert second matching tag");
        }
        assert_eq!(
            repo.search("invoice", 10, true)
                .expect("search multiple matching tags")
                .len(),
            1,
            "one entry must not be duplicated by multiple matching tags"
        );

        let conn = repo.conn.lock().expect("lock repository db");
        conn.execute(
            "UPDATE clipboard_history SET content_hash = 99 WHERE id = 1",
            [],
        )
        .expect("change image hash");
        drop(conn);
        assert!(repo
            .search("8675", 10, false)
            .expect("search stale OCR")
            .is_empty());
    }

    fn setup_repository_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().expect("open clipboard repository test db");
        run_migrations(&conn).expect("migrate clipboard repository test db");
        Arc::new(Mutex::new(conn))
    }

    fn test_entry(tags: &[&str]) -> ClipboardEntry {
        ClipboardEntry {
            id: 0,
            content_type: "text".to_string(),
            content: "savepoint payload".to_string(),
            html_content: None,
            source_app: "Test".to_string(),
            source_app_path: None,
            timestamp: 100,
            preview: "savepoint payload".to_string(),
            is_pinned: false,
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: true,
        }
    }

    fn make_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tiez-clipboard-repo-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("create clipboard repository temp dir");
        dir
    }

    #[test]
    fn save_rolls_back_cleared_tombstone_and_history_when_tag_insert_fails() {
        let conn = setup_repository_db();
        let entry = test_entry(&["blocked"]);
        let hash = crate::database::calc_text_hash(&entry.content) as i64;
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            guard
                .execute(
                    "INSERT INTO cloud_sync_tombstones
                        (content_type, content_hash, hash_version, deleted_at)
                     VALUES ('text', ?1, 2, 50)",
                    [hash],
                )
                .expect("insert matching tombstone");
            guard
                .execute_batch(
                    "CREATE TRIGGER reject_entry_tag_insert
                     BEFORE INSERT ON entry_tags
                     BEGIN SELECT RAISE(ABORT, 'entry tag unavailable'); END;",
                )
                .expect("install entry-tag failure trigger");
        }

        let error = SqliteClipboardRepository::new(conn.clone())
            .save(&entry, None)
            .expect_err("tag persistence must fail the save");
        assert!(error.contains("entry tag unavailable"));

        let guard = conn.lock().expect("lock failed-save result db");
        let history_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })
            .expect("count rolled-back history");
        let tombstone_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM cloud_sync_tombstones
                 WHERE content_type = 'text' AND content_hash = ?1 AND hash_version = 2",
                [hash],
                |row| row.get(0),
            )
            .expect("count restored tombstone");
        let tag_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM entry_tags", [], |row| row.get(0))
            .expect("count rolled-back tags");
        assert_eq!(history_count, 0);
        assert_eq!(tombstone_count, 1);
        assert_eq!(tag_count, 0);
    }

    #[test]
    fn save_propagates_tombstone_clear_failure_without_writing_history() {
        let conn = setup_repository_db();
        let entry = test_entry(&[]);
        let hash = crate::database::calc_text_hash(&entry.content) as i64;
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            guard
                .execute(
                    "INSERT INTO cloud_sync_tombstones
                        (content_type, content_hash, hash_version, deleted_at)
                     VALUES ('text', ?1, 2, 50)",
                    [hash],
                )
                .expect("insert matching tombstone");
            guard
                .execute_batch(
                    "CREATE TRIGGER reject_tombstone_clear
                     BEFORE DELETE ON cloud_sync_tombstones
                     BEGIN SELECT RAISE(ABORT, 'tombstone unavailable'); END;",
                )
                .expect("install tombstone failure trigger");
        }

        let error = SqliteClipboardRepository::new(conn.clone())
            .save(&entry, None)
            .expect_err("tombstone failure must fail the save");
        assert!(error.contains("tombstone unavailable"));

        let guard = conn.lock().expect("lock failed-save result db");
        let history_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })
            .expect("count absent history");
        let tombstone_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count retained tombstone");
        assert_eq!(history_count, 0);
        assert_eq!(tombstone_count, 1);
    }

    #[test]
    fn successful_save_clears_matching_tombstone_and_persists_clean_tags() {
        let conn = setup_repository_db();
        let entry = test_entry(&[" work ", "work", ""]);
        let hash = crate::database::calc_text_hash(&entry.content) as i64;
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            guard
                .execute(
                    "INSERT INTO cloud_sync_tombstones
                        (content_type, content_hash, hash_version, deleted_at)
                     VALUES ('text', ?1, 2, 50)",
                    [hash],
                )
                .expect("insert matching tombstone");
        }

        let id = SqliteClipboardRepository::new(conn.clone())
            .save(&entry, None)
            .expect("save entry atomically");

        let guard = conn.lock().expect("lock successful-save result db");
        let tags_json: String = guard
            .query_row(
                "SELECT tags FROM clipboard_history WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("read saved tag JSON");
        let normalized_tags: Vec<String> = guard
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = ?1 ORDER BY tag")
            .expect("prepare normalized tag query")
            .query_map([id], |row| row.get(0))
            .expect("query normalized tags")
            .collect::<Result<_, _>>()
            .expect("read normalized tags");
        let tombstone_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count cleared tombstones");
        assert_eq!(tags_json, "[\"work\"]");
        assert_eq!(normalized_tags, vec!["work"]);
        assert_eq!(tombstone_count, 0);
    }

    #[test]
    fn save_savepoint_nests_without_committing_owner_transaction() {
        let conn = setup_repository_db();
        let entry = test_entry(&["nested"]);
        let hash = crate::database::calc_text_hash(&entry.content) as i64;
        let repo = SqliteClipboardRepository::new(conn.clone());
        let guard = conn.lock().expect("lock clipboard repository test db");
        guard
            .execute(
                "INSERT INTO cloud_sync_tombstones
                    (content_type, content_hash, hash_version, deleted_at)
                 VALUES ('text', ?1, 2, 50)",
                [hash],
            )
            .expect("insert matching tombstone");
        guard
            .execute_batch("BEGIN;")
            .expect("begin owner transaction");

        repo.save_with_conn(&guard, &entry, None)
            .expect("save inside owner transaction");
        let in_transaction_history: i64 = guard
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })
            .expect("count uncommitted history");
        let in_transaction_tombstones: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count uncommitted tombstones");
        assert_eq!(in_transaction_history, 1);
        assert_eq!(in_transaction_tombstones, 0);

        guard
            .execute_batch("ROLLBACK;")
            .expect("roll back owner transaction");
        let final_history: i64 = guard
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })
            .expect("count rolled-back history");
        let final_tombstones: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count restored tombstones");
        assert_eq!(final_history, 0);
        assert_eq!(final_tombstones, 1);
    }

    #[test]
    fn stale_positive_id_save_restores_tombstone_and_preserves_tags() {
        let conn = setup_repository_db();
        let mut entry = test_entry(&["new"]);
        entry.id = 999;
        let hash = crate::database::calc_text_hash(&entry.content) as i64;
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            guard
                .execute(
                    "INSERT INTO cloud_sync_tombstones
                        (content_type, content_hash, hash_version, deleted_at)
                     VALUES ('text', ?1, 2, 50)",
                    [hash],
                )
                .expect("insert matching tombstone");
            guard
                .execute(
                    "INSERT INTO entry_tags (entry_id, tag) VALUES (999, 'old')",
                    [],
                )
                .expect("insert pre-existing normalized tag");
        }

        let error = SqliteClipboardRepository::new(conn.clone())
            .save(&entry, None)
            .expect_err("a stale positive id must fail");
        assert!(error.contains("was not found for update"));

        let guard = conn.lock().expect("lock stale-save result db");
        let tombstone_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM cloud_sync_tombstones
                 WHERE content_type = 'text' AND content_hash = ?1 AND hash_version = 2",
                [hash],
                |row| row.get(0),
            )
            .expect("count restored tombstone");
        let tags: Vec<String> = guard
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = 999 ORDER BY tag")
            .expect("prepare retained tag query")
            .query_map([], |row| row.get(0))
            .expect("query retained tags")
            .collect::<Result<_, _>>()
            .expect("read retained tags");
        assert_eq!(tombstone_count, 1);
        assert_eq!(tags, vec!["old"]);
    }

    #[test]
    fn deleting_one_of_two_entries_preserves_shared_image_attachment() {
        let conn = setup_repository_db();
        let root = make_temp_dir("shared-image");
        let attachments = root.join("attachments");
        fs::create_dir_all(&attachments).expect("create attachments dir");
        let attachment = attachments.join("shared.png");
        fs::write(&attachment, b"shared image").expect("write shared attachment");
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            for id in [1, 2] {
                guard
                    .execute(
                        "INSERT INTO clipboard_history
                            (id, content_type, content, source_app, timestamp, preview,
                             content_hash, content_hash_version, tags, is_external,
                             sync_updated_at, sync_updated_by)
                         VALUES (?1, 'image', ?2, 'Test', 100, '[Image Content]',
                                 42, 2, '[]', 1, 100, 'local')",
                        params![id, attachment.to_string_lossy().as_ref()],
                    )
                    .expect("insert shared image entry");
            }
        }
        let repo = SqliteClipboardRepository::new(conn.clone());

        repo.delete(1, Some(&root))
            .expect("delete first image entry");
        assert!(
            attachment.exists(),
            "the surviving row still references the image"
        );

        repo.delete(2, Some(&root))
            .expect("delete second image entry");
        assert!(
            !attachment.exists(),
            "the final deletion may remove the image"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deleting_one_of_two_entries_preserves_shared_rich_fallback() {
        let conn = setup_repository_db();
        let root = make_temp_dir("shared-rich-fallback");
        let attachments = root.join("attachments");
        fs::create_dir_all(&attachments).expect("create attachments dir");
        let fallback = attachments.join("shared-fallback.png");
        fs::write(&fallback, b"shared fallback").expect("write rich fallback");
        let html = format!(
            "<p>shared</p><!--TIEZ_RICH_IMAGE:file://{}-->",
            fallback.to_string_lossy()
        );
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            for (id, content, hash) in [(1, "first", 41), (2, "second", 42)] {
                guard
                    .execute(
                        "INSERT INTO clipboard_history
                            (id, content_type, content, html_content, source_app, timestamp,
                             preview, content_hash, content_hash_version, tags, is_external,
                             sync_updated_at, sync_updated_by)
                         VALUES (?1, 'rich_text', ?2, ?3, 'Test', 100, ?2,
                                 ?4, 2, '[]', 0, 100, 'local')",
                        params![id, content, html, hash],
                    )
                    .expect("insert shared rich-text entry");
            }
        }
        let repo = SqliteClipboardRepository::new(conn.clone());

        repo.delete(1, Some(&root))
            .expect("delete first rich-text entry");
        assert!(
            fallback.exists(),
            "the surviving row still references the fallback"
        );

        repo.delete(2, Some(&root))
            .expect("delete second rich-text entry");
        assert!(
            !fallback.exists(),
            "the final deletion may remove the fallback"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_local_delete_keeps_database_rows_and_attachment() {
        let conn = setup_repository_db();
        let root = make_temp_dir("failed-delete");
        let attachments = root.join("attachments");
        fs::create_dir_all(&attachments).expect("create attachments dir");
        let attachment = attachments.join("keep.txt");
        fs::write(&attachment, b"keep me").expect("write retained attachment");
        {
            let guard = conn.lock().expect("lock clipboard repository test db");
            guard
                .execute(
                    "INSERT INTO clipboard_history
                        (id, content_type, content, source_app, timestamp, preview,
                         content_hash, content_hash_version, tags, is_external,
                         sync_updated_at, sync_updated_by)
                     VALUES (1, 'file', ?1, 'Test', 100, 'keep.txt', 42, 2,
                             '[\"blocked\"]', 1, 100, 'local')",
                    [attachment.to_string_lossy().as_ref()],
                )
                .expect("insert external clipboard entry");
            guard
                .execute(
                    "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'blocked')",
                    [],
                )
                .expect("insert normalized tag");
            guard
                .execute_batch(
                    "CREATE TRIGGER reject_entry_tag_delete
                     BEFORE DELETE ON entry_tags
                     BEGIN SELECT RAISE(ABORT, 'entry tag delete unavailable'); END;",
                )
                .expect("install delete failure trigger");
        }

        let error = SqliteClipboardRepository::new(conn.clone())
            .delete(1, Some(&root))
            .expect_err("tag deletion must fail the local deletion");
        assert!(error.contains("entry tag delete unavailable"));
        assert!(attachment.exists());

        let guard = conn.lock().expect("lock failed-delete result db");
        let history_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM clipboard_history WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count retained history");
        let tag_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM entry_tags WHERE entry_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count retained tag");
        let tombstone_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count rolled-back tombstone");
        assert_eq!(history_count, 1);
        assert_eq!(tag_count, 1);
        assert_eq!(tombstone_count, 0);
        drop(guard);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deleting_undecryptable_legacy_row_preserves_hash_and_version() {
        let conn = Connection::open_in_memory().expect("open clipboard test db");
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content TEXT NOT NULL,
                html_content TEXT,
                is_external INTEGER NOT NULL DEFAULT 0,
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL
             );
             CREATE TABLE entry_tags (entry_id INTEGER NOT NULL, tag TEXT NOT NULL);
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );
             INSERT INTO clipboard_history
                (id, content, content_type, content_hash, content_hash_version)
             VALUES (1, 'dpapi:not-decryptable', 'text', 4242, 1);",
        )
        .expect("create legacy deletion fixture");
        let conn = Arc::new(Mutex::new(conn));

        SqliteClipboardRepository::new(conn.clone())
            .delete(1, None)
            .expect("delete legacy row");

        let guard = conn.lock().expect("lock result db");
        let (hash, version): (i64, i64) = guard
            .query_row(
                "SELECT content_hash, hash_version FROM cloud_sync_tombstones",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read preserved tombstone");
        assert_eq!((hash, version), (4242, 1));
    }
}
