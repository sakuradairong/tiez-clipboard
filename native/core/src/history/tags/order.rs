use super::{failure, HistoryService, TagInfo, TagPage, TagQuery};
use crate::{CoreError, CoreResult};
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet};
use tiez_storage::database::has_sensitive_tag;

impl HistoryService {
    pub(super) fn tag_page_by_count(&self, query: TagQuery) -> CoreResult<TagPage> {
        // Session rows are bounded; count a repeated tag only once per row.
        let mut session_counts = BTreeMap::<&str, usize>::new();
        for entry in &self.session {
            for name in entry.tags.iter().collect::<BTreeSet<_>>() {
                *session_counts.entry(name).or_default() += 1;
            }
        }
        let connection = self.connection.lock().map_err(failure)?;
        if let Some(after) = &query.after {
            let stored: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM saved_tags WHERE name=?1 UNION ALL SELECT 1 FROM entry_tags WHERE tag=?1)",
                [after], |row| row.get(0),
            ).map_err(failure)?;
            if !stored && !session_counts.contains_key(after.as_str()) {
                return Err(CoreError::new(
                    "invalid_tag_cursor",
                    "The count-order cursor tag does not exist",
                ));
            }
        }
        let limit = query.limit.clamp(1, 200) as usize;
        let mut statement = connection.prepare(
            "WITH sessions AS (SELECT key AS name, value AS session_count FROM json_each(?1)),
             usage AS (SELECT tag AS name, COUNT(*) AS stored_count FROM entry_tags GROUP BY tag),
             names AS (SELECT name FROM saved_tags UNION SELECT name FROM usage UNION SELECT name FROM sessions),
             counts AS (SELECT names.name, COALESCE(usage.stored_count,0) AS stored_count,
                        COALESCE(sessions.session_count,0) AS session_count, NULLIF(saved_tags.color,'') AS color,
                        COALESCE(usage.stored_count,0) + COALESCE(sessions.session_count,0) AS total
                        FROM names LEFT JOIN usage USING(name) LEFT JOIN sessions USING(name) LEFT JOIN saved_tags USING(name)),
             cursor AS (SELECT total FROM counts WHERE name=?2)
             SELECT name,stored_count,session_count,color FROM counts
             WHERE (?2 IS NULL OR total < (SELECT total FROM cursor)
                    OR (total = (SELECT total FROM cursor) AND name > ?2 COLLATE BINARY))
             ORDER BY total DESC, name COLLATE BINARY LIMIT ?3"
        ).map_err(failure)?;
        let rows = statement
            .query_map(
                params![
                    serde_json::to_string(&session_counts).map_err(failure)?,
                    query.after,
                    limit as i64 + 1
                ],
                |row| {
                    let name: String = row.get(0)?;
                    Ok(TagInfo {
                        is_sensitive: has_sensitive_tag(std::slice::from_ref(&name)),
                        name,
                        stored_count: row.get(1)?,
                        session_count: row.get(2)?,
                        color: row.get(3)?,
                    })
                },
            )
            .map_err(failure)?;
        let mut items = rows.collect::<Result<Vec<_>, _>>().map_err(failure)?;
        let has_more = items.len() > limit;
        items.truncate(limit);
        let next_after = has_more.then(|| items.last().unwrap().name.clone());
        Ok(TagPage {
            revision: self.revision,
            items,
            next_after,
            has_more,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::TagSort;
    use super::*;
    use rusqlite::Connection;
    use tiez_storage::domain::models::ClipboardEntry;
    fn service() -> HistoryService {
        let connection = Connection::open_in_memory().unwrap();
        tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
        HistoryService::from_connection(connection, std::env::temp_dir())
    }
    fn add(service: &mut HistoryService, text: &str, tags: &[&str]) {
        service
            .add_manual(
                "text".into(),
                text.into(),
                None,
                tags.iter().map(|tag| tag.to_string()).collect(),
            )
            .unwrap();
    }
    #[test]
    fn count_sort_merges_session_totals_preserving_metadata_and_reads() {
        let mut service = service();
        add(&mut service, "one", &["a", "z"]);
        add(&mut service, "two", &["a"]);
        service
            .set_tag_color("z".into(), Some("var(--original)".into()), service.revision)
            .unwrap();
        service
            .create_tag("empty".into(), service.revision)
            .unwrap();
        let mut session: ClipboardEntry = service.entry(1).unwrap();
        session.id = -1;
        session.tags = vec!["z".into(), "z".into(), "session".into(), "密码".into()];
        service.session.push_back(session.clone());
        session.id = -2;
        service.session.push_back(session);
        let before = service.revision;
        let writes: i64 = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        let page = service
            .tag_page(TagQuery {
                sort: TagSort::Count,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            page.items
                .iter()
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>(),
            ["z", "a", "session", "密码", "empty", "sensitive"]
        );
        assert_eq!(
            (page.items[0].stored_count, page.items[0].session_count),
            (1, 2)
        );
        assert_eq!(page.items[0].color.as_deref(), Some("var(--original)"));
        assert!(page.items[3].is_sensitive);
        assert_eq!(service.entry(-1).unwrap().tags.len(), 4);
        assert_eq!(page.revision, before);
        assert_eq!(
            service
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            writes
        );
    }
    #[test]
    fn count_paging_orders_the_entire_catalog_and_rejects_stale_or_missing_cursors() {
        let mut service = service();
        for index in 0..242 {
            service
                .create_tag(format!("page-{index:03}"), service.revision)
                .unwrap();
        }
        add(&mut service, "high", &["page-241"]);
        add(&mut service, "equal", &["page-240"]);
        let before = service.revision;
        let mut items = Vec::new();
        let mut after = None;
        loop {
            let page = service
                .tag_page(TagQuery {
                    sort: TagSort::Count,
                    limit: 7,
                    after,
                    expected_revision: Some(before),
                })
                .unwrap();
            assert!(page.items.len() <= 7);
            items.extend(page.items.into_iter().map(|tag| tag.name));
            after = page.next_after;
            assert_eq!(page.has_more, after.is_some());
            if after.is_none() {
                break;
            }
        }
        assert_eq!(items.len(), 244);
        assert_eq!(&items[..3], &["page-240", "page-241", "page-000"]);
        assert_eq!(items.iter().collect::<BTreeSet<_>>().len(), items.len());
        let first = service
            .tag_page(TagQuery {
                sort: TagSort::Count,
                limit: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(first.items.len(), 200);
        assert!(first.has_more);
        assert_eq!(
            service
                .tag_page(TagQuery {
                    sort: TagSort::Count,
                    after: Some("missing".into()),
                    expected_revision: Some(before),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "invalid_tag_cursor"
        );
        assert_eq!(
            service
                .tag_page(TagQuery {
                    sort: TagSort::Count,
                    after: first.next_after.clone(),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
        add(&mut service, "late", &["page-001"]);
        assert_eq!(
            service
                .tag_page(TagQuery {
                    sort: TagSort::Count,
                    after: first.next_after,
                    expected_revision: Some(before),
                    ..Default::default()
                })
                .unwrap_err()
                .code,
            "history_conflict"
        );
        let parsed: TagQuery = serde_json::from_str("{}").unwrap();
        assert!(matches!(parsed.sort, TagSort::Name));
        assert!(serde_json::from_str::<TagQuery>(r#"{"sort":"unknown"}"#).is_err());
    }
}
