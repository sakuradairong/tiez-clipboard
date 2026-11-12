use super::*;

fn history() -> HistoryService {
    let connection = Connection::open_in_memory().unwrap();
    tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
    HistoryService::from_connection(connection, std::env::temp_dir())
}
fn add(history: &mut HistoryService, content: &str) -> i64 {
    history.add_manual("text".into(), content.into(), None, vec![]).unwrap().id
}
fn stored(history: &HistoryService, id: i64) -> (String, i32, i64, i64) {
    history.connection.lock().unwrap().query_row(
        "SELECT content,use_count,timestamp,sync_updated_at FROM clipboard_history WHERE id=?1",
        [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap()
}

#[test]
fn changed_edited_private_promoted_and_reused_ids_are_never_deleted_or_counted() {
    for delete in [false, true] {
        for mutation in 0..4 {
            let mut history = history();
            let stored_id = add(&mut history, "  原始\r\n  ");
            let id = if mutation == 3 {
                history.record_processed(history.entry(stored_id).unwrap(), false).unwrap().id
            } else { stored_id };
            let accepted = history.revision();
            let current = match mutation {
                0 => { history.edit(id, "changed".into()).unwrap(); id }
                1 => { history.set_tags(id, vec!["sensitive".into()]).unwrap(); id }
                2 => { history.delete(id).unwrap(); let next = add(&mut history, "replacement"); assert_eq!(id, next); next }
                _ => history.pin(id, true).unwrap().id,
            };
            let before = stored(&history, current);
            let revision = history.revision();
            history.persistent_sync_pending = false;
            assert_eq!(history.apply_after_paste(id, accepted, delete, true).unwrap_err().code, "history_conflict");
            assert_eq!(stored(&history, current), before);
            assert_eq!(history.revision(), revision);
            assert!(!history.persistent_sync_pending);
        }
    }
}

#[test]
fn unrelated_capture_is_allowed_but_missing_feed_or_future_revision_is_rejected() {
    let mut history = history();
    let id = add(&mut history, "kept");
    let accepted = history.revision();
    let other = add(&mut history, "new capture");
    history.apply_after_paste(id, accepted, false, false).unwrap();
    assert_eq!(history.entry(id).unwrap().use_count, 1);
    let accepted = history.revision();
    for i in 0..=CHANGE_LIMIT { history.edit(other, format!("new {i}")).unwrap(); }
    for revision in [accepted, history.revision() + 1] {
        assert_eq!(history.apply_after_paste(id, revision, true, false).unwrap_err().code, "history_conflict");
    }
    assert_eq!(history.entry(id).unwrap().use_count, 1);
}

#[test]
fn history_failures_roll_back_deletion_usage_timestamps_and_change_feed() {
    for delete in [false, true] {
        for commit_failure in [false, true] {
            let mut history = history();
            let id = add(&mut history, "  unchanged\r\n\t");
            let before = stored(&history, id);
            let revision = history.revision();
            history.persistent_sync_pending = false;
            let event = if delete { "DELETE" } else { "UPDATE OF use_count" };
            let action = if commit_failure {
                "INSERT INTO post_paste_guard VALUES('missing-parent');"
            } else { "SELECT RAISE(ABORT,'fixture');" };
            history.connection.lock().unwrap().execute_batch(&format!(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE post_paste_guard(key TEXT REFERENCES settings(key) DEFERRABLE INITIALLY DEFERRED);
                 CREATE TRIGGER post_paste_failure AFTER {event} ON clipboard_history BEGIN {action} END;"
            )).unwrap();
            assert!(history.apply_after_paste(id, revision, delete, true).is_err());
            assert_eq!(stored(&history, id), before);
            assert_eq!(history.revision(), revision);
            assert!(!history.persistent_sync_pending);
            assert!(history.changes(revision).unwrap().changes.is_empty());
        }
    }
}

#[test]
fn a_backward_clock_never_lowers_history_or_sync_timestamps() {
    let mut history = history();
    let id = add(&mut history, "unchanged");
    let future = now_ms() + 86_400_000;
    history.connection.lock().unwrap().execute(
        "UPDATE clipboard_history SET timestamp=?1,sync_updated_at=?1 WHERE id=?2", (future, id)).unwrap();
    history.apply_after_paste(id, history.revision(), false, true).unwrap();
    let (_, count, time, sync) = stored(&history, id);
    assert_eq!((count, time, sync), (1, future, future + 1));
}
