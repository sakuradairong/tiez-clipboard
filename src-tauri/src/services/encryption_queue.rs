use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::database::{has_sensitive_tag, DbState};
use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
use rusqlite::{Connection, OptionalExtension};

#[derive(Clone, Copy, Debug)]
pub enum EncryptionAction {
    Encrypt,
    Decrypt,
}

#[derive(Clone, Copy, Debug)]
pub struct EncryptionJob {
    pub id: i64,
    pub action: EncryptionAction,
}

#[derive(Clone)]
pub struct EncryptionQueue {
    sender: Sender<EncryptionJob>,
}

impl EncryptionQueue {
    pub fn enqueue(&self, job: EncryptionJob) {
        let _ = self.sender.send(job);
    }
}

pub fn init_encryption_queue(app_handle: AppHandle) -> EncryptionQueue {
    let (tx, rx) = mpsc::channel::<EncryptionJob>();
    thread::spawn(move || worker(app_handle, rx));
    EncryptionQueue { sender: tx }
}

fn worker(app_handle: AppHandle, rx: Receiver<EncryptionJob>) {
    while let Ok(job) = rx.recv() {
        let mut jobs = vec![job];
        while let Ok(next) = rx.try_recv() {
            jobs.push(next);
        }

        for job in jobs {
            let db_state = app_handle.state::<DbState>();
            let conn = match db_state.conn.lock() {
                Ok(c) => c,
                Err(_) => {
                    thread::sleep(Duration::from_millis(30));
                    continue;
                }
            };

            let result = apply_current_encryption_job(&db_state.repo, &conn, job);

            if let Err(err) = result {
                eprintln!("encryption queue job failed (id={}): {}", job.id, err);
            }

            drop(conn);
            thread::sleep(Duration::from_millis(30));
        }
    }
}

fn apply_current_encryption_job(
    repo: &SqliteClipboardRepository,
    conn: &Connection,
    job: EncryptionJob,
) -> Result<(), String> {
    let tags: Option<String> = conn
        .query_row(
            "SELECT tags FROM clipboard_history WHERE id = ?",
            [job.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    let Some(tags) = tags else {
        return Ok(());
    };
    let tags: Vec<String> = serde_json::from_str(&tags).map_err(|err| err.to_string())?;
    let sensitive = has_sensitive_tag(&tags);
    match job.action {
        EncryptionAction::Encrypt if sensitive => repo.encrypt_entry_with_conn(conn, job.id),
        EncryptionAction::Decrypt if !sensitive => repo.decrypt_entry_with_conn(conn, job.id),
        // A later tag edit or global rename has superseded this queued action.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_current_encryption_job, EncryptionAction, EncryptionJob};
    use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    #[test]
    fn queued_actions_cannot_undo_a_later_sensitive_tag_rename() {
        let conn = Connection::open_in_memory().expect("open queued action fixture");
        crate::infrastructure::repository::migrations::run_migrations(&conn)
            .expect("migrate queued action fixture");
        conn.execute_batch(
            "INSERT INTO clipboard_history
                (id, content_type, content, source_app, timestamp, preview, tags)
             VALUES (1, 'text', 'dpapi:protected', 'test', 1, 'dpapi:protected', '[\"Password\"]'),
                    (2, 'text', 'ordinary', 'test', 1, 'ordinary', '[]');
             CREATE TRIGGER reject_stale_action BEFORE UPDATE OF content ON clipboard_history
             BEGIN SELECT RAISE(ABORT, 'stale encryption action ran'); END;",
        )
        .expect("seed queued action fixture");
        let shared = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(shared.clone());
        let conn = shared.lock().expect("lock queue fixture");
        apply_current_encryption_job(
            &repo,
            &conn,
            EncryptionJob {
                id: 1,
                action: EncryptionAction::Decrypt,
            },
        )
        .expect("discard obsolete decrypt");
        apply_current_encryption_job(
            &repo,
            &conn,
            EncryptionJob {
                id: 2,
                action: EncryptionAction::Encrypt,
            },
        )
        .expect("discard obsolete encrypt");
    }
}
