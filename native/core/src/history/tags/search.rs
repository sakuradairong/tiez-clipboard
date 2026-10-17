use super::*;
use rusqlite::Connection;
use tiez_storage::domain::models::ClipboardEntry;
fn service() -> HistoryService {
    let connection = Connection::open_in_memory().unwrap();
    tiez_storage::infrastructure::repository::migrations::run_migrations(&connection).unwrap();
    HistoryService::from_connection(connection, std::env::temp_dir())
}
#[test]
fn search_is_unicode_literal_and_preserves_names_metadata_and_readonly_state() {
    let mut service = service();
    service
        .set_tag_color("É ' %_ 标签".into(), Some("var(--original)".into()), 0)
        .unwrap();
    service
        .create_tag("e ' wildxx 标签".into(), service.revision)
        .unwrap();
    let added = service
        .add_manual(
            "text".into(),
            "  original\r\n ".into(),
            None,
            vec!["É ' %_ 标签".into()],
        )
        .unwrap();
    let mut entry: ClipboardEntry = service.entry(added.id).unwrap();
    entry.id = -1;
    entry.tags = vec!["É ' %_ 标签".into(), "会话-%_".into()];
    service.session.push_back(entry);
    service
        .create_tag("  raw spaces  ".into(), service.revision)
        .unwrap();
    let revision = service.revision;
    let writes: i64 = service
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap();
    for sort in [TagSort::Name, TagSort::Count] {
        let page = service
            .tag_page(TagQuery {
                search: "é ' %_".into(),
                sort,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 1);
        let tag = &page.items[0];
        assert_eq!(tag.name, "É ' %_ 标签");
        assert_eq!((tag.stored_count, tag.session_count), (1, 1));
        assert_eq!(tag.color.as_deref(), Some("var(--original)"));
        assert_eq!(page.revision, revision);
    }
    let literal = service
        .tag_page(TagQuery {
            search: "%_".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(literal.items.len(), 2);
    assert!(service
        .tag_page(TagQuery {
            search: "é ' wildcard".into(),
            ..Default::default()
        })
        .unwrap()
        .items
        .is_empty());
    assert!(
        service
            .tag_page(TagQuery {
                search: " raw spaces ".into(),
                ..Default::default()
            })
            .unwrap()
            .items
            .len()
            == 1
    );
    assert!(service
        .tag_page(TagQuery {
            search: "absent".into(),
            ..Default::default()
        })
        .unwrap()
        .next_after
        .is_none());
    assert_eq!(
        service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        writes
    );
    assert_eq!(service.revision, revision);
    assert_eq!(service.entry(-1).unwrap().tags.len(), 2);
}
#[test]
fn search_filters_all_pages_before_sorting_and_checks_the_cursor_query() {
    let mut service = service();
    for index in 0..245 {
        service
            .create_tag(format!("scope-{index:03}"), service.revision)
            .unwrap();
    }
    service
        .add_manual(
            "text".into(),
            "late high".into(),
            None,
            vec!["scope-244".into()],
        )
        .unwrap();
    let revision = service.revision;
    for sort_name in ["name", "count"] {
        let mut names = Vec::new();
        let mut after = None;
        loop {
            let page = service
                .tag_page(TagQuery {
                    search: "SCOPE-2".into(),
                    sort: if sort_name == "count" {
                        TagSort::Count
                    } else {
                        TagSort::Name
                    },
                    limit: 3,
                    after,
                    expected_revision: Some(revision),
                })
                .unwrap();
            assert!(page.items.len() <= 3);
            names.extend(page.items.into_iter().map(|tag| tag.name));
            after = page.next_after;
            assert_eq!(page.has_more, after.is_some());
            if after.is_none() {
                break;
            }
        }
        assert_eq!(names.len(), 45);
        assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), 45);
        assert_eq!(
            names[0],
            if sort_name == "count" {
                "scope-244"
            } else {
                "scope-200"
            }
        );
    }
    assert_eq!(
        service
            .tag_page(TagQuery {
                search: "scope-2".into(),
                after: Some("scope-001".into()),
                expected_revision: Some(revision),
                ..Default::default()
            })
            .unwrap_err()
            .code,
        "invalid_tag_cursor"
    );
    service.create_tag("scope-245".into(), revision).unwrap();
    assert_eq!(
        service
            .tag_page(TagQuery {
                search: "scope-2".into(),
                after: Some("scope-201".into()),
                expected_revision: Some(revision),
                ..Default::default()
            })
            .unwrap_err()
            .code,
        "history_conflict"
    );
    let query: TagQuery = serde_json::from_str("{}").unwrap();
    assert!(query.search.is_empty());
}
