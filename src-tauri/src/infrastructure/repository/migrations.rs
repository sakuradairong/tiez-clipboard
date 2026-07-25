use crate::database::{calc_text_hash, uses_text_content_hash, ENCRYPT_PREFIX};
#[cfg(windows)]
use crate::infrastructure::encryption;
use rusqlite::{params, Connection, Result};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    let current_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Migration 1: Initial Baseline
    if current_version < 1 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                source_app TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                preview TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (1)", [])?;
    }

    // Migration 2: Add core feature columns
    if current_version < 2 {
        let columns = [
            ("is_pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("tags", "TEXT NOT NULL DEFAULT '[]'"),
            ("use_count", "INTEGER NOT NULL DEFAULT 0"),
            ("pinned_order", "INTEGER NOT NULL DEFAULT 0"),
            ("content_hash", "INTEGER NOT NULL DEFAULT 0"),
            ("html_content", "TEXT"),
        ];

        for (name, def) in columns {
            if !has_column(conn, "clipboard_history", name)? {
                conn.execute(
                    &format!("ALTER TABLE clipboard_history ADD COLUMN {} {}", name, def),
                    [],
                )?;
            }
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (2)", [])?;
    }

    // Migration 3: Add is_external
    if current_version < 3 {
        if !has_column(conn, "clipboard_history", "is_external")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN is_external INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (3)", [])?;
    }

    // Migration 4: Tag management
    if current_version < 4 {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS saved_tags (
                name TEXT PRIMARY KEY,
                color TEXT
            )",
            [],
        )?;

        // Insert default tags
        let _ = conn.execute(
            "INSERT OR IGNORE INTO saved_tags (name) VALUES ('sensitive')",
            [],
        );
        let _ = conn.execute(
            "INSERT OR IGNORE INTO saved_tags (name) VALUES ('密码')",
            [],
        );

        conn.execute("INSERT INTO schema_migrations (version) VALUES (4)", [])?;
    }

    // Migration 5: Performance indexes
    if current_version < 5 {
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_pinned_order_time
                ON clipboard_history (is_pinned, pinned_order, timestamp);
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_type_hash
                ON clipboard_history (content_type, content_hash);
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_timestamp
                ON clipboard_history (timestamp);
        ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (5)", [])?;
    }

    // Migration 6: Normalize tags into entry_tags
    if current_version < 6 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_entry_tags_tag ON entry_tags (tag);
            CREATE INDEX IF NOT EXISTS idx_entry_tags_entry ON entry_tags (entry_id);
        ",
        )?;

        // Backfill entry_tags from clipboard_history.tags JSON
        conn.execute("BEGIN", [])?;
        let backfill = (|| -> Result<()> {
            let mut stmt = conn.prepare("SELECT id, tags FROM clipboard_history")?;
            let rows = stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let tags: Option<String> = row.get(1)?;
                Ok((id, tags.unwrap_or_else(|| "[]".to_string())))
            })?;

            for row in rows {
                let (id, tags_json) = row?;
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                for tag in tags {
                    if tag.trim().is_empty() {
                        continue;
                    }
                    conn.execute(
                        "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                        params![id, tag],
                    )?;
                }
            }
            Ok(())
        })();

        if let Err(err) = backfill {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
        conn.execute("COMMIT", [])?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (6)", [])?;
    }

    // Migration 7: Cloud sync tombstones for deletion propagation
    if current_version < 7 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_cloud_sync_tombstones_deleted_at
                ON cloud_sync_tombstones (deleted_at);
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (7)", [])?;
    }

    // Migration 8: Local incremental sync index (for delta diff against last uploaded state)
    if current_version < 8 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
            );
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (8)", [])?;
    }

    // Migration 9: Persist source app path for source icon rendering
    if current_version < 9 {
        if !has_column(conn, "clipboard_history", "source_app_path")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN source_app_path TEXT",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (9)", [])?;
    }

    // Migration 10: Cloud sync content type preferences (per-device; not synced in settings snapshot)
    if current_version < 10 {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('cloud_sync_content_prefs', '{\"text\":true,\"image\":true,\"file_path\":true,\"emoji\":true}')",
            [],
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (10)", [])?;
    }

    // Migration 11: Per-entry metadata revision for deterministic multi-device merges.
    if current_version < 11 {
        if !has_column(conn, "clipboard_history", "sync_updated_at")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN sync_updated_at INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !has_column(conn, "clipboard_history", "sync_updated_by")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN sync_updated_by TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        conn.execute(
            "UPDATE clipboard_history
             SET sync_updated_at = timestamp
             WHERE sync_updated_at <= 0",
            [],
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (11)", [])?;
    }

    // Migration 12: Local OCR and QR index for image clipboard entries.
    //
    // Some development databases used version 12 for the local hash migration
    // before this upstream migration landed. Checking the table as well as the
    // version repairs those databases without replaying either migration.
    if current_version < 12 || !has_table(conn, "clipboard_image_analysis")? {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS clipboard_image_analysis (
                entry_id INTEGER PRIMARY KEY,
                content_hash INTEGER NOT NULL,
                ocr_text TEXT NOT NULL DEFAULT '',
                qr_codes TEXT NOT NULL DEFAULT '[]',
                language TEXT,
                analyzed_at INTEGER NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS trg_clipboard_image_analysis_delete
            AFTER DELETE ON clipboard_history
            BEGIN
                DELETE FROM clipboard_image_analysis WHERE entry_id = OLD.id;
            END;
            ",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version) VALUES (12)",
            [],
        )?;
    }

    // Migration 13: Match the history pagination sort order exactly.
    //
    // The original index omitted the id tie-breaker and filtered history had
    // no content_type-prefixed sort index. Both cases can make SQLite build a
    // temporary B-tree while the clipboard window is opening or scrolling.
    if current_version < 13 {
        conn.execute_batch(
            "
            DROP INDEX IF EXISTS idx_clipboard_history_pinned_order_time;
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_sort
                ON clipboard_history (
                    is_pinned DESC,
                    pinned_order DESC,
                    timestamp DESC,
                    id DESC
                );
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_type_sort
                ON clipboard_history (
                    content_type,
                    is_pinned DESC,
                    pinned_order DESC,
                    timestamp DESC,
                    id DESC
                );
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (13)", [])?;
    }

    // Migration 14: Version content hashes. Existing tombstones retain the legacy
    // trim-equivalence semantics (v1), while readable live rows are canonicalized to
    // whitespace-preserving hashes (v2). Encrypted rows remain v1 if decryption fails.
    if current_version < 14 {
        conn.execute("BEGIN", [])?;
        let backfill = (|| -> Result<()> {
            let history_is_versioned =
                has_column(conn, "clipboard_history", "content_hash_version")?;
            let tombstones_are_versioned =
                has_column(conn, "cloud_sync_tombstones", "hash_version")?;

            // A fully versioned schema means the old local migration 12 already
            // completed. Only record its new canonical version number.
            if history_is_versioned && tombstones_are_versioned {
                conn.execute(
                    "INSERT OR IGNORE INTO schema_migrations (version) VALUES (14)",
                    [],
                )?;
                return Ok(());
            }

            if !history_is_versioned {
                conn.execute(
                    "ALTER TABLE clipboard_history ADD COLUMN content_hash_version INTEGER NOT NULL DEFAULT 2",
                    [],
                )?;
            }
            conn.execute("UPDATE clipboard_history SET content_hash_version = 1", [])?;

            // Rebuild so hash version is part of the tombstone identity. A v1 and v2
            // tombstone may legitimately share the same numeric hash.
            conn.execute_batch(
                "CREATE TABLE cloud_sync_tombstones_v14 (
                    content_type TEXT NOT NULL,
                    content_hash INTEGER NOT NULL,
                    hash_version INTEGER NOT NULL DEFAULT 1,
                    deleted_at INTEGER NOT NULL,
                    PRIMARY KEY (content_type, content_hash, hash_version)
                 );
                 INSERT INTO cloud_sync_tombstones_v14
                    (content_type, content_hash, hash_version, deleted_at)
                    SELECT content_type, content_hash, 1, deleted_at FROM cloud_sync_tombstones;
                 DROP TABLE cloud_sync_tombstones;
                 ALTER TABLE cloud_sync_tombstones_v14 RENAME TO cloud_sync_tombstones;
                 CREATE INDEX idx_cloud_sync_tombstones_deleted_at
                    ON cloud_sync_tombstones (deleted_at);",
            )?;

            let mut stmt = conn
                .prepare("SELECT id, content_type, content, content_hash FROM clipboard_history")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            let mut updates = Vec::new();
            for row in rows {
                let (id, content_type, stored_content, old_hash) = row?;
                if !uses_text_content_hash(&content_type) {
                    updates.push((id, old_hash));
                    continue;
                }
                let plain_content = if stored_content.starts_with(ENCRYPT_PREFIX) {
                    #[cfg(windows)]
                    let decrypted = encryption::decrypt_value(&stored_content);
                    #[cfg(not(windows))]
                    let decrypted: Option<String> = None;
                    let Some(decrypted) = decrypted else {
                        continue;
                    };
                    decrypted
                } else {
                    stored_content
                };
                updates.push((id, calc_text_hash(&plain_content) as i64));
            }
            drop(stmt);

            for (id, new_hash) in updates {
                conn.execute(
                    "UPDATE clipboard_history
                     SET content_hash = ?1, content_hash_version = 2 WHERE id = ?2",
                    params![new_hash, id],
                )?;
            }

            // Hash versions are part of sync keys and digests, so no pre-migration
            // suppression entry remains trustworthy after this backfill.
            conn.execute("DELETE FROM cloud_sync_local_index", [])?;
            conn.execute("INSERT INTO schema_migrations (version) VALUES (14)", [])?;
            Ok(())
        })();
        if let Err(err) = backfill {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
        conn.execute("COMMIT", [])?;
    }

    Ok(())
}

fn has_column(conn: &Connection, table_name: &str, column_name: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table_name))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_table(conn: &Connection, table_name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
        )",
        [table_name],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::{has_column, run_migrations};
    use crate::database::calc_text_hash;
    use rusqlite::Connection;

    #[test]
    fn migration_11_backfills_sync_revision_from_timestamp() {
        let conn = Connection::open_in_memory().expect("open migration test db");
        conn.execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_migrations (version) VALUES (10);
             CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL DEFAULT 'text',
                content TEXT NOT NULL DEFAULT '',
                content_hash INTEGER NOT NULL DEFAULT 0,
                timestamp INTEGER NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                pinned_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash)
             );
             INSERT INTO clipboard_history (id, timestamp) VALUES (1, 12345);",
        )
        .expect("create version 10 schema");

        run_migrations(&conn).expect("run migration 11");

        assert!(has_column(&conn, "clipboard_history", "sync_updated_at")
            .expect("check sync_updated_at"));
        assert!(has_column(&conn, "clipboard_history", "sync_updated_by")
            .expect("check sync_updated_by"));
        let (updated_at, updated_by): (i64, String) = conn
            .query_row(
                "SELECT sync_updated_at, sync_updated_by
                 FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read migrated revision");
        assert_eq!(updated_at, 12345);
        assert!(updated_by.is_empty());

        let analysis_table_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'table' AND name = 'clipboard_image_analysis'
                )",
                [],
                |row| row.get(0),
            )
            .expect("check image analysis table");
        assert!(analysis_table_exists);

        conn.execute(
            "INSERT INTO clipboard_image_analysis
                (entry_id, content_hash, ocr_text, qr_codes, analyzed_at)
             VALUES (1, 42, 'searchable text', '[]', 12345)",
            [],
        )
        .expect("insert image analysis");
        conn.execute("DELETE FROM clipboard_history WHERE id = 1", [])
            .expect("delete clipboard entry");
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM clipboard_image_analysis WHERE entry_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count orphaned image analysis");
        assert_eq!(remaining, 0);

        let sort_indexes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index'
                   AND name IN (
                       'idx_clipboard_history_sort',
                       'idx_clipboard_history_type_sort'
                   )",
                [],
                |row| row.get(0),
            )
            .expect("count history sort indexes");
        assert_eq!(sort_indexes, 2);

        let history_plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM clipboard_history
                 ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
                 LIMIT 80 OFFSET 0",
                [],
                |row| row.get(3),
            )
            .expect("explain history pagination");
        assert!(history_plan.contains("idx_clipboard_history_sort"));

        let filtered_plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM clipboard_history
                 WHERE content_type = 'image'
                 ORDER BY is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC
                 LIMIT 80 OFFSET 0",
                [],
                |row| row.get(3),
            )
            .expect("explain filtered history pagination");
        assert!(filtered_plan.contains("idx_clipboard_history_type_sort"));
    }

    fn setup_version_11_hash_schema() -> Connection {
        let conn = Connection::open_in_memory().expect("open migration test db");
        conn.execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_migrations (version) VALUES (11);
             CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                timestamp INTEGER NOT NULL DEFAULT 0,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                pinned_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash)
             );",
        )
        .expect("create version 11 schema");
        conn
    }

    #[test]
    fn migration_14_separates_edge_whitespace_hashes_and_invalidates_sync_index() {
        let conn = setup_version_11_hash_schema();
        let legacy_hash = calc_text_hash("hello") as i64;
        conn.execute(
            "INSERT INTO clipboard_history (id, content_type, content, content_hash)
             VALUES (1, 'text', 'hello', ?1),
                    (2, 'text', 'hello ', ?1),
                    (3, 'file', 'C:/hello.txt', ?1),
                    (4, 'file', 'C:/hello.txt ', ?1),
                    (5, 'video', 'C:/hello.mp4', ?1),
                    (6, 'video', 'C:/hello.mp4\n', ?1)",
            [legacy_hash],
        )
        .expect("insert legacy rows");
        conn.execute(
            "INSERT INTO cloud_sync_local_index (sync_key, digest) VALUES ('text:legacy', 'old')",
            [],
        )
        .expect("insert stale index");
        conn.execute(
            "INSERT INTO cloud_sync_tombstones (content_type, content_hash, deleted_at)
             VALUES ('text', ?1, 99)",
            [legacy_hash],
        )
        .expect("insert legacy tombstone");

        run_migrations(&conn).expect("run migration 14");

        let plain_hash: i64 = conn
            .query_row(
                "SELECT content_hash FROM clipboard_history WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read plain hash");
        let spaced_hash: i64 = conn
            .query_row(
                "SELECT content_hash FROM clipboard_history WHERE id = 2",
                [],
                |row| row.get(0),
            )
            .expect("read spaced hash");
        let (plain_version, spaced_version): (i64, i64) = conn
            .query_row(
                "SELECT
                    (SELECT content_hash_version FROM clipboard_history WHERE id = 1),
                    (SELECT content_hash_version FROM clipboard_history WHERE id = 2)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read live hash versions");
        let tombstone_version: i64 = conn
            .query_row(
                "SELECT hash_version FROM cloud_sync_tombstones WHERE content_type = 'text'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated tombstone version");
        let index_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cloud_sync_local_index", [], |row| {
                row.get(0)
            })
            .expect("count sync index");
        assert_eq!(plain_hash, calc_text_hash("hello") as i64);
        assert_eq!(spaced_hash, calc_text_hash("hello ") as i64);
        assert_ne!(plain_hash, spaced_hash);
        assert_eq!((plain_version, spaced_version), (2, 2));
        let migrated_path_hashes: Vec<(i64, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT content_hash, content_hash_version
                     FROM clipboard_history WHERE id BETWEEN 3 AND 6 ORDER BY id",
                )
                .expect("prepare path hash query");
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("query path hashes")
                .collect::<Result<_, _>>()
                .expect("read path hashes")
        };
        assert_eq!(
            migrated_path_hashes,
            vec![
                (calc_text_hash("C:/hello.txt") as i64, 2),
                (calc_text_hash("C:/hello.txt ") as i64, 2),
                (calc_text_hash("C:/hello.mp4") as i64, 2),
                (calc_text_hash("C:/hello.mp4\n") as i64, 2),
            ]
        );
        assert_ne!(migrated_path_hashes[0].0, migrated_path_hashes[1].0);
        assert_ne!(migrated_path_hashes[2].0, migrated_path_hashes[3].0);
        assert_eq!(tombstone_version, 1);
        assert_eq!(index_count, 0);
    }

    #[cfg(windows)]
    #[test]
    fn migration_14_hashes_decrypted_sensitive_content() {
        let conn = setup_version_11_hash_schema();
        let encrypted = crate::infrastructure::encryption::encrypt_value("hello ")
            .expect("encrypt migration fixture");
        conn.execute(
            "INSERT INTO clipboard_history (id, content_type, content, content_hash)
             VALUES (1, 'text', ?1, ?2)",
            rusqlite::params![encrypted, calc_text_hash("hello") as i64],
        )
        .expect("insert encrypted legacy row");

        run_migrations(&conn).expect("run migration 14");

        let migrated_hash: i64 = conn
            .query_row(
                "SELECT content_hash FROM clipboard_history WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read encrypted row hash");
        assert_eq!(migrated_hash, calc_text_hash("hello ") as i64);
    }

    #[test]
    fn migration_14_live_hash_is_the_key_used_by_subsequent_deletion() {
        let conn = setup_version_11_hash_schema();
        let legacy_hash = calc_text_hash("hello") as i64;
        conn.execute(
            "INSERT INTO clipboard_history (id, content_type, content, content_hash)
             VALUES (1, 'text', 'hello ', ?1)",
            [legacy_hash],
        )
        .expect("insert legacy row");

        run_migrations(&conn).expect("run migration 14");
        let (content_type, live_hash): (String, i64) = conn
            .query_row(
                "SELECT content_type, content_hash FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read migrated live key");
        conn.execute(
            "INSERT INTO cloud_sync_tombstones (content_type, content_hash, deleted_at)
             VALUES (?1, ?2, 123)",
            rusqlite::params![content_type, live_hash],
        )
        .expect("record canonical deletion");
        conn.execute("DELETE FROM clipboard_history WHERE id = 1", [])
            .expect("delete live row");

        let deletion_hash: i64 = conn
            .query_row(
                "SELECT content_hash FROM cloud_sync_tombstones WHERE content_type = 'text'",
                [],
                |row| row.get(0),
            )
            .expect("read deletion key");
        assert_eq!(live_hash, calc_text_hash("hello ") as i64);
        assert_eq!(deletion_hash, live_hash);
    }

    #[test]
    fn legacy_local_migration_12_is_reconciled_with_upstream_migrations() {
        let conn = Connection::open_in_memory().expect("open migration test db");
        conn.execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_migrations (version) VALUES (12);
             CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                content_hash_version INTEGER NOT NULL,
                timestamp INTEGER NOT NULL DEFAULT 0,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                pinned_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                hash_version INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash, hash_version)
             );
             INSERT INTO cloud_sync_tombstones
                (content_type, content_hash, hash_version, deleted_at)
             VALUES ('text', 42, 2, 99);",
        )
        .expect("create legacy local migration 12 schema");

        run_migrations(&conn).expect("reconcile divergent migration 12");

        let analysis_table_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'table' AND name = 'clipboard_image_analysis'
                )",
                [],
                |row| row.get(0),
            )
            .expect("check repaired image analysis table");
        let tombstone: (i64, i64, i64) = conn
            .query_row(
                "SELECT content_hash, hash_version, deleted_at
                 FROM cloud_sync_tombstones WHERE content_type = 'text'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read preserved versioned tombstone");
        let latest_version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read reconciled migration version");

        assert!(analysis_table_exists);
        assert_eq!(tombstone, (42, 2, 99));
        assert_eq!(latest_version, 14);
    }
}
