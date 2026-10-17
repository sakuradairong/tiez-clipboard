use super::*;
use rusqlite::params;

#[derive(Serialize)]
struct SessionOrder {
    id: i64,
    timestamp: i64,
    use_count: i32,
    is_pinned: bool,
    pinned_order: i64,
}
impl HistoryService {
    /// Public page() validates limit, scan-offset bounds and the accepted revision first.
    /// Sort small metadata projections globally, then hydrate only a bounded candidate batch.
    pub(super) fn sorted_page(&self, query: HistoryQuery) -> CoreResult<HistoryPage> {
        let sessions: Vec<_> = self
            .session
            .iter()
            .map(|entry| SessionOrder {
                id: entry.id,
                timestamp: entry.timestamp,
                use_count: entry.use_count,
                is_pinned: entry.is_pinned,
                pinned_order: entry.pinned_order,
            })
            .collect();
        let sessions = serde_json::to_string(&sessions).map_err(failure)?;
        let mut items = Vec::new();
        let (mut offset, mut scanned, mut has_more) = (query.offset, 0, true);
        while items.len() < query.limit as usize && scanned < SCAN_LIMIT {
            let take = (SCAN_LIMIT - scanned).min(200);
            let ids = {
                let connection = self.connection.lock().map_err(failure)?;
                let mut statement = connection.prepare(
                    "WITH candidates AS (SELECT id,timestamp,use_count,is_pinned,pinned_order FROM clipboard_history UNION ALL
                        SELECT json_extract(value,'$.id'), json_extract(value,'$.timestamp'), json_extract(value,'$.use_count'), json_extract(value,'$.is_pinned'), json_extract(value,'$.pinned_order') FROM json_each(?1))
                     SELECT id FROM candidates
                     ORDER BY CASE WHEN ?2 THEN use_count ELSE timestamp END DESC, is_pinned DESC, pinned_order DESC, timestamp DESC, id DESC LIMIT ?3 OFFSET ?4"
                ).map_err(failure)?;
                let rows = statement
                    .query_map(
                        params![
                            sessions,
                            matches!(query.sort, HistorySort::Usage),
                            take,
                            offset
                        ],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(failure)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(failure)?
            };
            let length = ids.len() as u32;
            let mut consumed = 0;
            // Drop the connection lock before entry()/matches() reuse repository/OCR queries.
            for id in ids {
                offset += 1;
                scanned += 1;
                consumed += 1;
                let entry = self.entry(id)?;
                if self.matches(&entry, &query)? {
                    items.push(entry.into());
                    if items.len() == query.limit as usize {
                        break;
                    }
                }
            }
            // Unconsumed lookahead is intentionally not included in next_offset.
            has_more = consumed < length || length == take;
            if !has_more {
                break;
            }
        }
        Ok(HistoryPage {
            revision: self.revision,
            items,
            next_offset: offset,
            has_more,
            stored_count: self.repository.get_count().map_err(failure)?,
            session_count: self.session.len(),
        })
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
    fn add(
        service: &mut HistoryService,
        text: &str,
        timestamp: i64,
        uses: i32,
        pinned: bool,
        tags: Vec<String>,
    ) -> i64 {
        let id = service
            .add_manual("text".into(), text.into(), None, tags)
            .unwrap()
            .id;
        service
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE clipboard_history SET timestamp=?1,use_count=?2,is_pinned=?3 WHERE id=?4",
                params![timestamp, uses, pinned, id],
            )
            .unwrap();
        id
    }
    fn all(service: &HistoryService, sort: &str) -> Vec<i64> {
        let mut offset = 0;
        let mut ids = Vec::new();
        loop {
            let page = service
                .page(HistoryQuery {
                    sort: if sort == "time" {
                        HistorySort::Time
                    } else {
                        HistorySort::Usage
                    },
                    limit: 1,
                    offset,
                    expected_revision: Some(service.revision),
                    tag: Some("shared".into()),
                    ..Default::default()
                })
                .unwrap();
            assert!(page.items.len() <= 1);
            offset = page.next_offset;
            ids.extend(page.items.into_iter().map(|entry| entry.id));
            if !page.has_more {
                break;
            }
        }
        ids
    }
    #[test]
    fn time_and_usage_merge_sessions_before_paging_without_pinned_priority_or_writes() {
        let mut service = service();
        let first = add(
            &mut service,
            "  first\r\n ",
            100,
            30,
            true,
            vec!["shared".into()],
        );
        let second = add(
            &mut service,
            "secret original",
            200,
            5,
            false,
            vec!["shared".into(), "密码".into()],
        );
        let third = add(&mut service, "third", 300, 0, false, vec!["shared".into()]);
        add(
            &mut service,
            "not shared",
            500,
            1000,
            true,
            vec!["other".into()],
        );
        let mut entry = service.entry(first).unwrap();
        entry.id = -1;
        entry.timestamp = 350;
        entry.use_count = 6;
        entry.is_pinned = false;
        service.session.push_back(entry);
        let revision = service.revision;
        let writes: i64 = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        assert_eq!(all(&service, "time"), [-1, third, second, first]);
        assert_eq!(all(&service, "usage"), [first, -1, second, third]);
        let sensitive = service
            .page(HistoryQuery {
                sort: HistorySort::Usage,
                search: "original".into(),
                tag: Some("shared".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(sensitive.items.len(), 1);
        assert_eq!(sensitive.items[0].id, second);
        assert_eq!(
            service
                .preview_entry(second, revision, false)
                .unwrap_err()
                .code,
            "sensitive_confirmation"
        );
        assert_eq!(service.entry(second).unwrap().content, "secret original");
        assert_eq!(service.entry(first).unwrap().content, "  first\r\n ");
        assert_eq!(service.revision, revision);
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
            service
                .page(HistoryQuery {
                    sort: HistorySort::Time,
                    pinned_only: true,
                    tag: Some("shared".into()),
                    ..Default::default()
                })
                .unwrap()
                .items[0]
                .id,
            first
        );
    }
    #[test]
    fn sorted_paging_reaches_late_high_usage_and_breaks_equal_keys_by_id() {
        let mut service = service();
        let mut ids = Vec::new();
        for index in 0..205 {
            ids.push(add(
                &mut service,
                &format!("row {index}"),
                10,
                if index == 0 { 20 } else { 0 },
                false,
                vec![],
            ));
        }
        let mut offset = 0;
        let revision = service.revision;
        let mut read = Vec::new();
        loop {
            let page = service
                .page(HistoryQuery {
                    sort: HistorySort::Usage,
                    limit: 7,
                    offset,
                    expected_revision: Some(revision),
                    ..Default::default()
                })
                .unwrap();
            assert!(page.items.len() <= 7);
            offset = page.next_offset;
            read.extend(page.items.into_iter().map(|entry| entry.id));
            if !page.has_more {
                break;
            }
        }
        assert_eq!(read.len(), 205);
        assert_eq!(read.iter().collect::<BTreeSet<_>>().len(), 205);
        assert_eq!(&read[..3], &[ids[0], ids[204], ids[203]]);
        assert_eq!(
            service
                .page(HistoryQuery {
                    sort: HistorySort::Time,
                    limit: 1,
                    ..Default::default()
                })
                .unwrap()
                .items[0]
                .id,
            ids[204]
        );
        assert_eq!(
            service
                .page(HistoryQuery {
                    sort: HistorySort::Usage,
                    offset: 1,
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
        add(&mut service, "late", 20, 100, false, vec![]);
        assert_eq!(
            service
                .page(HistoryQuery {
                    sort: HistorySort::Time,
                    offset: 1,
                    expected_revision: Some(revision),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
        let query: HistoryQuery = serde_json::from_str("{}").unwrap();
        assert!(matches!(query.sort, HistorySort::Default));
        assert!(serde_json::from_str::<HistoryQuery>(r#"{"sort":"unknown"}"#).is_err());
    }
    #[test]
    fn filtered_sorted_scan_budget_advances_and_does_not_repeat_sessions() {
        let mut service = service();
        service.connection.lock().unwrap().execute_batch("WITH RECURSIVE rows(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM rows WHERE n<2501)
             INSERT INTO clipboard_history(id,content_type,content,source_app,timestamp,preview)
             SELECT n,'text',CASE WHEN n=1 THEN 'needle' ELSE 'other' END,'fixture',n,'fixture' FROM rows;").unwrap();
        let mut session = service.entry(1).unwrap();
        session.id = -1;
        session.content = "session only".into();
        session.timestamp = 9999;
        service.session.push_back(session);
        let first = service
            .page(HistoryQuery {
                sort: HistorySort::Time,
                limit: 1,
                search: "needle".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(first.items.is_empty() && first.has_more);
        assert_eq!(first.next_offset, SCAN_LIMIT);
        let next = service
            .page(HistoryQuery {
                sort: HistorySort::Time,
                limit: 1,
                search: "needle".into(),
                offset: first.next_offset,
                expected_revision: Some(first.revision),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.items[0].id, 1);
        let empty = service
            .page(HistoryQuery {
                sort: HistorySort::Usage,
                content_type: Some("image".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(empty.items.is_empty());
    }
    #[test]
    fn equal_sort_keys_preserve_legacy_pinned_priority_and_order() {
        let mut service = service();
        let old_pin = add(&mut service, "older pin", 20, 5, true, vec![]);
        let new_pin = add(&mut service, "newer pin", 20, 5, true, vec![]);
        let newest = add(&mut service, "newest ordinary", 30, 5, false, vec![]);
        let most_used = add(&mut service, "most used", 5, 7, false, vec![]);
        service.connection.lock().unwrap().execute("UPDATE clipboard_history SET pinned_order=CASE WHEN id=?1 THEN 100 ELSE 50 END WHERE is_pinned=1",[old_pin]).unwrap();
        let usage = service
            .page(HistoryQuery {
                sort: HistorySort::Usage,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            usage.items.iter().map(|row| row.id).collect::<Vec<_>>(),
            [most_used, old_pin, new_pin, newest]
        );
        let time = service
            .page(HistoryQuery {
                sort: HistorySort::Time,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            time.items.iter().map(|row| row.id).collect::<Vec<_>>(),
            [newest, old_pin, new_pin, most_used]
        );
    }
}
