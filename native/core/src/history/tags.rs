use super::{failure, HistoryService};
use crate::{CoreError, CoreResult};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tiez_storage::database::has_sensitive_tag;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TagQuery {
    pub limit: u32,
    pub after: Option<String>,
    pub expected_revision: Option<u64>,
}
impl Default for TagQuery {
    fn default() -> Self {
        Self {
            limit: 100,
            after: None,
            expected_revision: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct TagInfo {
    pub name: String,
    pub stored_count: i64,
    pub session_count: usize,
    pub color: Option<String>,
    pub is_sensitive: bool,
}
#[derive(Debug, Serialize)]
pub struct TagPage {
    pub revision: u64,
    pub items: Vec<TagInfo>,
    pub next_after: Option<String>,
    pub has_more: bool,
}
#[derive(Debug, Serialize)]
pub struct TagMutation {
    pub revision: u64,
    pub changed: bool,
    pub persistent_sync_pending: bool,
}
impl HistoryService {
    pub fn tag_page(&self, query: TagQuery) -> CoreResult<TagPage> {
        if (query.after.is_some() && query.expected_revision.is_none())
            || query
                .expected_revision
                .is_some_and(|revision| revision != self.revision)
        {
            return Err(CoreError::new(
                "history_conflict",
                "Tags changed; refresh before continuing",
            ));
        }
        let limit = query.limit.clamp(1, 200) as usize;
        let connection = self.connection.lock().map_err(failure)?;
        let mut statement = connection.prepare(
            "WITH names AS (SELECT name FROM saved_tags UNION SELECT tag AS name FROM entry_tags)
             SELECT name, (SELECT COUNT(*) FROM entry_tags WHERE tag = names.name),
                 (SELECT NULLIF(color, '') FROM saved_tags WHERE name = names.name)
             FROM names WHERE (?1 IS NULL OR name > ?1) ORDER BY name COLLATE BINARY LIMIT ?2"
        ).map_err(failure)?;
        let rows = statement
            .query_map(params![query.after, limit as i64 + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(failure)?;
        let mut tags = BTreeMap::<String, TagInfo>::new();
        for row in rows {
            let (name, stored_count, color) = row.map_err(failure)?;
            tags.insert(
                name.clone(),
                TagInfo {
                    is_sensitive: has_sensitive_tag(std::slice::from_ref(&name)),
                    name,
                    stored_count,
                    session_count: 0,
                    color,
                },
            );
        }
        // Session history is bounded to 500 rows. Deduplicate a row's tags when counting.
        // A session name outside the DB page cannot precede that page's limit+1 DB names,
        // so merging here preserves counts for every item that can enter the returned page.
        for entry in &self.session {
            for name in entry.tags.iter().collect::<BTreeSet<_>>() {
                if query.after.as_ref().is_some_and(|after| name <= after) {
                    continue;
                }
                tags.entry(name.clone())
                    .or_insert_with(|| TagInfo {
                        is_sensitive: has_sensitive_tag(std::slice::from_ref(name)),
                        name: name.clone(),
                        stored_count: 0,
                        session_count: 0,
                        color: None,
                    })
                    .session_count += 1;
            }
        }
        let has_more = tags.len() > limit;
        let items: Vec<_> = tags.into_values().take(limit).collect();
        let next_after = if has_more {
            items.last().map(|tag| tag.name.clone())
        } else {
            None
        };
        Ok(TagPage {
            revision: self.revision,
            items,
            next_after,
            has_more,
        })
    }
    fn tag_revision(&self, expected_revision: u64) -> CoreResult<()> {
        if expected_revision != self.revision {
            return Err(CoreError::new(
                "history_conflict",
                "Tags or history changed; review the tag again",
            ));
        }
        Ok(())
    }
    fn tag_name(name: &str) -> CoreResult<()> {
        if name.trim().is_empty() {
            return Err(CoreError::new("invalid_tag", "Tag name must not be empty"));
        }
        Ok(())
    }
    fn tag_unchanged(&self) -> TagMutation {
        TagMutation {
            revision: self.revision,
            changed: false,
            persistent_sync_pending: self.persistent_sync_pending,
        }
    }
    fn tag_committed(&mut self, revision: u64) -> TagMutation {
        self.revision = revision;
        self.persistent_sync_pending = true;
        // Catalog metadata has no clipboard ID. Reuse the existing full-resync contract
        // rather than inventing an entry ID or advancing only one window's state.
        self.changes.clear();
        TagMutation {
            revision,
            changed: true,
            persistent_sync_pending: true,
        }
    }
    pub fn create_tag(&mut self, name: String, expected_revision: u64) -> CoreResult<TagMutation> {
        self.tag_revision(expected_revision)?;
        Self::tag_name(&name)?;
        let revision = {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            let inserted = transaction
                .execute("INSERT OR IGNORE INTO saved_tags(name) VALUES (?1)", [name])
                .map_err(failure)?;
            if inserted == 0 {
                transaction.rollback().map_err(failure)?;
                return Ok(self.tag_unchanged());
            }
            let revision = self.next_revision()?;
            transaction.commit().map_err(failure)?;
            revision
        };
        Ok(self.tag_committed(revision))
    }
    pub fn set_tag_color(
        &mut self,
        name: String,
        color: Option<String>,
        expected_revision: u64,
    ) -> CoreResult<TagMutation> {
        self.tag_revision(expected_revision)?;
        Self::tag_name(&name)?;
        // Retain the legacy color string contract (including CSS colors) for migration.
        // Rendering/choosing a native color is a separate UI concern. Empty means unset.
        let color = color.filter(|value| !value.is_empty());
        let revision = {
            let mut connection = self.connection.lock().map_err(failure)?;
            let transaction = connection.transaction().map_err(failure)?;
            let existing: Option<Option<String>> = transaction
                .query_row(
                    "SELECT NULLIF(color, '') FROM saved_tags WHERE name = ?1",
                    [&name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(failure)?;
            if existing.flatten() == color {
                transaction.rollback().map_err(failure)?;
                return Ok(self.tag_unchanged());
            }
            let revision = self.next_revision()?;
            if let Some(color) = color {
                transaction
                    .execute(
                        "INSERT INTO saved_tags(name,color) VALUES (?1,?2)
                    ON CONFLICT(name) DO UPDATE SET color=excluded.color",
                        params![name, color],
                    )
                    .map_err(failure)?;
            } else {
                transaction
                    .execute("UPDATE saved_tags SET color=NULL WHERE name=?1", [name])
                    .map_err(failure)?;
            }
            transaction.commit().map_err(failure)?;
            revision
        };
        Ok(self.tag_committed(revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tiez_storage::domain::models::ClipboardEntry;
    fn service() -> HistoryService {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
        HistoryService::from_connection(connection, std::env::temp_dir())
    }
    #[test]
    fn catalog_merges_saved_persistent_and_session_tags_without_writes() {
        let mut service = service();
        let added = service
            .add_manual(
                "text".into(),
                "plain".into(),
                None,
                vec!["b".into(), "密码".into()],
            )
            .unwrap();
        service.create_tag("empty".into(), added.revision).unwrap();
        service
            .set_tag_color(
                "b".into(),
                Some("rgba(1, 2, 3, .5)".into()),
                service.revision,
            )
            .unwrap();
        let mut entry = service.entry(added.id).unwrap();
        entry.id = -42;
        entry.tags = vec!["a".into(), "b".into(), "b".into(), "session".into()];
        service.session.push_back(entry);
        let before = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap();
        let revision = service.revision;
        let mut after = None;
        let mut items = Vec::new();
        loop {
            let page = service
                .tag_page(TagQuery {
                    limit: 1,
                    after,
                    expected_revision: Some(revision),
                })
                .unwrap();
            assert_eq!(page.revision, revision);
            items.extend(page.items);
            if !page.has_more {
                assert!(page.next_after.is_none());
                break;
            }
            after = page.next_after;
        }
        assert_eq!(
            items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "empty", "sensitive", "session", "密码"]
        );
        assert_eq!((items[1].stored_count, items[1].session_count), (1, 1));
        assert_eq!(items[1].color.as_deref(), Some("rgba(1, 2, 3, .5)"));
        assert_eq!((items[2].stored_count, items[2].session_count), (0, 0));
        assert!(items[3].is_sensitive && items[5].is_sensitive);
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(service.revision, revision);
        assert_eq!(service.session[0].id, -42);
    }
    #[test]
    fn metadata_mutations_are_idempotent_and_invalidate_paged_readers() {
        let mut service = service();
        assert!(
            !service
                .set_tag_color("missing".into(), None, 0)
                .unwrap()
                .changed
        );
        assert!(
            service
                .create_tag("原生 ' %_ 标签".into(), 0)
                .unwrap()
                .changed
        );
        assert!(
            !service
                .create_tag("原生 ' %_ 标签".into(), 1)
                .unwrap()
                .changed
        );
        assert_eq!(
            service
                .tag_page(TagQuery {
                    after: Some("".into()),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
        assert!(
            service
                .set_tag_color("原生 ' %_ 标签".into(), Some("#ff8800".into()), 1)
                .unwrap()
                .changed
        );
        assert!(
            !service
                .set_tag_color("原生 ' %_ 标签".into(), Some("#ff8800".into()), 2)
                .unwrap()
                .changed
        );
        assert_eq!(
            service.create_tag("stale".into(), 1).unwrap_err().code,
            "history_conflict"
        );
        assert_eq!(
            service
                .tag_page(TagQuery {
                    expected_revision: Some(1),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
        assert_eq!(
            service.create_tag(" \n ".into(), 2).unwrap_err().code,
            "invalid_tag"
        );
        assert_eq!(
            service
                .set_tag_color("".into(), Some("red".into()), 2)
                .unwrap_err()
                .code,
            "invalid_tag"
        );
        assert!(
            service
                .set_tag_color("原生 ' %_ 标签".into(), None, 2)
                .unwrap()
                .changed
        );
        assert!(
            !service
                .set_tag_color("原生 ' %_ 标签".into(), Some("".into()), 3)
                .unwrap()
                .changed
        );
        let changes = service.changes(0).unwrap();
        assert!(
            changes.resync_required
                && changes.changes.is_empty()
                && changes.persistent_sync_pending
        );
        assert_eq!(service.revision, 3);
        assert!(service.tag_page(TagQuery::default()).unwrap().items[0]
            .color
            .is_none());
    }
    #[test]
    fn pages_are_bounded_and_reject_history_revision_changes() {
        let mut service = service();
        let initial_count = service.tag_page(TagQuery::default()).unwrap().items.len();
        for index in 0..240 {
            service
                .create_tag(format!("tag-{index:03}"), service.revision)
                .unwrap();
        }
        let page = service
            .tag_page(TagQuery {
                limit: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 200);
        assert!(page.has_more);
        let cursor = page.next_after;
        let page = service
            .tag_page(TagQuery {
                after: cursor.clone(),
                expected_revision: Some(service.revision),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 40 + initial_count);
        assert!(!page.has_more);
        service
            .add_manual("text".into(), "new".into(), None, vec![])
            .unwrap();
        assert_eq!(
            service
                .tag_page(TagQuery {
                    after: cursor,
                    expected_revision: Some(240),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
    }
    #[test]
    fn statement_and_commit_failure_preserve_metadata_revision_and_feed() {
        let mut service = service();
        let original =
            serde_json::to_value(service.tag_page(TagQuery::default()).unwrap()).unwrap();
        service.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_tag BEFORE INSERT ON saved_tags BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(service.create_tag("nope".into(), 0).is_err());
        assert_eq!(service.revision, 0);
        assert!(!service.persistent_sync_pending);
        service.connection.lock().unwrap().execute_batch("DROP TRIGGER fail_tag;
            CREATE TABLE commit_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE commit_child(id INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_tag_commit AFTER INSERT ON saved_tags BEGIN INSERT INTO commit_child VALUES (7); END;").unwrap();
        assert!(service
            .set_tag_color("nope".into(), Some("red".into()), 0)
            .is_err());
        assert_eq!(service.revision, 0);
        assert!(!service.persistent_sync_pending);
        assert_eq!(
            serde_json::to_value(service.tag_page(TagQuery::default()).unwrap()).unwrap(),
            original
        );
        assert!(!service.changes(0).unwrap().resync_required);
        service
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_tag_commit;")
            .unwrap();
        service
            .set_tag_color("yes".into(), Some("red".into()), 0)
            .unwrap();
        service.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_color BEFORE UPDATE ON saved_tags BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(service
            .set_tag_color("yes".into(), Some("blue".into()), 1)
            .is_err());
        assert_eq!(service.revision, 1);
        assert_eq!(
            service
                .tag_page(TagQuery::default())
                .unwrap()
                .items
                .into_iter()
                .find(|tag| tag.name == "yes")
                .unwrap()
                .color
                .as_deref(),
            Some("red")
        );
    }
    #[test]
    fn catalog_keeps_session_identity_and_existing_legacy_names() {
        let mut service = service();
        // Loading old data must not trim, recase, hide or migrate names/colors.
        service
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO saved_tags(name,color) VALUES (?1,?2)",
                params!["  Legacy  ", "navy"],
            )
            .unwrap();
        service.session.push_back(ClipboardEntry {
            id: -1,
            content_type: "text".into(),
            content: " text ".into(),
            html_content: None,
            source_app: "".into(),
            source_app_path: None,
            timestamp: 0,
            preview: "".into(),
            is_pinned: false,
            tags: vec!["PASSWORD".into(), "password".into()],
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: false,
        });
        let page = service.tag_page(TagQuery::default()).unwrap();
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.items[0].name, "  Legacy  ");
        assert_eq!(page.items[0].color.as_deref(), Some("navy"));
        assert!(page.items[1].is_sensitive && page.items[2].is_sensitive);
        assert_eq!(service.session[0].content, " text ");
        assert_eq!(service.session[0].id, -1);
        assert_eq!(service.revision, 0);
    }
}
