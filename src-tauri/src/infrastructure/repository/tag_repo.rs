use crate::database::ENCRYPT_PREFIX;
use crate::domain::models::ClipboardEntry;
use crate::infrastructure::encryption;
use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub trait TagRepository {
    fn set_color(&self, name: &str, color: Option<String>) -> Result<(), String>;
    fn get_colors(&self) -> Result<HashMap<String, String>, String>;
    fn get_all_with_counts(&self) -> Result<HashMap<String, i32>, String>;
    fn create(&self, name: &str) -> Result<(), String>;
    fn rename(&self, old_name: &str, new_name: &str) -> Result<(), String>;
    fn delete_globally(&self, name: &str, data_dir: Option<&std::path::Path>)
        -> Result<(), String>;
    fn get_entries_by_tag(&self, tag: &str) -> Result<Vec<ClipboardEntry>, String>;
    fn update_entry_tags(&self, id: i64, tags: Vec<String>) -> Result<(), String>;
}

pub struct SqliteTagRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteTagRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    fn maybe_decrypt_text(&self, value: &str) -> String {
        if value.starts_with(ENCRYPT_PREFIX) {
            encryption::decrypt_value(value).unwrap_or_else(|| value.to_string())
        } else {
            value.to_string()
        }
    }

    fn refresh_entry_tags_json(conn: &Connection, entry_id: i64) -> Result<(), String> {
        let mut stmt = conn
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = ? ORDER BY tag")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![entry_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;

        let mut tags: Vec<String> = Vec::new();
        for row in rows {
            if let Ok(tag) = row {
                if !tag.trim().is_empty() {
                    tags.push(tag);
                }
            }
        }

        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "UPDATE clipboard_history
             SET tags = ?1,
                 sync_updated_at = ?3,
                 sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
             WHERE id = ?2",
            params![tags_json, entry_id, now_ms()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl TagRepository for SqliteTagRepository {
    fn set_color(&self, name: &str, color: Option<String>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        if let Some(c) = color {
            conn.execute(
                "INSERT INTO saved_tags (name, color) VALUES (?1, ?2) 
                 ON CONFLICT(name) DO UPDATE SET color = ?2",
                params![name, c],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE saved_tags SET color = NULL WHERE name = ?1",
                params![name],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn get_colors(&self) -> Result<HashMap<String, String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT name, color FROM saved_tags WHERE color IS NOT NULL AND color != ''")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;

        let mut map = HashMap::new();
        for row in rows {
            if let Ok((name, color)) = row {
                map.insert(name, color);
            }
        }
        Ok(map)
    }

    fn get_all_with_counts(&self) -> Result<HashMap<String, i32>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT tag, COUNT(*) FROM entry_tags GROUP BY tag")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
            })
            .map_err(|e| e.to_string())?;

        let mut tag_counts: HashMap<String, i32> = HashMap::new();
        for row in rows {
            if let Ok((tag, count)) = row {
                tag_counts.insert(tag, count);
            }
        }

        // Also include saved tags with 0 count if not present
        let mut stmt_saved = conn
            .prepare("SELECT name FROM saved_tags")
            .map_err(|e| e.to_string())?;
        let saved_rows = stmt_saved
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;

        for row in saved_rows {
            if let Ok(name) = row {
                tag_counts.entry(name).or_insert(0);
            }
        }

        Ok(tag_counts)
    }

    fn create(&self, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO saved_tags (name) VALUES (?)",
            params![name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn rename(&self, old_name: &str, new_name: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // Update saved_tags table: merge color info if exists
        let old_color: Option<String> = conn
            .query_row(
                "SELECT color FROM saved_tags WHERE name = ?",
                params![old_name],
                |row| row.get(0),
            )
            .ok();

        conn.execute(
            "INSERT OR IGNORE INTO saved_tags (name, color) VALUES (?1, ?2)",
            params![new_name, old_color],
        )
        .map_err(|e| e.to_string())?;

        let _ = conn.execute("DELETE FROM saved_tags WHERE name = ?", params![old_name]);

        // Update entry_tags and refresh JSON cache
        let mut stmt = conn
            .prepare("SELECT entry_id FROM entry_tags WHERE tag = ?")
            .map_err(|e| e.to_string())?;
        let ids: Vec<i64> = stmt
            .query_map(params![old_name], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        for id in ids {
            conn.execute(
                "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                params![id, new_name],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "DELETE FROM entry_tags WHERE entry_id = ? AND tag = ?",
                params![id, old_name],
            )
            .map_err(|e| e.to_string())?;
            Self::refresh_entry_tags_json(&conn, id)?;
        }
        Ok(())
    }

    fn delete_globally(
        &self,
        name: &str,
        data_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        let entries = {
            let mut stmt = tx
                .prepare(
                    "SELECT h.id, h.content, h.html_content, h.is_external
                     FROM clipboard_history h
                     INNER JOIN entry_tags t ON t.entry_id = h.id
                     WHERE t.tag = ?",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![name], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i32>(3)? == 1,
                    ))
                })
                .map_err(|e| e.to_string())?;
            let mut entries = Vec::new();
            for row in rows {
                entries.push(row.map_err(|e| e.to_string())?);
            }
            entries
        };

        let clipboard_repo = SqliteClipboardRepository::new(self.conn.clone());
        let mut cleanup_paths = HashSet::new();
        if let Some(dir) = data_dir {
            let attachments_dir = dir.join("attachments");
            for (_, content, html, is_external) in &entries {
                cleanup_paths.extend(clipboard_repo.collect_attachment_paths_for_cleanup(
                    content,
                    html.as_deref(),
                    *is_external,
                    &attachments_dir,
                ));
            }
        }

        // Tombstones, entry metadata/history, normalized tags, and the saved tag
        // are one database unit. Filesystem cleanup is deliberately deferred until
        // after commit so a database rollback never leaves retained rows without files.
        for (id, _, _, _) in &entries {
            clipboard_repo.delete_with_conn(&tx, *id, None)?;
        }
        tx.execute("DELETE FROM saved_tags WHERE name = ?", params![name])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;

        if let Some(dir) = data_dir {
            clipboard_repo.cleanup_unreferenced_attachment_paths_with_conn(
                &conn,
                cleanup_paths,
                &dir.join("attachments"),
            );
        }
        Ok(())
    }

    fn get_entries_by_tag(&self, tag: &str) -> Result<Vec<ClipboardEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT ch.id, ch.content_type, ch.content, ch.html_content, ch.source_app, ch.timestamp, ch.preview, ch.is_pinned, ch.tags, ch.use_count, ch.is_external, ch.pinned_order, ch.source_app_path 
             FROM clipboard_history ch
             INNER JOIN entry_tags et ON ch.id = et.entry_id
             WHERE et.tag = ? 
             ORDER BY ch.is_pinned DESC, ch.pinned_order DESC, ch.timestamp DESC",
        ).map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([tag], |row| {
                let tags_str: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
                let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
                let content_raw: String = row.get(2)?;
                let html_raw: Option<String> = row.get(3).ok();
                let preview_raw: String = row.get(6)?;
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
                    file_preview_exists: true, // simplified
                })
            })
            .map_err(|e| e.to_string())?;

        let mut history = Vec::new();
        for row in rows {
            if let Ok(entry) = row {
                history.push(entry);
            }
        }
        Ok(history)
    }

    fn update_entry_tags(&self, id: i64, tags: Vec<String>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut cleaned: Vec<String> = Vec::new();
        for tag in tags {
            let t = tag.trim();
            if t.is_empty() {
                continue;
            }
            let t_owned = t.to_string();
            if seen.insert(t_owned.clone()) {
                cleaned.push(t_owned);
            }
        }

        conn.execute("DELETE FROM entry_tags WHERE entry_id = ?", params![id])
            .map_err(|e| e.to_string())?;
        for tag in &cleaned {
            conn.execute(
                "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                params![id, tag],
            )
            .map_err(|e| e.to_string())?;
        }

        let tags_json = serde_json::to_string(&cleaned).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "UPDATE clipboard_history
             SET tags = ?1,
                 sync_updated_at = ?3,
                 sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
             WHERE id = ?2",
            params![tags_json, id, now_ms()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SqliteTagRepository, TagRepository};
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    fn setup_tag_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().expect("open tag test db");
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO settings (key, value) VALUES ('app.anon_id', 'device-a');
             CREATE TABLE saved_tags (name TEXT PRIMARY KEY, color TEXT);
             CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                html_content TEXT,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL DEFAULT 2,
                tags TEXT NOT NULL DEFAULT '[]',
                is_external INTEGER NOT NULL DEFAULT 0,
                sync_updated_at INTEGER NOT NULL DEFAULT 0,
                sync_updated_by TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL DEFAULT 2,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );",
        )
        .expect("create tag test schema");
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn deleting_global_tag_records_tombstone_before_removing_entry() {
        let conn = setup_tag_db();
        {
            let guard = conn.lock().expect("lock tag test db");
            guard
                .execute("INSERT INTO saved_tags (name) VALUES ('remove-me')", [])
                .expect("insert saved tag");
            guard
                .execute(
                    "INSERT INTO clipboard_history
                     (id, content_type, content, content_hash, tags)
                     VALUES (1, 'text', 'shared', 4242, '[\"remove-me\"]')",
                    [],
                )
                .expect("insert tagged entry");
            guard
                .execute(
                    "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remove-me')",
                    [],
                )
                .expect("insert normalized tag");
        }
        let repo = SqliteTagRepository::new(conn.clone());

        repo.delete_globally("remove-me", None)
            .expect("delete global tag");

        let guard = conn.lock().expect("lock result db");
        let history_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })
            .expect("count history");
        let (content_type, content_hash, deleted_at): (String, i64, i64) = guard
            .query_row(
                "SELECT content_type, content_hash, deleted_at
                 FROM cloud_sync_tombstones",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read tombstone");
        assert_eq!(history_count, 0);
        assert_eq!(content_type, "text");
        assert_eq!(
            content_hash,
            crate::database::calc_text_hash("shared") as i64
        );
        assert!(deleted_at > 0);
    }

    #[test]
    fn tombstone_failure_keeps_tagged_entry() {
        let conn = setup_tag_db();
        {
            let guard = conn.lock().expect("lock tag test db");
            guard
                .execute_batch(
                    "INSERT INTO saved_tags (name) VALUES ('remove-me');
                     INSERT INTO clipboard_history
                        (id, content_type, content, content_hash, tags)
                        VALUES (1, 'text', 'hello', 42, '[\"remove-me\"]');
                     INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remove-me');
                     CREATE TRIGGER reject_tombstone BEFORE INSERT ON cloud_sync_tombstones
                     BEGIN SELECT RAISE(ABORT, 'tombstone unavailable'); END;",
                )
                .expect("seed failed-delete fixture");
        }

        let error = SqliteTagRepository::new(conn.clone())
            .delete_globally("remove-me", None)
            .expect_err("tombstone insertion must fail");
        assert!(error.contains("tombstone unavailable"));
        let guard = conn.lock().expect("lock tag test db");
        let remaining: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM clipboard_history WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count retained entry");
        let saved_tag_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM saved_tags WHERE name = 'remove-me'",
                [],
                |row| row.get(0),
            )
            .expect("count retained saved tag");
        let entry_tag_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM entry_tags WHERE entry_id = 1 AND tag = 'remove-me'",
                [],
                |row| row.get(0),
            )
            .expect("count retained normalized tag");
        let tombstone_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get(0)
            })
            .expect("count rolled-back tombstones");
        assert_eq!(remaining, 1);
        assert_eq!(saved_tag_count, 1);
        assert_eq!(entry_tag_count, 1);
        assert_eq!(tombstone_count, 0);
    }

    #[test]
    fn deleting_global_tag_reuses_attachment_cleanup() {
        let conn = setup_tag_db();
        let root = std::env::temp_dir().join(format!(
            "tiez-tag-delete-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let attachments = root.join("attachments");
        std::fs::create_dir_all(&attachments).expect("create attachments dir");
        let attachment = attachments.join("payload.txt");
        std::fs::write(&attachment, b"payload").expect("write attachment");
        {
            let guard = conn.lock().expect("lock tag test db");
            guard
                .execute("INSERT INTO saved_tags (name) VALUES ('remove-me')", [])
                .expect("insert saved tag");
            guard
                .execute(
                    "INSERT INTO clipboard_history
                     (id, content_type, content, content_hash, tags, is_external)
                     VALUES (1, 'file', ?1, 4242, '[\"remove-me\"]', 1)",
                    [attachment.to_string_lossy().as_ref()],
                )
                .expect("insert external entry");
            guard
                .execute(
                    "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remove-me')",
                    [],
                )
                .expect("insert normalized tag");
        }

        SqliteTagRepository::new(conn)
            .delete_globally("remove-me", Some(&root))
            .expect("delete global tag");

        assert!(!attachment.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn global_tag_delete_preserves_shared_rich_fallback() {
        let conn = setup_tag_db();
        let root = std::env::temp_dir().join(format!(
            "tiez-tag-shared-rich-delete-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let attachments = root.join("attachments");
        std::fs::create_dir_all(&attachments).expect("create attachments dir");
        let fallback = attachments.join("shared-fallback.png");
        std::fs::write(&fallback, b"shared fallback").expect("write shared fallback");
        let html = format!(
            "<p>shared</p><!--TIEZ_RICH_IMAGE:file://{}-->",
            fallback.to_string_lossy()
        );
        {
            let guard = conn.lock().expect("lock tag test db");
            guard
                .execute_batch("INSERT INTO saved_tags (name) VALUES ('remove-me');")
                .expect("insert saved tag");
            guard
                .execute(
                    "INSERT INTO clipboard_history
                     (id, content_type, content, html_content, content_hash, tags)
                     VALUES (1, 'rich_text', 'delete me', ?1, 41, '[\"remove-me\"]'),
                            (2, 'rich_text', 'keep me', ?1, 42, '[]')",
                    [html],
                )
                .expect("insert shared rich-text entries");
            guard
                .execute(
                    "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remove-me')",
                    [],
                )
                .expect("insert normalized tag");
        }

        SqliteTagRepository::new(conn.clone())
            .delete_globally("remove-me", Some(&root))
            .expect("delete global tag");

        assert!(
            fallback.exists(),
            "a surviving row still references the fallback"
        );
        let guard = conn.lock().expect("lock shared fallback result db");
        let remaining: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM clipboard_history WHERE id = 2",
                [],
                |row| row.get(0),
            )
            .expect("count surviving row");
        assert_eq!(remaining, 1);
        drop(guard);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn updating_entry_tags_advances_sync_revision() {
        let conn = setup_tag_db();
        {
            let guard = conn.lock().expect("lock tag test db");
            guard
                .execute(
                    "INSERT INTO clipboard_history
                     (id, content_type, content, content_hash, tags, sync_updated_at)
                     VALUES (1, 'text', 'shared', 4242, '[]', 1)",
                    [],
                )
                .expect("insert entry");
        }
        let repo = SqliteTagRepository::new(conn.clone());

        repo.update_entry_tags(1, vec!["new-tag".to_string()])
            .expect("update tags");

        let guard = conn.lock().expect("lock result db");
        let (tags, updated_at, updated_by): (String, i64, String) = guard
            .query_row(
                "SELECT tags, sync_updated_at, sync_updated_by
                 FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read sync revision");
        assert_eq!(tags, "[\"new-tag\"]");
        assert!(updated_at > 1);
        assert_eq!(updated_by, "device-a");
    }
}
