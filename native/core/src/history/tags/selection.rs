use super::*;
use crate::history::HistoryMutation;

#[derive(Debug, Serialize)]
pub struct EntryTagSelection {
    pub id: i64,
    pub revision: u64,
    pub tags: Vec<String>,
}
impl HistoryService {
    /// Metadata only: opening a tag picker never returns or decrypts entry content.
    pub fn tag_selection(&self, id: i64) -> CoreResult<EntryTagSelection> {
        let tags = if id < 0 {
            self.session
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(super::super::missing)?
                .tags
                .clone()
        } else {
            let connection = self.connection.lock().map_err(failure)?;
            let json: String = connection
                .query_row(
                    "SELECT tags FROM clipboard_history WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(failure)?
                .ok_or_else(super::super::missing)?;
            serde_json::from_str(&json).map_err(failure)?
        };
        Ok(EntryTagSelection {
            id,
            revision: self.revision,
            tags,
        })
    }
    /// Catalog choices retain exact legacy names. Free-input normalization belongs to
    /// the explicit new-name editor; the legacy set_tags API keeps its trim behavior.
    pub fn set_tags_exact(
        &mut self,
        id: i64,
        tags: Vec<String>,
        expected_revision: u64,
    ) -> CoreResult<HistoryMutation> {
        self.tag_revision(expected_revision)?;
        let entry = self.entry(id)?;
        let mut seen = BTreeSet::new();
        let tags: Vec<_> = tags
            .into_iter()
            .filter(|name| seen.insert(name.clone()))
            .collect();
        if entry.tags.iter().collect::<BTreeSet<_>>() == tags.iter().collect::<BTreeSet<_>>() {
            return Ok(self.unchanged(id));
        }
        for name in &tags {
            if name.trim().is_empty()
                && !entry.tags.contains(name)
                && !self.session.iter().any(|other| other.tags.contains(name))
            {
                let present:bool=self.connection.lock().map_err(failure)?.query_row(
                    "SELECT EXISTS(SELECT 1 FROM saved_tags WHERE name=?1 UNION ALL SELECT 1 FROM entry_tags WHERE tag=?1)",
                    [name],|row|row.get(0)).map_err(failure)?;
                if !present {
                    return Err(CoreError::new(
                        "invalid_tag",
                        "New tag names must not be empty",
                    ));
                }
            }
        }
        self.commit_entry_tags(entry, tags)
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
    fn add(service: &mut HistoryService) -> i64 {
        service
            .add_manual(
                "rich_text".into(),
                "  original\r\n ".into(),
                Some("<b>original</b>".into()),
                vec!["old".into()],
            )
            .unwrap()
            .id
    }
    #[test]
    fn exact_selection_preserves_legacy_names_order_and_readonly_semantic_noops() {
        let mut service = service();
        let id = add(&mut service);
        let before = service.tag_selection(id).unwrap();
        assert_eq!(before.tags, ["old"]);
        let serialized = serde_json::to_value(before).unwrap();
        assert_eq!(serialized.as_object().unwrap().len(), 3);
        assert!(serialized.get("content").is_none());
        let result = service
            .set_tags_exact(
                id,
                vec![" 目录 ".into(), "' %_".into(), " 目录 ".into()],
                service.revision,
            )
            .unwrap();
        assert!(result.changed);
        assert_eq!(service.tag_selection(id).unwrap().tags, [" 目录 ", "' %_"]);
        let writes: i64 = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        let read = service.tag_selection(id).unwrap();
        let noop = service
            .set_tags_exact(id, vec!["' %_".into(), " 目录 ".into()], read.revision)
            .unwrap();
        assert!(!noop.changed);
        assert_eq!(noop.revision, result.revision);
        assert_eq!(service.tag_selection(id).unwrap().tags, [" 目录 ", "' %_"]);
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            writes
        );
        assert_eq!(
            service.entry(id).unwrap().html_content.as_deref(),
            Some("<b>original</b>")
        );
        service
            .set_tags(id, vec![" ordinary ".into(), "ordinary".into()])
            .unwrap();
        assert_eq!(service.entry(id).unwrap().tags, ["ordinary"]);
    }
    #[test]
    fn selection_conflicts_missing_and_empty_names_leave_state_untouched() {
        let mut service = service();
        let id = add(&mut service);
        let before = service.tag_selection(id).unwrap();
        service.create_tag("new".into(), before.revision).unwrap();
        let revision = service.revision;
        assert_eq!(
            service
                .set_tags_exact(id, vec![], before.revision)
                .unwrap_err()
                .code,
            "history_conflict"
        );
        assert_eq!(
            service
                .set_tags_exact(id, vec![" ".into()], revision)
                .unwrap_err()
                .code,
            "invalid_tag"
        );
        assert_eq!(
            service
                .set_tags_exact(999, vec![], revision)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(service.tag_selection(999).unwrap_err().code, "not_found");
        assert_eq!(service.tag_selection(id).unwrap().tags, before.tags);
        assert_eq!(service.revision, revision);
        service
            .connection
            .lock()
            .unwrap()
            .execute("INSERT INTO saved_tags(name) VALUES(' ')", [])
            .unwrap();
        service
            .set_tags_exact(id, vec![" ".into()], revision)
            .unwrap();
        assert_eq!(service.tag_selection(id).unwrap().tags, [" "]);
    }
    #[test]
    fn session_selection_noop_and_failed_promotion_keep_negative_identity() {
        let mut service = service();
        service.session.push_back(ClipboardEntry {
            id: -7,
            content_type: "rich_text".into(),
            content: "  session\r\n ".into(),
            html_content: Some("<b>session</b>".into()),
            source_app: "fixture".into(),
            source_app_path: None,
            timestamp: 1,
            preview: "preview".into(),
            is_pinned: false,
            tags: vec!["old".into()],
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: false,
        });
        assert_eq!(service.tag_selection(-7).unwrap().revision, 0);
        assert!(
            !service
                .set_tags_exact(-7, vec!["old".into(), "old".into()], 0)
                .unwrap()
                .changed
        );
        service.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_selection BEFORE INSERT ON clipboard_history BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(service.set_tags_exact(-7, vec!["密码".into()], 0).is_err());
        assert_eq!(service.tag_selection(-7).unwrap().tags, ["old"]);
        assert_eq!(service.revision, 0);
        assert!(service.changes(0).unwrap().changes.is_empty());
        service
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_selection;")
            .unwrap();
        let result = service.set_tags_exact(-7, vec!["密码".into()], 0).unwrap();
        assert!(result.id > 0 && result.changed && result.persistent_sync_pending);
        assert_eq!(result.replaced_id, Some(-7));
        assert_eq!(service.tag_selection(-7).unwrap_err().code, "not_found");
        let entry = service.entry(result.id).unwrap();
        assert_eq!(entry.content, "  session\r\n ");
        assert_eq!(entry.html_content.as_deref(), Some("<b>session</b>"));
        assert_eq!(
            service
                .preview_entry(result.id, result.revision, false)
                .unwrap_err()
                .code,
            "sensitive_confirmation"
        );
    }
    #[test]
    fn exact_selection_accepts_legacy_empty_names_from_other_session_entries() {
        let mut service = service();
        let id = add(&mut service);
        let mut session = service.entry(id).unwrap();
        session.id = -17;
        session.tags = vec![" \t ".into(), "".into()];
        service.session.push_back(session);
        let snapshot = service.tag_selection(id).unwrap();
        let catalog = service.tag_page(Default::default()).unwrap();
        assert!(catalog
            .items
            .iter()
            .any(|tag| tag.name == " \t " && tag.session_count == 1));
        let saved = service
            .set_tags_exact(id, vec![" \t ".into(), "".into()], snapshot.revision)
            .unwrap();
        assert!(saved.changed);
        assert_eq!(service.tag_selection(id).unwrap().tags, [" \t ", ""]);
        assert_eq!(service.tag_selection(-17).unwrap().tags, [" \t ", ""]);
        assert_eq!(service.session.len(), 1);
        service.session.clear();
        let again = service
            .set_tags_exact(id, vec!["".into(), " \t ".into()], saved.revision)
            .unwrap();
        assert!(!again.changed);
        assert_eq!(again.revision, saved.revision);
        assert_eq!(service.tag_selection(id).unwrap().tags, [" \t ", ""]);
    }
    #[test]
    fn exact_sensitive_transitions_and_failed_sql_or_commit_are_atomic() {
        let mut service = service();
        let id = add(&mut service);
        service.connection.lock().unwrap().execute("INSERT INTO clipboard_image_analysis(entry_id,content_hash,ocr_text,analyzed_at) SELECT id,content_hash,'private OCR',1 FROM clipboard_history WHERE id=?1",[id]).unwrap();
        service
            .set_tags_exact(id, vec!["PASSWORD".into()], service.revision)
            .unwrap();
        let read = service.tag_selection(id).unwrap();
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM clipboard_image_analysis", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            service
                .preview_entry(id, read.revision, false)
                .unwrap_err()
                .code,
            "sensitive_confirmation"
        );
        let raw: String = service
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT content FROM clipboard_history WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        #[cfg(all(windows, not(feature = "portable")))]
        assert!(raw.starts_with("dpapi:"));
        let feed = serde_json::to_value(service.changes(0).unwrap()).unwrap();
        service.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_exact_delete BEFORE DELETE ON entry_tags BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(service
            .set_tags_exact(id, vec!["ordinary".into()], read.revision)
            .is_err());
        service.connection.lock().unwrap().execute_batch("DROP TRIGGER fail_exact_delete;
            CREATE TABLE selection_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE selection_child(id INTEGER REFERENCES selection_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_exact_commit AFTER UPDATE ON clipboard_history BEGIN INSERT INTO selection_child VALUES(999); END;").unwrap();
        assert!(service
            .set_tags_exact(id, vec!["ordinary".into()], read.revision)
            .is_err());
        assert_eq!(service.tag_selection(id).unwrap().tags, ["PASSWORD"]);
        assert_eq!(service.revision, read.revision);
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT content FROM clipboard_history WHERE id=?1",
                    [id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            raw
        );
        assert_eq!(
            serde_json::to_value(service.changes(0).unwrap()).unwrap(),
            feed
        );
        service
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_exact_commit;")
            .unwrap();
        let result = service
            .set_tags_exact(id, vec!["ordinary".into()], read.revision)
            .unwrap();
        assert_eq!(
            service
                .preview_entry(id, result.revision, false)
                .unwrap()
                .content,
            "  original\r\n "
        );
    }
}
