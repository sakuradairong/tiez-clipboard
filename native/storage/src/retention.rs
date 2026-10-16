//! Transaction-owned history retention with attachment cleanup deferred to commit.
use crate::shared_repository::clipboard_repo::SqliteClipboardRepository;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub struct CleanupPlan {
    removed_ids: Vec<i64>,
    paths: Vec<PathBuf>,
}
impl CleanupPlan {
    pub fn removed_ids(&self) -> &[i64] {
        &self.removed_ids
    }
    /// Include removed session attachments while protecting surviving in-memory references.
    /// This only builds a plan; it never touches files before the owner commits.
    pub fn include_session<'a>(
        &mut self,
        repository: &SqliteClipboardRepository,
        directory: &Path,
        removed: impl IntoIterator<Item = &'a crate::domain::models::ClipboardEntry>,
        retained: impl IntoIterator<Item = &'a crate::domain::models::ClipboardEntry>,
    ) {
        let attachments = directory.join("attachments");
        let collect = |entry: &crate::domain::models::ClipboardEntry| {
            repository.collect_attachment_paths_for_cleanup(
                &entry.content,
                entry.html_content.as_deref(),
                entry.is_external,
                &attachments,
            )
        };
        self.paths.extend(removed.into_iter().flat_map(&collect));
        let protected: std::collections::HashSet<_> =
            retained.into_iter().flat_map(collect).collect();
        self.paths.retain(|path| !protected.contains(path));
    }
    /// Must be called after the owner's transaction committed on this connection.
    pub fn finish(
        self,
        repository: &SqliteClipboardRepository,
        connection: &Connection,
        directory: &Path,
    ) -> Result<Vec<i64>, String> {
        if !connection.is_autocommit() {
            return Err("Attachment cleanup requires a committed transaction".into());
        }
        repository.cleanup_unreferenced_attachment_paths_with_conn(
            connection,
            self.paths,
            &directory.join("attachments"),
        );
        Ok(self.removed_ids)
    }
}

/// The owner rolls this database work back if saving, encryption or pruning fails.
/// Dropping an uncommitted plan never deletes filesystem data.
pub fn prune(
    repository: &SqliteClipboardRepository,
    connection: &Connection,
    directory: &Path,
) -> Result<CleanupPlan, String> {
    if connection.is_autocommit() {
        return Err("Retention requires an owner transaction".into());
    }
    let removed_ids = repository.limit_candidates_with_conn(connection)?;
    delete_selected(repository, connection, directory, removed_ids)
}

/// Prepares explicit deletions in the owner's transaction; filesystem cleanup is
/// deferred until CleanupPlan::finish after commit, just as for retention.
pub fn delete_selected(
    repository: &SqliteClipboardRepository,
    connection: &Connection,
    directory: &Path,
    removed_ids: Vec<i64>,
) -> Result<CleanupPlan, String> {
    if connection.is_autocommit() {
        return Err("Deletion requires an owner transaction".into());
    }
    let mut paths = Vec::new();
    for id in &removed_ids {
        if let Some(entry) = repository.get_entry_by_id_with_conn(connection, *id)? {
            paths.extend(repository.collect_attachment_paths_for_cleanup(
                &entry.content,
                entry.html_content.as_deref(),
                entry.is_external,
                &directory.join("attachments"),
            ));
        }
    }
    for id in &removed_ids {
        repository.delete_with_conn(connection, *id, None)?;
    }
    Ok(CleanupPlan { removed_ids, paths })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_repository::migrations::run_migrations;
    use std::sync::{Arc, Mutex};
    fn fixture() -> (
        Arc<Mutex<Connection>>,
        SqliteClipboardRepository,
        PathBuf,
        PathBuf,
    ) {
        let root = std::env::temp_dir().join(format!(
            "TieZ-retention-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("attachments")).unwrap();
        let file = root.join("attachments/owned.png");
        std::fs::write(&file, b"fixture image").unwrap();
        let c = Connection::open_in_memory().unwrap();
        run_migrations(&c).unwrap();
        crate::database::seed_defaults(&c).unwrap();
        c.execute(
            "UPDATE settings SET value='1' WHERE key='app.persistent_limit'",
            [],
        )
        .unwrap();
        for id in [1, 2] {
            c.execute("INSERT INTO clipboard_history (id,content_type,content,source_app,timestamp,preview,content_hash,content_hash_version,tags,is_external) VALUES (?1,'image',?2,'fixture',?1,'image',?1,2,'[]',1)", rusqlite::params![id, if id == 1 { file.to_string_lossy().into_owned() } else { "C:/outside-fixture.png".into() }]).unwrap();
        }
        let c = Arc::new(Mutex::new(c));
        let repo = SqliteClipboardRepository::new(Arc::clone(&c));
        (c, repo, root, file)
    }
    #[test]
    fn prune_rollback_keeps_rows_and_attachment_until_commit() {
        let (c, repo, root, file) = fixture();
        {
            let mut c = c.lock().unwrap();
            let tx = c.transaction().unwrap();
            let plan = prune(&repo, &tx, &root).unwrap();
            assert_eq!(plan.removed_ids(), &[1]);
            assert!(file.exists());
            drop(plan);
            tx.rollback().unwrap();
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM clipboard_history", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert!(file.exists());
            let tx = c.transaction().unwrap();
            let plan = prune(&repo, &tx, &root).unwrap();
            tx.commit().unwrap();
            assert!(file.exists());
            assert_eq!(plan.finish(&repo, &c, &root).unwrap(), vec![1]);
            assert!(!file.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn protected_shared_attachment_survives_limit_cleanup() {
        let (c, repo, root, file) = fixture();
        {
            let mut c = c.lock().unwrap();
            c.execute(
                "UPDATE settings SET value='0' WHERE key='app.persistent_limit'",
                [],
            )
            .unwrap();
            c.execute(
                "UPDATE clipboard_history SET content=?1,is_pinned=1 WHERE id=2",
                [file.to_string_lossy().as_ref()],
            )
            .unwrap();
            let tx = c.transaction().unwrap();
            let plan = prune(&repo, &tx, &root).unwrap();
            tx.commit().unwrap();
            assert_eq!(plan.finish(&repo, &c, &root).unwrap(), vec![1]);
            assert!(file.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn later_delete_failure_rolls_back_all_evictions_and_files() {
        let (c, repo, root, file) = fixture();
        {
            let mut c = c.lock().unwrap();
            c.execute(
                "UPDATE settings SET value='0' WHERE key='app.persistent_limit'",
                [],
            )
            .unwrap();
            c.execute_batch("CREATE TRIGGER reject_later_delete BEFORE DELETE ON clipboard_history WHEN OLD.id=2 BEGIN SELECT RAISE(ABORT,'synthetic retention failure'); END").unwrap();
            let tx = c.transaction().unwrap();
            assert!(prune(&repo, &tx, &root).is_err());
            tx.rollback().unwrap();
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM clipboard_history", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert!(file.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
