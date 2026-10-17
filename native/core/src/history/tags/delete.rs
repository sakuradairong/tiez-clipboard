use super::{failure, HistoryService};
use crate::{CoreError, CoreResult};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TagDeletePreview {
    pub name: String,
    pub revision: u64,
    pub stored_count: i64,
    pub session_count: usize,
    pub saved_tag: bool,
}
#[derive(Debug, Serialize)]
pub struct TagDeleteResult {
    pub revision: u64,
    pub changed: bool,
    pub removed_count: usize,
    pub stored_count: usize,
    pub session_count: usize,
    pub persistent_sync_pending: bool,
    pub cleanup_error: Option<String>,
}
fn catalog_counts(connection: &Connection, name: &str) -> CoreResult<(i64, bool)> {
    connection.query_row(
        "SELECT (SELECT COUNT(*) FROM entry_tags WHERE tag=?1), EXISTS(SELECT 1 FROM saved_tags WHERE name=?1)",
        [name], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(failure)
}
impl HistoryService {
    pub fn tag_delete_preview(&self, name: String) -> CoreResult<TagDeletePreview> {
        let (stored_count, saved_tag) =
            catalog_counts(&*self.connection.lock().map_err(failure)?, &name)?;
        let session_count = self
            .session
            .iter()
            .filter(|entry| entry.tags.contains(&name))
            .count();
        if !saved_tag && stored_count == 0 && session_count == 0 {
            return Err(CoreError::new(
                "not_found",
                "Tag was not found; refresh the tag directory",
            ));
        }
        Ok(TagDeletePreview {
            name,
            revision: self.revision,
            stored_count,
            session_count,
            saved_tag,
        })
    }
    /// Legacy global tag deletion removes matching entries, including pinned/multi-tagged rows.
    /// Publish a bounded full-resync result instead of an arbitrarily large removed-ID array.
    pub fn delete_tag(
        &mut self,
        name: String,
        expected_revision: u64,
    ) -> CoreResult<TagDeleteResult> {
        self.tag_revision(expected_revision)?;
        let session_count = self
            .session
            .iter()
            .filter(|entry| entry.tags.contains(&name))
            .count();
        let (revision, stored_count, persisted, cleanup_error) = {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            let (_, saved_tag) = catalog_counts(&transaction, &name)?;
            let stored = {
                let mut statement = transaction
                    .prepare("SELECT entry_id FROM entry_tags WHERE tag=?1 ORDER BY entry_id")
                    .map_err(failure)?;
                let rows = statement
                    .query_map([&name], |row| row.get::<_, i64>(0))
                    .map_err(failure)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(failure)?
            };
            let stored_count = stored.len();
            if !saved_tag && stored_count == 0 && session_count == 0 {
                transaction.rollback().map_err(failure)?;
                return Ok(TagDeleteResult {
                    revision: self.revision,
                    changed: false,
                    removed_count: 0,
                    stored_count: 0,
                    session_count: 0,
                    persistent_sync_pending: self.persistent_sync_pending,
                    cleanup_error: None,
                });
            }
            let revision = self.next_revision()?;
            let mut cleanup = tiez_storage::retention::delete_selected(
                &self.repository,
                &transaction,
                &self.data_dir,
                stored,
            )
            .map_err(failure)?;
            cleanup.include_session(
                &self.repository,
                &self.data_dir,
                self.session
                    .iter()
                    .filter(|entry| entry.tags.contains(&name)),
                self.session
                    .iter()
                    .filter(|entry| !entry.tags.contains(&name)),
            );
            transaction
                .execute("DELETE FROM saved_tags WHERE name=?1", [&name])
                .map_err(failure)?;
            transaction.commit().map_err(failure)?;
            let cleanup_error = cleanup
                .finish(&self.repository, &connection, &self.data_dir)
                .err();
            (
                revision,
                stored_count,
                saved_tag || stored_count > 0,
                cleanup_error,
            )
        };
        self.session.retain(|entry| !entry.tags.contains(&name));
        self.revision = revision;
        self.persistent_sync_pending |= persisted;
        self.changes.clear();
        Ok(TagDeleteResult {
            revision,
            changed: true,
            removed_count: stored_count + session_count,
            stored_count,
            session_count,
            persistent_sync_pending: self.persistent_sync_pending,
            cleanup_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tiez_storage::domain::models::ClipboardEntry;
    fn service() -> HistoryService {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
        HistoryService::from_connection(connection, std::env::temp_dir())
    }
    fn stored(service: &mut HistoryService, text: &str, tags: &[&str]) -> i64 {
        service
            .add_manual(
                "text".into(),
                text.into(),
                None,
                tags.iter().map(|tag| tag.to_string()).collect(),
            )
            .unwrap()
            .id
    }
    fn session(service: &mut HistoryService, id: i64, tags: &[&str], content: &str, image: bool) {
        service.session.push_back(ClipboardEntry {
            id,
            content_type: if image { "image" } else { "text" }.into(),
            content: content.into(),
            html_content: None,
            source_app: "fixture".into(),
            source_app_path: None,
            timestamp: 1,
            preview: "preview".into(),
            is_pinned: true,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            use_count: 0,
            is_external: image,
            pinned_order: 0,
            file_preview_exists: false,
        });
    }
    #[test]
    fn delete_covers_tag_scope_pins_other_tags_sessions_and_tombstones() {
        let mut service = service();
        let first = stored(&mut service, "first", &["remove", "other"]);
        service.pin(first, true).unwrap();
        let second = stored(&mut service, "secret", &["remove", "密码"]);
        let kept = stored(&mut service, "  keep\r\n ", &["other"]);
        service
            .create_tag("remove".into(), service.revision)
            .unwrap();
        service
            .create_tag("other".into(), service.revision)
            .unwrap();
        session(
            &mut service,
            -5,
            &["remove", "other"],
            "removed session",
            false,
        );
        session(&mut service, -6, &["other"], "kept session", false);
        let before = service.revision;
        let writes: i64 = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        let preview = service.tag_delete_preview("remove".into()).unwrap();
        assert_eq!(
            (
                preview.stored_count,
                preview.session_count,
                preview.revision,
                preview.saved_tag
            ),
            (2, 1, before, true)
        );
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            writes
        );
        let result = service.delete_tag("remove".into(), before).unwrap();
        assert!(result.changed && result.persistent_sync_pending && result.cleanup_error.is_none());
        assert_eq!(
            (
                result.stored_count,
                result.session_count,
                result.removed_count,
                result.revision
            ),
            (2, 1, 3, before + 1)
        );
        for id in [first, second, -5] {
            assert_eq!(service.entry(id).unwrap_err().code, "not_found");
        }
        assert_eq!(service.entry(kept).unwrap().content, "  keep\r\n ");
        assert_eq!(service.entry(-6).unwrap().content, "kept session");
        assert!(service
            .tag_page(Default::default())
            .unwrap()
            .items
            .iter()
            .any(|tag| tag.name == "other"));
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            2
        );
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM entry_tags WHERE entry_id IN (?1,?2)",
                    params![first, second],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert!(service.changes(before).unwrap().resync_required);
        assert!(
            !service
                .delete_tag("remove".into(), result.revision)
                .unwrap()
                .changed
        );
        assert_eq!(
            service
                .tag_delete_preview("remove".into())
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            service
                .delete_tag("remove".into(), before)
                .unwrap_err()
                .code,
            "history_conflict"
        );
    }
    #[test]
    fn zero_count_and_session_only_deletion_keep_sync_semantics() {
        let mut service = service();
        session(&mut service, -1, &["session"], "  keep format\r\n ", false);
        let result = service.delete_tag("session".into(), 0).unwrap();
        assert!(result.changed && !result.persistent_sync_pending);
        assert_eq!((result.stored_count, result.session_count), (0, 1));
        service.create_tag("zero".into(), result.revision).unwrap();
        let preview = service.tag_delete_preview("zero".into()).unwrap();
        assert!(preview.saved_tag);
        assert_eq!(preview.stored_count, 0);
        let result = service.delete_tag("zero".into(), preview.revision).unwrap();
        assert!(result.changed && result.persistent_sync_pending);
        assert_eq!(result.removed_count, 0);
        assert!(service
            .tag_page(Default::default())
            .unwrap()
            .items
            .iter()
            .all(|tag| tag.name != "zero"));
    }
    #[test]
    fn failure_rolls_back_saved_tag_history_sessions_and_tombstones() {
        let mut service = service();
        let first = stored(&mut service, "first", &["remove"]);
        let second = stored(&mut service, "second", &["remove"]);
        service
            .create_tag("remove".into(), service.revision)
            .unwrap();
        session(&mut service, -1, &["remove"], "session", false);
        let before = service.revision;
        let catalog = serde_json::to_value(service.tag_page(Default::default()).unwrap()).unwrap();
        let feed = serde_json::to_value(service.changes(0).unwrap()).unwrap();
        service.connection.lock().unwrap().execute_batch(&format!("CREATE TRIGGER fail_tag_delete BEFORE DELETE ON clipboard_history WHEN OLD.id={second} BEGIN SELECT RAISE(ABORT,'fixture'); END;")).unwrap();
        assert!(service.delete_tag("remove".into(), before).is_err());
        service.connection.lock().unwrap().execute_batch("DROP TRIGGER fail_tag_delete;
            CREATE TABLE delete_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE delete_child(id INTEGER REFERENCES delete_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_tag_commit AFTER DELETE ON saved_tags BEGIN INSERT INTO delete_child VALUES(19); END;").unwrap();
        assert!(service.delete_tag("remove".into(), before).is_err());
        for id in [first, second, -1] {
            assert!(service.entry(id).is_ok());
        }
        assert_eq!(service.revision, before);
        assert_eq!(
            serde_json::to_value(service.tag_page(Default::default()).unwrap()).unwrap(),
            catalog
        );
        assert_eq!(
            serde_json::to_value(service.changes(0).unwrap()).unwrap(),
            feed
        );
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    }
    #[test]
    fn deleting_a_tag_is_not_limited_to_one_thousand_or_the_current_page() {
        let mut service = service();
        for index in 0..1003 {
            stored(&mut service, &format!("tag deletion {index}"), &["all"]);
        }
        let preview = service.tag_delete_preview("all".into()).unwrap();
        assert_eq!(preview.stored_count, 1003);
        let result = service.delete_tag("all".into(), preview.revision).unwrap();
        assert_eq!(result.removed_count, 1003);
        assert!(service.page(Default::default()).unwrap().items.is_empty());
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cloud_sync_tombstones", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1003
        );
    }
    #[test]
    fn attachments_wait_for_commit_and_protect_surviving_stored_and_session_refs() {
        let mut service = service();
        let directory = std::env::temp_dir().join(format!(
            "TieZ-tag-delete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(directory.join("attachments")).unwrap();
        service.data_dir = directory.clone();
        let shared = directory.join("attachments/shared.png");
        let doomed = directory.join("attachments/doomed.png");
        std::fs::write(&shared, b"fixture shared").unwrap();
        std::fs::write(&doomed, b"fixture doomed").unwrap();
        // Stored and retained session references share one file; removed session owns another.
        service.connection.lock().unwrap().execute("INSERT INTO clipboard_history(id,content_type,content,source_app,timestamp,preview,content_hash,content_hash_version,tags,is_external) VALUES(1,'image',?1,'fixture',1,'image',1,2,'[\"remove\"]',1)", [shared.to_str().unwrap()]).unwrap();
        service
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO entry_tags(entry_id,tag) VALUES(1,'remove')",
                [],
            )
            .unwrap();
        service.create_tag("remove".into(), 0).unwrap();
        session(
            &mut service,
            -1,
            &["remove"],
            doomed.to_str().unwrap(),
            true,
        );
        session(&mut service, -2, &["keep"], shared.to_str().unwrap(), true);
        service.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_saved_tag BEFORE DELETE ON saved_tags BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let before = service.revision;
        assert!(service.delete_tag("remove".into(), before).is_err());
        assert!(shared.exists() && doomed.exists());
        assert!(service.entry(1).is_ok() && service.entry(-1).is_ok());
        service
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_saved_tag;")
            .unwrap();
        let deleted = service.delete_tag("remove".into(), before).unwrap();
        assert_eq!(deleted.removed_count, 2);
        assert!(shared.exists());
        assert!(!doomed.exists());
        // A retained stored reference also protects a removed session's file.
        service.connection.lock().unwrap().execute("INSERT INTO clipboard_history(id,content_type,content,source_app,timestamp,preview,content_hash,content_hash_version,tags,is_external) VALUES(2,'image',?1,'fixture',2,'image',2,2,'[\"stored-keep\"]',1)", [shared.to_str().unwrap()]).unwrap();
        service
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO entry_tags(entry_id,tag) VALUES(2,'stored-keep')",
                [],
            )
            .unwrap();
        service.delete_tag("keep".into(), deleted.revision).unwrap();
        assert!(shared.exists());
        service
            .delete_tag("stored-keep".into(), service.revision)
            .unwrap();
        assert!(!shared.exists());
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
