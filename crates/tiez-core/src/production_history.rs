//! Storage-neutral policies shared by the production Tauri history adapter.
//!
//! The caller keeps ownership of its transport/domain type. This module only
//! requires the fields needed to merge persisted and session-only entries,
//! preserve stable ordering, apply search semantics, and enforce result limits.

/// Minimal view of a production clipboard-history entry.
pub trait HistoryRecord {
    fn id(&self) -> i64;
    fn content_type(&self) -> &str;
    fn content(&self) -> &str;
    fn source_app(&self) -> &str;
    fn tags(&self) -> &[String];
    fn is_pinned(&self) -> bool;
    fn pinned_order(&self) -> i64;
    fn timestamp(&self) -> i64;
}

/// Merge a repository page with session-only entries while preserving TieZ's
/// stable pinned-first ordering and first-page-only session behavior.
pub fn merge_history_page<T, I>(
    mut persisted: Vec<T>,
    session_newest_first: I,
    limit: i32,
    offset: i32,
    content_type: Option<&str>,
) -> Vec<T>
where
    T: HistoryRecord,
    I: IntoIterator<Item = T>,
{
    if offset == 0 {
        for item in session_newest_first {
            if content_type.is_some_and(|expected| item.content_type() != expected) {
                continue;
            }
            if !contains_stable_id(&persisted, item.id()) {
                persisted.push(item);
            }
        }
    }

    persisted.sort_by(|left, right| {
        right
            .is_pinned()
            .cmp(&left.is_pinned())
            .then_with(|| right.pinned_order().cmp(&left.pinned_order()))
            .then_with(|| right.timestamp().cmp(&left.timestamp()))
            .then_with(|| right.id().cmp(&left.id()))
    });
    truncate_to_limit(&mut persisted, limit);
    persisted
}

/// Merge repository search results with matching session-only entries while
/// preserving the production timestamp/ID ordering.
pub fn merge_history_search<T, I>(
    mut persisted: Vec<T>,
    session_newest_first: I,
    search_term: &str,
    limit: i32,
    tag_only: bool,
) -> Vec<T>
where
    T: HistoryRecord,
    I: IntoIterator<Item = T>,
{
    let term = search_term.to_lowercase();
    for item in session_newest_first {
        let matches = if tag_only {
            item.tags()
                .iter()
                .any(|tag| tag.to_lowercase().contains(&term))
        } else {
            item.content().to_lowercase().contains(&term)
                || item.source_app().to_lowercase().contains(&term)
                || item
                    .tags()
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&term))
        };

        if matches && !contains_stable_id(&persisted, item.id()) {
            persisted.push(item);
        }
    }

    persisted.sort_by(|left, right| {
        right
            .timestamp()
            .cmp(&left.timestamp())
            .then_with(|| right.id().cmp(&left.id()))
    });
    truncate_to_limit(&mut persisted, limit);
    persisted
}

fn contains_stable_id<T: HistoryRecord>(items: &[T], candidate_id: i64) -> bool {
    candidate_id != 0 && items.iter().any(|item| item.id() == candidate_id)
}

fn truncate_to_limit<T>(items: &mut Vec<T>, limit: i32) {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if items.len() > limit {
        items.truncate(limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestRecord {
        id: i64,
        content_type: String,
        content: String,
        source_app: String,
        tags: Vec<String>,
        is_pinned: bool,
        pinned_order: i64,
        timestamp: i64,
    }

    impl TestRecord {
        fn new(id: i64, timestamp: i64) -> Self {
            Self {
                id,
                content_type: "text".to_owned(),
                content: format!("content {id}"),
                source_app: "TieZ".to_owned(),
                tags: Vec::new(),
                is_pinned: false,
                pinned_order: 0,
                timestamp,
            }
        }
    }

    impl HistoryRecord for TestRecord {
        fn id(&self) -> i64 {
            self.id
        }

        fn content_type(&self) -> &str {
            &self.content_type
        }

        fn content(&self) -> &str {
            &self.content
        }

        fn source_app(&self) -> &str {
            &self.source_app
        }

        fn tags(&self) -> &[String] {
            &self.tags
        }

        fn is_pinned(&self) -> bool {
            self.is_pinned
        }

        fn pinned_order(&self) -> i64 {
            self.pinned_order
        }

        fn timestamp(&self) -> i64 {
            self.timestamp
        }
    }

    #[test]
    fn page_merge_preserves_session_ids_filters_and_pinned_order() {
        let persisted = vec![TestRecord::new(1, 100), TestRecord::new(2, 200)];
        let mut duplicate = TestRecord::new(2, 400);
        duplicate.source_app = "session duplicate".to_owned();
        let mut pinned = TestRecord::new(-1, 50);
        pinned.is_pinned = true;
        pinned.pinned_order = 3;
        let mut filtered = TestRecord::new(-2, 500);
        filtered.content_type = "image".to_owned();

        let result = merge_history_page(
            persisted,
            vec![duplicate, filtered, pinned],
            10,
            0,
            Some("text"),
        );

        assert_eq!(
            result.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![-1, 2, 1]
        );
    }

    #[test]
    fn later_pages_do_not_merge_session_entries() {
        let result = merge_history_page(
            vec![TestRecord::new(1, 100)],
            vec![TestRecord::new(-1, 500)],
            10,
            10,
            None,
        );

        assert_eq!(
            result.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn search_merge_matches_content_source_and_tags_with_stable_order() {
        let persisted = vec![TestRecord::new(1, 100)];
        let mut source_match = TestRecord::new(-1, 300);
        source_match.source_app = "Microsoft Edge".to_owned();
        let mut tag_match = TestRecord::new(-2, 200);
        tag_match.tags.push("Work".to_owned());
        let no_match = TestRecord::new(-3, 400);

        let text_results = merge_history_search(
            persisted.clone(),
            vec![no_match.clone(), tag_match.clone(), source_match],
            "edge",
            10,
            false,
        );
        assert_eq!(
            text_results.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![-1, 1]
        );

        let tag_results =
            merge_history_search(persisted, vec![no_match, tag_match], "work", 1, true);
        assert_eq!(
            tag_results.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![-2]
        );
    }
}
