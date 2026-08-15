//! Production SQLite mutation semantics shared by TieZ desktop runtimes.
//!
//! Platform adapters remain responsible for capture, encryption, attachment
//! lifetime, and UI events. This module owns the atomic history/tag/tombstone
//! writes that must stay identical across those adapters.

use crate::content_identity::{calc_legacy_text_hash, uses_text_content_hash};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;

const SAVEPOINT: &str = "tiez_clipboard_repository_write";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedClipboardRecord<'a> {
    pub id: i64,
    pub content_type: &'a str,
    pub content: &'a str,
    pub identity_content: &'a str,
    pub html_content: Option<&'a str>,
    pub source_app: &'a str,
    pub source_app_path: Option<&'a str>,
    pub timestamp: i64,
    pub preview: &'a str,
    pub is_pinned: bool,
    pub content_hash: i64,
    pub tags: &'a [String],
    pub is_external: bool,
    pub pinned_order: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredClipboardRecord {
    pub content: String,
    pub html_content: Option<String>,
    pub is_external: bool,
    pub content_type: String,
    pub content_hash: i64,
    pub content_hash_version: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteRecordPlan<'a> {
    pub id: i64,
    pub content_type: &'a str,
    pub content_hash: i64,
    pub content_hash_version: i64,
    pub deleted_at: i64,
}

pub fn is_syncable_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "text" | "code" | "url" | "rich_text" | "image" | "file" | "video" | "emoji_sync"
    )
}

pub fn save_prepared_record(
    conn: &Connection,
    record: &PreparedClipboardRecord<'_>,
) -> Result<i64, String> {
    let cleaned_tags = clean_tags(record.tags);
    let tags_json = serde_json::to_string(&cleaned_tags).map_err(|error| error.to_string())?;

    with_savepoint(conn, || {
        clear_tombstone(
            conn,
            record.content_type,
            record.identity_content,
            record.content_hash,
        )?;

        let entry_id = if record.id > 0 {
            let affected = conn
                .execute(
                    "UPDATE clipboard_history SET
                        content_type = ?1,
                        content = ?2,
                        html_content = ?3,
                        source_app = ?4,
                        timestamp = ?5,
                        preview = ?6,
                        content_hash = ?7,
                        content_hash_version = 2,
                        tags = ?8,
                        is_external = ?9,
                        source_app_path = ?10,
                        use_count = use_count + 1,
                        sync_updated_at = ?11,
                        sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
                     WHERE id = ?12",
                    params![
                        record.content_type,
                        record.content,
                        record.html_content,
                        record.source_app,
                        record.timestamp,
                        record.preview,
                        record.content_hash,
                        tags_json,
                        i32::from(record.is_external),
                        record.source_app_path,
                        record.timestamp,
                        record.id,
                    ],
                )
                .map_err(|error| error.to_string())?;
            if affected != 1 {
                return Err(format!(
                    "clipboard entry {} was not found for update",
                    record.id
                ));
            }
            record.id
        } else {
            conn.execute(
                "INSERT INTO clipboard_history
                    (content_type, content, html_content, source_app, timestamp, preview,
                     is_pinned, content_hash, content_hash_version, tags, is_external,
                     pinned_order, source_app_path, sync_updated_at, sync_updated_by)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 2, ?9, ?10, ?11, ?12, ?13,
                     COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), ''))",
                params![
                    record.content_type,
                    record.content,
                    record.html_content,
                    record.source_app,
                    record.timestamp,
                    record.preview,
                    i32::from(record.is_pinned),
                    record.content_hash,
                    tags_json,
                    i32::from(record.is_external),
                    record.pinned_order,
                    record.source_app_path,
                    record.timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
            conn.last_insert_rowid()
        };

        sync_entry_tags(conn, entry_id, &cleaned_tags)?;
        Ok(entry_id)
    })
}

pub fn load_stored_record(
    conn: &Connection,
    id: i64,
) -> Result<Option<StoredClipboardRecord>, String> {
    conn.query_row(
        "SELECT content, html_content, is_external, content_type, content_hash,
                content_hash_version
         FROM clipboard_history WHERE id = ?1",
        [id],
        |row| {
            Ok(StoredClipboardRecord {
                content: row.get(0)?,
                html_content: row.get(1)?,
                is_external: row.get::<_, i32>(2)? == 1,
                content_type: row.get(3)?,
                content_hash: row.get(4)?,
                content_hash_version: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn delete_record(conn: &Connection, plan: DeleteRecordPlan<'_>) -> Result<bool, String> {
    with_savepoint(conn, || {
        let deleted = conn
            .execute("DELETE FROM clipboard_history WHERE id = ?1", [plan.id])
            .map_err(|error| error.to_string())?;
        if deleted != 1 {
            return Ok(false);
        }
        upsert_tombstone(
            conn,
            plan.content_type,
            plan.content_hash,
            plan.content_hash_version,
            plan.deleted_at,
        )?;
        conn.execute("DELETE FROM entry_tags WHERE entry_id = ?1", [plan.id])
            .map_err(|error| error.to_string())?;
        Ok(true)
    })
}

pub fn set_pinned(
    conn: &Connection,
    id: i64,
    is_pinned: bool,
    updated_at: i64,
) -> Result<bool, String> {
    let affected = if is_pinned {
        conn.execute(
            "UPDATE clipboard_history
             SET is_pinned = 1,
                 pinned_order = (SELECT COALESCE(MAX(pinned_order), 0) + 1
                                 FROM clipboard_history WHERE is_pinned = 1),
                 sync_updated_at = ?2,
                 sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
             WHERE id = ?1",
            params![id, updated_at],
        )
    } else {
        conn.execute(
            "UPDATE clipboard_history
             SET is_pinned = 0,
                 pinned_order = 0,
                 sync_updated_at = ?2,
                 sync_updated_by = COALESCE((SELECT value FROM settings WHERE key = 'app.anon_id'), '')
             WHERE id = ?1",
            params![id, updated_at],
        )
    }
    .map_err(|error| error.to_string())?;
    Ok(affected == 1)
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut cleaned = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || !seen.insert(tag.to_owned()) {
            continue;
        }
        cleaned.push(tag.to_owned());
    }
    cleaned
}

fn sync_entry_tags(conn: &Connection, entry_id: i64, tags: &[String]) -> Result<(), String> {
    conn.execute("DELETE FROM entry_tags WHERE entry_id = ?1", [entry_id])
        .map_err(|error| error.to_string())?;
    for tag in tags {
        conn.execute(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
            params![entry_id, tag],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn upsert_tombstone(
    conn: &Connection,
    content_type: &str,
    content_hash: i64,
    hash_version: i64,
    deleted_at: i64,
) -> Result<(), String> {
    if !is_syncable_content_type(content_type) || content_hash == 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO cloud_sync_tombstones
            (content_type, content_hash, hash_version, deleted_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(content_type, content_hash, hash_version)
         DO UPDATE SET deleted_at = MAX(cloud_sync_tombstones.deleted_at, excluded.deleted_at)",
        params![content_type, content_hash, hash_version.max(1), deleted_at],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn clear_tombstone(
    conn: &Connection,
    content_type: &str,
    identity_content: &str,
    content_hash: i64,
) -> Result<(), String> {
    if !is_syncable_content_type(content_type) || content_hash == 0 {
        return Ok(());
    }
    let legacy_hash = if uses_text_content_hash(content_type) {
        calc_legacy_text_hash(identity_content) as i64
    } else {
        content_hash
    };
    conn.execute(
        "DELETE FROM cloud_sync_tombstones
         WHERE content_type = ?1
           AND ((hash_version <= 1 AND content_hash IN (?2, ?3))
             OR (hash_version >= 2 AND content_hash = ?3))",
        params![content_type, legacy_hash, content_hash],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn with_savepoint<T>(
    conn: &Connection,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT};"))
        .map_err(|error| error.to_string())?;

    match operation() {
        Ok(value) => match conn.execute_batch(&format!("RELEASE SAVEPOINT {SAVEPOINT};")) {
            Ok(()) => Ok(value),
            Err(release_error) => {
                let rollback = conn.execute_batch(&format!(
                    "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT};"
                ));
                match rollback {
                    Ok(()) => Err(release_error.to_string()),
                    Err(rollback_error) => Err(format!(
                        "{release_error}; savepoint rollback also failed: {rollback_error}"
                    )),
                }
            }
        },
        Err(operation_error) => {
            let rollback = conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT};"
            ));
            match rollback {
                Ok(()) => Err(operation_error),
                Err(rollback_error) => Err(format!(
                    "{operation_error}; savepoint rollback also failed: {rollback_error}"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_identity::{calc_legacy_text_hash, calc_text_hash};
    use crate::database_migrations::run_migrations;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('app.anon_id', 'native-test')",
            [],
        )
        .unwrap();
        conn
    }

    fn text_record<'a>(content: &'a str, tags: &'a [String]) -> PreparedClipboardRecord<'a> {
        PreparedClipboardRecord {
            id: 0,
            content_type: "text",
            content,
            identity_content: content,
            html_content: None,
            source_app: "Notepad",
            source_app_path: None,
            timestamp: 1234,
            preview: content,
            is_pinned: false,
            content_hash: calc_text_hash(content) as i64,
            tags,
            is_external: false,
            pinned_order: 0,
        }
    }

    #[test]
    fn save_writes_v2_identity_sync_revision_and_normalized_tags() {
        let conn = database();
        let tags = vec![" work ".to_owned(), "work".to_owned(), "".to_owned()];

        let id = save_prepared_record(&conn, &text_record("hello", &tags)).unwrap();

        let row: (i64, i64, String, String) = conn
            .query_row(
                "SELECT content_hash, content_hash_version, tags, sync_updated_by
                 FROM clipboard_history WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, calc_text_hash("hello") as i64);
        assert_eq!(row.1, 2);
        assert_eq!(row.2, "[\"work\"]");
        assert_eq!(row.3, "native-test");
        assert_eq!(
            conn.query_row(
                "SELECT tag FROM entry_tags WHERE entry_id = ?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "work"
        );
    }

    #[test]
    fn save_clears_legacy_and_current_tombstones_atomically() {
        let conn = database();
        let content = " hello \r\n";
        let current_hash = calc_text_hash(content) as i64;
        let legacy_hash = calc_legacy_text_hash(content) as i64;
        for (hash, version) in [(legacy_hash, 1), (current_hash, 1), (current_hash, 2)] {
            conn.execute(
                "INSERT INTO cloud_sync_tombstones
                    (content_type, content_hash, hash_version, deleted_at)
                 VALUES ('text', ?1, ?2, 10)",
                params![hash, version],
            )
            .unwrap();
        }

        save_prepared_record(&conn, &text_record(content, &[])).unwrap();

        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
    }

    #[test]
    fn failed_save_restores_tombstone_and_history() {
        let conn = database();
        let content = "atomic";
        let hash = calc_text_hash(content) as i64;
        conn.execute(
            "INSERT INTO cloud_sync_tombstones
                (content_type, content_hash, hash_version, deleted_at)
             VALUES ('text', ?1, 2, 10)",
            [hash],
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_entry_tag BEFORE INSERT ON entry_tags
             BEGIN SELECT RAISE(ABORT, 'reject tag'); END;",
        )
        .unwrap();
        let tags = vec!["work".to_owned()];

        assert!(save_prepared_record(&conn, &text_record(content, &tags)).is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
    }

    #[test]
    fn delete_creates_tombstone_and_removes_tags_in_one_unit() {
        let conn = database();
        let tags = vec!["work".to_owned()];
        let id = save_prepared_record(&conn, &text_record("delete me", &tags)).unwrap();
        let hash = calc_text_hash("delete me") as i64;

        assert!(delete_record(
            &conn,
            DeleteRecordPlan {
                id,
                content_type: "text",
                content_hash: hash,
                content_hash_version: 2,
                deleted_at: 5678,
            },
        )
        .unwrap());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM entry_tags", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT deleted_at FROM cloud_sync_tombstones
                 WHERE content_type = 'text' AND content_hash = ?1 AND hash_version = 2",
                [hash],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            5678
        );
    }

    #[test]
    fn missing_delete_does_not_create_a_tombstone() {
        let conn = database();

        assert!(!delete_record(
            &conn,
            DeleteRecordPlan {
                id: 404,
                content_type: "text",
                content_hash: calc_text_hash("missing") as i64,
                content_hash_version: 2,
                deleted_at: 5678,
            },
        )
        .unwrap());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
    }

    #[test]
    fn set_pin_updates_order_and_sync_revision() {
        let conn = database();
        let id = save_prepared_record(&conn, &text_record("pin me", &[])).unwrap();

        assert!(set_pinned(&conn, id, true, 9000).unwrap());
        let pinned: (i64, i64, i64, String) = conn
            .query_row(
                "SELECT is_pinned, pinned_order, sync_updated_at, sync_updated_by
                 FROM clipboard_history WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(pinned, (1, 1, 9000, "native-test".to_owned()));
        assert!(!set_pinned(&conn, id + 1, true, 9001).unwrap());
    }
}
