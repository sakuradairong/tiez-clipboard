//! Successful external-open accounting; no content re-save or session promotion.
use super::*;

impl HistoryService {
    /// Only the still-current logical entry may receive a post-dispatch use. The
    /// legacy SQLite row ID can be reused, so existence alone is insufficient.
    pub(crate) fn record_open_usage(
        &mut self,
        id: i64,
        accepted_revision: u64,
    ) -> CoreResult<HistoryMutation> {
        let changes = self.changes(accepted_revision)?;
        if changes.resync_required
            || changes.changes.iter().any(|change| {
                change.id == id
                    || change.id == 0
                    || change.replaced_id == Some(id)
                    || change.removed_ids.contains(&id)
            })
        {
            return Err(CoreError::new(
                "history_conflict",
                "Entry changed after external opening; usage was not recorded",
            ));
        }
        let entry = self.entry(id)?;
        let uses = entry.use_count.saturating_add(1);
        if uses == entry.use_count {
            return Ok(self.unchanged(id));
        }
        let revision = self.next_revision()?;
        if id < 0 {
            self.session
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(missing)?
                .use_count = uses;
        } else {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            let rows = transaction.execute(
                "UPDATE clipboard_history SET use_count=?1,
                 sync_updated_at=MAX(COALESCE(sync_updated_at,0)+1,?2),
                 sync_updated_by=COALESCE((SELECT value FROM settings WHERE key='app.anon_id'),'') WHERE id=?3",
                rusqlite::params![uses, now_ms(), id]).map_err(failure)?;
            if rows != 1 {
                return Err(missing());
            }
            transaction.commit().map_err(failure)?;
        }
        Ok(self.committed(revision, id, None, Vec::new(), id > 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> HistoryService {
        let connection = Connection::open_in_memory().unwrap();
        tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
        HistoryService::from_connection(connection, std::env::temp_dir())
    }
    fn add(service: &mut HistoryService, body: &str) -> i64 {
        service
            .add_manual("text".into(), body.into(), None, vec![])
            .unwrap()
            .id
    }
    fn stored(
        service: &HistoryService,
        id: i64,
    ) -> (String, String, Option<String>, i64, i64, String) {
        service.connection.lock().unwrap().query_row(
            "SELECT content,preview,html_content,timestamp,sync_updated_at,sync_updated_by FROM clipboard_history WHERE id=?1",
            [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).unwrap()
    }
    #[test]
    fn stored_usage_preserves_original_encrypted_fields_time_and_tags_and_publishes_after_commit() {
        let mut service = service();
        let body = "  private 原始\r\n\t  ";
        let id = service
            .add_manual(
                "rich_text".into(),
                body.into(),
                Some("<b>原始</b>".into()),
                vec!["sensitive".into()],
            )
            .unwrap()
            .id;
        service.connection.lock().unwrap().execute_batch("INSERT INTO settings(key,value) VALUES('app.anon_id','owned-device') ON CONFLICT(key) DO UPDATE SET value=excluded.value").unwrap();
        let before = stored(&service, id);
        let revision = service.revision;
        service.persistent_sync_pending = false;
        let result = service.record_open_usage(id, revision).unwrap();
        let after = stored(&service, id);
        assert_eq!(
            (&before.0, &before.1, &before.2, before.3),
            (&after.0, &after.1, &after.2, after.3)
        );
        assert!(after.4 > before.4);
        assert_eq!(after.5, "owned-device");
        let entry = service.entry(id).unwrap();
        assert_eq!(entry.content, body);
        assert_eq!(entry.use_count, 1);
        assert_eq!(entry.tags, ["sensitive"]);
        assert!(result.changed && result.persistent_sync_pending);
        assert_eq!(result.revision, revision + 1);
        let changes = service.changes(revision).unwrap();
        assert_eq!(changes.changes.len(), 1);
        assert_eq!(changes.changes[0].id, id);
        assert!(!changes.resync_required);
    }
    #[test]
    fn session_usage_never_writes_or_promotes_and_saturation_is_a_noop() {
        let mut service = service();
        let stored_id = add(&mut service, "session 原始\r\n ");
        let mut entry = service.entry(stored_id).unwrap();
        service.delete(stored_id).unwrap();
        entry.id = 0;
        let id = service.record_processed(entry, false).unwrap().id;
        let revision = service.revision;
        service.persistent_sync_pending = false;
        service
            .connection
            .lock()
            .unwrap()
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        let result = service.record_open_usage(id, revision).unwrap();
        assert!(id < 0 && result.changed && !result.persistent_sync_pending);
        assert_eq!(service.entry(id).unwrap().use_count, 1);
        assert_eq!(service.repository.get_count().unwrap(), 0);
        assert_eq!(service.entry(id).unwrap().content, "session 原始\r\n ");
        service
            .session
            .iter_mut()
            .find(|entry| entry.id == id)
            .unwrap()
            .use_count = i32::MAX;
        let revision = service.revision;
        assert!(!service.record_open_usage(id, revision).unwrap().changed);
        assert_eq!(service.revision, revision);
        assert!(service.changes(revision).unwrap().changes.is_empty());
    }
    #[test]
    fn statement_and_commit_failures_roll_back_count_stamps_feed_and_sync_flag() {
        for commit_failure in [false, true] {
            let mut service = service();
            let id = add(&mut service, "unchanged\r\n ");
            let before = stored(&service, id);
            let revision = service.revision;
            service.persistent_sync_pending = false;
            let sql = if commit_failure {
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE usage_guard(key TEXT REFERENCES settings(key) DEFERRABLE INITIALLY DEFERRED);
                 CREATE TRIGGER usage_failure AFTER UPDATE OF use_count ON clipboard_history BEGIN
                 INSERT INTO usage_guard VALUES('missing-usage-guard'); END;"
            } else {
                "CREATE TRIGGER usage_failure BEFORE UPDATE OF use_count ON clipboard_history BEGIN SELECT RAISE(ABORT,'fixture'); END;"
            };
            service
                .connection
                .lock()
                .unwrap()
                .execute_batch(sql)
                .unwrap();
            assert!(service.record_open_usage(id, revision).is_err());
            assert_eq!(service.revision, revision);
            assert_eq!(stored(&service, id), before);
            assert_eq!(service.entry(id).unwrap().use_count, 0);
            let changes = service.changes(revision).unwrap();
            assert!(changes.changes.is_empty() && !changes.persistent_sync_pending);
        }
    }
    #[test]
    fn target_mutation_promotion_and_deleted_reused_ids_never_receive_stale_usage() {
        for mutation in 0..3 {
            let mut service = service();
            let id = add(&mut service, "original");
            let revision = service.revision;
            match mutation {
                0 => {
                    service.edit(id, "modified".into()).unwrap();
                }
                1 => {
                    service.set_tags(id, vec!["sensitive".into()]).unwrap();
                }
                _ => {
                    service.delete(id).unwrap();
                    assert_eq!(add(&mut service, "replacement"), id);
                }
            }
            let changed_revision = service.revision;
            assert_eq!(
                service.record_open_usage(id, revision).unwrap_err().code,
                "history_conflict"
            );
            assert_eq!(service.entry(id).unwrap().use_count, 0);
            assert_eq!(service.revision, changed_revision);
        }
        let mut service = service();
        let stored_id = add(&mut service, "promoted");
        let mut entry = service.entry(stored_id).unwrap();
        entry.id = 0;
        let id = service.record_processed(entry, false).unwrap().id;
        let revision = service.revision;
        let promoted = service.pin(id, true).unwrap();
        assert!(promoted.id > 0);
        assert_eq!(
            service.record_open_usage(id, revision).unwrap_err().code,
            "history_conflict"
        );
        assert_eq!(service.entry(promoted.id).unwrap().use_count, 0);
    }
    #[test]
    fn unrelated_changes_allow_accounting_but_lost_feed_and_missing_ids_do_not() {
        let mut service = service();
        let id = add(&mut service, "opened");
        let other = add(&mut service, "other");
        let revision = service.revision;
        service.edit(other, "other changed".into()).unwrap();
        assert!(service.record_open_usage(id, revision).unwrap().changed);
        let revision = service.revision;
        for index in 0..=CHANGE_LIMIT {
            service.edit(other, format!("change {index}")).unwrap();
        }
        assert_eq!(
            service.record_open_usage(id, revision).unwrap_err().code,
            "history_conflict"
        );
        assert_eq!(service.entry(id).unwrap().use_count, 1);
        assert_eq!(
            service
                .record_open_usage(9999, service.revision)
                .unwrap_err()
                .code,
            "not_found"
        );
    }
}
