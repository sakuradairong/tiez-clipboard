use crate::app_state::SessionHistory;
use crate::domain::models::ClipboardEntry;
use crate::infrastructure::repository::clipboard_repo::{
    ClipboardRepository, SqliteClipboardRepository,
};
use tiez_core::production_history::{merge_history_page, merge_history_search, HistoryRecord};

pub(crate) struct ResolvedHistoryContent {
    pub content: String,
    pub content_type: String,
    pub html_content: Option<String>,
}

pub(crate) trait HistoryRepository {
    fn list(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String>;

    fn search(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String>;

    fn content(&self, id: i64) -> Result<Option<(String, String, Option<String>)>, String>;
}

impl HistoryRepository for SqliteClipboardRepository {
    fn list(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String> {
        self.get_history(limit, offset, content_type)
    }

    fn search(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String> {
        ClipboardRepository::search(self, query, limit, tag_only)
    }

    fn content(&self, id: i64) -> Result<Option<(String, String, Option<String>)>, String> {
        self.get_entry_content_with_html(id)
    }
}

pub(crate) struct TauriHistoryAdapter<'a, R> {
    repository: &'a R,
    session: &'a SessionHistory,
}

impl<'a, R: HistoryRepository> TauriHistoryAdapter<'a, R> {
    pub(crate) fn new(repository: &'a R, session: &'a SessionHistory) -> Self {
        Self {
            repository,
            session,
        }
    }

    pub(crate) fn list(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let persisted = self.repository.list(limit, offset, content_type)?;
        let session = if offset == 0 {
            self.session_newest_first()?
        } else {
            Vec::new()
        };
        Ok(merge_history_page(
            persisted,
            session,
            limit,
            offset,
            content_type,
        ))
    }

    pub(crate) fn search(
        &self,
        search_term: &str,
        limit: i32,
        tag_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let persisted = self.repository.search(search_term, limit, tag_only)?;
        Ok(merge_history_search(
            persisted,
            self.session_newest_first()?,
            search_term,
            limit,
            tag_only,
        ))
    }

    pub(crate) fn content(&self, id: i64) -> Result<Option<ResolvedHistoryContent>, String> {
        if let Some(item) = self
            .session_newest_first()?
            .into_iter()
            .find(|item| item.id == id)
        {
            return Ok(Some(ResolvedHistoryContent {
                content: item.content,
                content_type: item.content_type,
                html_content: item.html_content,
            }));
        }

        Ok(self
            .repository
            .content(id)?
            .map(
                |(content, content_type, html_content)| ResolvedHistoryContent {
                    content,
                    content_type,
                    html_content,
                },
            ))
    }

    fn session_newest_first(&self) -> Result<Vec<ClipboardEntry>, String> {
        let items = self
            .session
            .0
            .lock()
            .map_err(|_| "SessionHistory lock is poisoned".to_owned())?;
        Ok(items.iter().rev().cloned().collect())
    }
}

impl HistoryRecord for ClipboardEntry {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeRepository {
        items: Vec<ClipboardEntry>,
        content: Option<(String, String, Option<String>)>,
    }

    impl HistoryRepository for FakeRepository {
        fn list(
            &self,
            _limit: i32,
            _offset: i32,
            _content_type: Option<&str>,
        ) -> Result<Vec<ClipboardEntry>, String> {
            Ok(self.items.clone())
        }

        fn search(
            &self,
            _query: &str,
            _limit: i32,
            _tag_only: bool,
        ) -> Result<Vec<ClipboardEntry>, String> {
            Ok(self.items.clone())
        }

        fn content(&self, _id: i64) -> Result<Option<(String, String, Option<String>)>, String> {
            Ok(self.content.clone())
        }
    }

    fn entry(id: i64, timestamp: i64, content: &str) -> ClipboardEntry {
        ClipboardEntry {
            id,
            content_type: "text".to_owned(),
            content: content.to_owned(),
            html_content: None,
            source_app: "TieZ".to_owned(),
            source_app_path: None,
            timestamp,
            preview: content.to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: true,
        }
    }

    #[test]
    fn adapter_merges_session_entries_without_changing_negative_ids() {
        let repository = FakeRepository {
            items: vec![entry(1, 100, "persisted")],
            content: None,
        };
        let session = SessionHistory(Mutex::new(VecDeque::from([entry(-1, 200, "session")])));
        let adapter = TauriHistoryAdapter::new(&repository, &session);

        let result = adapter.list(10, 0, None).unwrap();

        assert_eq!(
            result.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![-1, 1]
        );
    }

    #[test]
    fn adapter_prefers_session_content_before_repository_content() {
        let repository = FakeRepository {
            items: Vec::new(),
            content: Some(("persisted".to_owned(), "text".to_owned(), None)),
        };
        let session = SessionHistory(Mutex::new(VecDeque::from([entry(
            -1,
            200,
            "session full content",
        )])));
        let adapter = TauriHistoryAdapter::new(&repository, &session);

        let content = adapter.content(-1).unwrap().unwrap();

        assert_eq!(content.content, "session full content");
        assert_eq!(content.content_type, "text");
    }

    #[test]
    fn adapter_searches_session_entries_through_shared_policy() {
        let repository = FakeRepository {
            items: Vec::new(),
            content: None,
        };
        let mut source_match = entry(-1, 100, "browser text");
        source_match.source_app = "Microsoft Edge".to_owned();
        let mut tag_match = entry(-2, 200, "tagged text");
        tag_match.tags.push("Work".to_owned());
        let session = SessionHistory(Mutex::new(VecDeque::from([source_match, tag_match])));
        let adapter = TauriHistoryAdapter::new(&repository, &session);

        let source_results = adapter.search("edge", 10, false).unwrap();
        let tag_results = adapter.search("work", 10, true).unwrap();

        assert_eq!(source_results[0].id, -1);
        assert_eq!(tag_results[0].id, -2);
    }

    #[test]
    fn adapter_falls_back_to_repository_content() {
        let repository = FakeRepository {
            items: Vec::new(),
            content: Some((
                "persisted content".to_owned(),
                "rich_text".to_owned(),
                Some("<p>persisted content</p>".to_owned()),
            )),
        };
        let session = SessionHistory(Mutex::new(VecDeque::new()));
        let adapter = TauriHistoryAdapter::new(&repository, &session);

        let content = adapter.content(7).unwrap().unwrap();

        assert_eq!(content.content, "persisted content");
        assert_eq!(content.content_type, "rich_text");
        assert_eq!(
            content.html_content.as_deref(),
            Some("<p>persisted content</p>")
        );
    }
}
