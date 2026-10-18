//! Authoritative native-open preparation/acceptance and owned temporary exports.
//! This Rust service is not yet exposed through the C ABI or WinUI. No edit watcher yet.
use crate::{CoreError, CoreResult, HistoryService};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiez_content::open::{
    Export, Handler as ContentHandler, PreparedOpen, Target as ContentTarget,
};
use tiez_native_platform::open::{self as platform, Handler, Receipt, Route, Target};
use tiez_storage::domain::models::ClipboardEntry;

const MAX_EXPORTS: usize = 64;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const LIFETIME: Duration = Duration::from_secs(3600);
fn error(code: &'static str, message: &'static str) -> CoreError {
    CoreError::new(code, message)
}
#[derive(Debug, Clone, Copy)]
pub struct OpenStatus {
    pub active_exports: usize,
    pub export_bytes: usize,
    pub closed: bool,
}
/// This stage does not increment usage or start edit writeback; both remain required integration.
#[derive(Debug)]
pub struct OpenResult {
    pub id: i64,
    pub history_revision: u64,
    pub route: Route,
    pub exported: bool,
    pub edit_watch_required: bool,
}
// No Debug/Serialize on records containing bodies or paths.
struct ExportFile {
    directory: PathBuf,
    parent: PathBuf,
    bytes: usize,
    expires: Instant,
}
impl ExportFile {
    fn cleanup(&self) -> std::io::Result<()> {
        let metadata = match fs::symlink_metadata(&self.directory) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        // Delete only the uniquely created, recorded namespace, never a replacement link/file.
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || fs::canonicalize(&self.directory)?.parent() != Some(self.parent.as_path())
        {
            return Err(std::io::Error::other("Export namespace changed"));
        }
        fs::remove_dir_all(&self.directory)
    }
}
enum OwnedTarget {
    Url(String),
    Path(PathBuf),
}
struct PreparedRequest {
    id: i64,
    revision: u64,
    handler: ContentHandler,
    target: OwnedTarget,
    export: Option<u64>,
    watch: bool,
}
/// One owner per core runtime. &mut serializes launch/cleanup; no detached worker or watcher.
/// Construction and status reads do not create directories, save settings or change history.
pub struct OpenService {
    history: Arc<Mutex<HistoryService>>,
    directory: PathBuf,
    temporary_parent: PathBuf,
    exports: BTreeMap<u64, ExportFile>,
    next_export: u64,
    closed: bool,
}
impl OpenService {
    pub fn new(history: Arc<Mutex<HistoryService>>, data_directory: PathBuf) -> CoreResult<Self> {
        if !data_directory.is_absolute() || !data_directory.is_dir() {
            return Err(error(
                "invalid_path",
                "An existing absolute data directory is required",
            ));
        }
        Ok(Self {
            history,
            directory: data_directory,
            temporary_parent: std::env::temp_dir(),
            exports: BTreeMap::new(),
            next_export: 0,
            closed: false,
        })
    }
    pub fn status(&self) -> OpenStatus {
        OpenStatus {
            active_exports: self.exports.len(),
            export_bytes: self.exports.values().map(|e| e.bytes).sum(),
            closed: self.closed,
        }
    }
    fn ensure_open(&self) -> CoreResult<()> {
        if self.closed {
            Err(error("open_closed", "External-open service is closed"))
        } else {
            Ok(())
        }
    }
    fn checked_entry(
        &self,
        id: i64,
        revision: u64,
        allow_sensitive: bool,
    ) -> CoreResult<ClipboardEntry> {
        let history = self
            .history
            .lock()
            .map_err(|_| error("history", "History unavailable"))?;
        if history.revision() != revision {
            return Err(error(
                "history_conflict",
                "History changed; refresh before opening",
            ));
        }
        let entry = history.entry(id)?;
        if tiez_storage::database::has_sensitive_tag(&entry.tags) && !allow_sensitive {
            return Err(error(
                "sensitive_confirmation",
                "Reveal sensitive content before opening it",
            ));
        }
        Ok(entry)
    }
    fn remove_export(&mut self, token: u64) -> bool {
        let Some(file) = self.exports.get(&token) else {
            return true;
        };
        if file.cleanup().is_err() {
            return false;
        }
        self.exports.remove(&token);
        true
    }
    fn clean_tokens(&mut self, tokens: Vec<u64>) -> CoreResult<usize> {
        let mut removed = 0;
        let mut failed = false;
        for token in tokens {
            if self.remove_export(token) {
                removed += 1;
            } else {
                failed = true;
            }
        }
        if failed {
            Err(error(
                "open_cleanup",
                "Temporary export cleanup remains pending; retry after closing the editor",
            ))
        } else {
            Ok(removed)
        }
    }
    fn reap_at(&mut self, now: Instant) -> CoreResult<usize> {
        let tokens = self
            .exports
            .iter()
            .filter(|(_, file)| file.expires <= now)
            .map(|(&token, _)| token)
            .collect();
        self.clean_tokens(tokens)
    }
    /// Expiry is cooperative: call from runtime polling and before opening; no new thread.
    pub fn reap_expired(&mut self) -> CoreResult<usize> {
        self.reap_at(Instant::now())
    }
    /// Marks closed even when a locked export cannot be removed. Repeated close retries cleanup.
    pub fn close(&mut self) -> CoreResult<usize> {
        self.closed = true;
        self.clean_tokens(self.exports.keys().copied().collect())
    }
    fn failed(&mut self, export: Option<u64>, mut failure: CoreError) -> CoreError {
        if export.is_some_and(|token| !self.remove_export(token)) {
            failure
                .message
                .push_str(" Temporary export cleanup remains pending.");
        }
        failure
    }
    fn create_export(&mut self, export: &Export) -> CoreResult<(u64, PathBuf)> {
        let bytes = export.bytes().len();
        if self.exports.len() >= MAX_EXPORTS
            || self
                .status()
                .export_bytes
                .checked_add(bytes)
                .is_none_or(|n| n > MAX_BYTES)
        {
            return Err(error(
                "open_export_limit",
                "Temporary export limit reached; close or expire existing exports",
            ));
        }
        let token = self
            .next_export
            .checked_add(1)
            .ok_or_else(|| error("open_export_limit", "Export identity space exhausted"))?;
        // Random names contain no history ID, application, URL or content. create_dir/create_new
        // prevent overwrites. The extension comes only from validated content preparation.
        let parent = fs::canonicalize(&self.temporary_parent)
            .map_err(|_| error("open_export", "Temporary parent unavailable"))?;
        let directory = parent.join(format!("TieZ-open-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).map_err(|_| {
            error(
                "open_export",
                "Temporary export directory could not be created",
            )
        })?;
        let path = directory.join(format!("content.{}", export.extension()));
        self.next_export = token;
        self.exports.insert(
            token,
            ExportFile {
                directory,
                parent,
                bytes,
                expires: Instant::now() + LIFETIME,
            },
        );
        let written = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(export.bytes())?;
            file.flush()
        })();
        if written.is_err() {
            return Err(self.failed(
                Some(token),
                error("open_export", "Temporary export could not be written"),
            ));
        }
        Ok((token, path))
    }
    fn prepare(
        &mut self,
        id: i64,
        revision: u64,
        allow_sensitive: bool,
        values: &BTreeMap<String, String>,
    ) -> CoreResult<PreparedRequest> {
        self.ensure_open()?;
        let entry = self.checked_entry(id, revision, allow_sensitive)?;
        self.reap_expired()?;
        let mut roots = tiez_content::clipboard_image_roots(&self.directory);
        if entry.content_type == "image" && entry.is_external {
            let path = std::path::Path::new(&entry.content);
            if path.is_absolute() {
                if let Some(parent) = path.parent() {
                    roots.push(parent.into());
                }
            }
        }
        let PreparedOpen { handler, target } =
            tiez_content::with_data_directory(&self.directory, || {
                PreparedOpen::prepare(&entry, values, &roots)
            })
            .map_err(|code| error(code, "Content could not be prepared for opening"))?;
        let mut export = None;
        let mut watch = false;
        let target = match target {
            ContentTarget::Url(value) => OwnedTarget::Url(value),
            ContentTarget::Existing(path) => OwnedTarget::Path(path),
            ContentTarget::Export(value) => {
                let (token, path) = self.create_export(&value)?;
                export = Some(token);
                watch = value.watch();
                OwnedTarget::Path(path)
            }
        };
        Ok(PreparedRequest {
            id,
            revision,
            handler,
            target,
            export,
            watch,
        })
    }
    fn dispatch(
        &mut self,
        request: PreparedRequest,
        launcher: impl FnOnce(Handler<'_>, Target<'_>) -> Result<Receipt, &'static str>,
    ) -> CoreResult<OpenResult> {
        let accepted = (|| {
            self.ensure_open()?;
            let history = self
                .history
                .lock()
                .map_err(|_| error("history", "History unavailable"))?;
            if history.revision() != request.revision {
                return Err(error(
                    "history_conflict",
                    "History changed; refresh before opening",
                ));
            }
            // Acceptance/linearization point: the original consent and body are still current.
            // Later history changes cannot revoke an already accepted external action. Never
            // hold this mutex across the blocking Windows dispatch or run on a UI dispatcher.
            Ok(())
        })();
        if let Err(failure) = accepted {
            return Err(self.failed(request.export, failure));
        }
        let handler = match &request.handler {
            ContentHandler::System => Handler::System,
            ContentHandler::Desktop(path) => Handler::Desktop(path),
            ContentHandler::Packaged(id) => Handler::Packaged(id),
        };
        let target = match &request.target {
            OwnedTarget::Url(value) => Target::Url(value),
            OwnedTarget::Path(path) => Target::Path(path),
        };
        let receipt = launcher(handler, target).map_err(|code| {
            self.failed(
                request.export,
                error(code, "External application did not accept the open request"),
            )
        })?;
        if let Some(file) = request
            .export
            .and_then(|token| self.exports.get_mut(&token))
        {
            file.expires = Instant::now() + LIFETIME;
        }
        Ok(OpenResult {
            id: request.id,
            history_revision: request.revision,
            route: receipt.route,
            exported: request.export.is_some(),
            edit_watch_required: request.watch,
        })
    }
    /// Core callers supply saved opener values, never a UI content body. C ABI wiring remains pending.
    pub fn open_at_revision(
        &mut self,
        id: i64,
        revision: u64,
        allow_sensitive: bool,
        values: &BTreeMap<String, String>,
    ) -> CoreResult<OpenResult> {
        let request = self.prepare(id, revision, allow_sensitive, values)?;
        self.dispatch(request, platform::launch)
    }
}
impl Drop for OpenService {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    // Drop fields in this order: open exports, all SQLite owners, fixture namespace.
    struct Fixture {
        service: OpenService,
        history: Arc<Mutex<HistoryService>>,
        scratch: Scratch,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("TieZ-open-core-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("exports")).unwrap();
            let history = Arc::new(Mutex::new(
                HistoryService::open(&root.join("fixture.db")).unwrap(),
            ));
            let mut service = OpenService::new(history.clone(), root.clone()).unwrap();
            service.temporary_parent = root.join("exports");
            Self {
                service,
                history,
                scratch: Scratch(root),
            }
        }
        fn add(
            &self,
            kind: &str,
            body: &str,
            html: Option<&str>,
            private: bool,
            persistent: bool,
        ) -> (i64, u64) {
            let mut history = self.history.lock().unwrap();
            let entry = ClipboardEntry {
                id: 0,
                content_type: kind.into(),
                content: body.into(),
                html_content: html.map(str::to_string),
                source_app: "fixture".into(),
                source_app_path: None,
                timestamp: 1,
                is_pinned: false,
                tags: if private {
                    vec!["sensitive".into()]
                } else {
                    vec![]
                },
                use_count: 0,
                is_external: false,
                pinned_order: 0,
                preview: body.into(),
                file_preview_exists: true,
            };
            let result = history.record_processed(entry, persistent).unwrap();
            (result.id, history.revision())
        }
        fn assert_empty(&self) {
            assert_eq!(self.service.status().active_exports, 0);
            assert_eq!(
                fs::read_dir(self.scratch.0.join("exports"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }
    fn accepted() -> Receipt {
        Receipt {
            route: Route::System,
            process_id: None,
        }
    }
    fn path(request: &PreparedRequest) -> &std::path::Path {
        match &request.target {
            OwnedTarget::Path(path) => path,
            _ => panic!("Expected path"),
        }
    }
    #[test]
    fn session_and_persisted_text_open_exact_bytes_without_history_or_usage_writes() {
        for persistent in [false, true] {
            let mut f = Fixture::new();
            let body = "  中文\r\n\t  ";
            let (id, revision) = f.add("text", body, None, false, persistent);
            assert_eq!(id > 0, persistent);
            f.assert_empty();
            let request = f
                .service
                .prepare(id, revision, false, &BTreeMap::new())
                .unwrap();
            let exported = path(&request).to_path_buf();
            assert_eq!(fs::read(&exported).unwrap(), body.as_bytes());
            let result = f
                .service
                .dispatch(request, |handler, target| {
                    assert!(matches!(handler, Handler::System));
                    assert!(matches!(target,Target::Path(p) if p==exported));
                    Ok(accepted())
                })
                .unwrap();
            assert_eq!((result.id, result.history_revision), (id, revision));
            assert!(result.exported && result.edit_watch_required);
            let history = f.history.lock().unwrap();
            assert_eq!(history.revision(), revision);
            let entry = history.entry(id).unwrap();
            assert_eq!(entry.content, body);
            assert_eq!(entry.use_count, 0);
            drop(history);
            assert_eq!(f.service.close().unwrap(), 1);
            assert!(!exported.exists());
            f.assert_empty();
            assert_eq!(
                f.service
                    .open_at_revision(id, revision, false, &BTreeMap::new())
                    .unwrap_err()
                    .code,
                "open_closed"
            );
        }
    }
    #[test]
    fn stale_missing_sensitive_and_bad_url_never_export_or_launch() {
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "private", None, true, false);
        assert_eq!(
            f.service
                .open_at_revision(id, revision - 1, false, &BTreeMap::new())
                .unwrap_err()
                .code,
            "history_conflict"
        );
        assert_eq!(
            f.service
                .open_at_revision(id, revision, false, &BTreeMap::new())
                .unwrap_err()
                .code,
            "sensitive_confirmation"
        );
        assert_eq!(
            f.service
                .open_at_revision(9999, revision, true, &BTreeMap::new())
                .unwrap_err()
                .code,
            "not_found"
        );
        f.assert_empty();
        let (url, revision) = f.add("url", "file:///C:/Windows/cmd.exe", None, false, false);
        assert_eq!(
            f.service
                .open_at_revision(url, revision, false, &BTreeMap::new())
                .unwrap_err()
                .code,
            "open_url"
        );
        f.assert_empty();
    }
    #[test]
    fn changed_entry_privacy_or_deletion_before_acceptance_rejects_and_cleans() {
        for change in 0..3 {
            let mut f = Fixture::new();
            let (id, revision) = f.add("text", "original", None, false, false);
            let request = f
                .service
                .prepare(id, revision, false, &BTreeMap::new())
                .unwrap();
            let exported = path(&request).to_path_buf();
            {
                let mut history = f.history.lock().unwrap();
                match change {
                    0 => {
                        history.edit(id, "changed".into()).unwrap();
                    }
                    1 => {
                        history.set_tags(id, vec!["sensitive".into()]).unwrap();
                    }
                    _ => {
                        history.delete(id).unwrap();
                    }
                }
            }
            let failure = f
                .service
                .dispatch(request, |_, _| panic!("Stale request reached Windows"))
                .unwrap_err();
            assert_eq!(failure.code, "history_conflict");
            assert!(!exported.exists());
            f.assert_empty();
        }
    }
    #[test]
    fn explicit_sensitive_reveal_allows_prepare_and_failed_launch_removes_export() {
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "private", None, true, true);
        let request = f
            .service
            .prepare(id, revision, true, &BTreeMap::new())
            .unwrap();
        let exported = path(&request).to_path_buf();
        assert_eq!(fs::read(&exported).unwrap(), b"private");
        let error = f
            .service
            .dispatch(request, |_, _| Err("fixture_launch_failure"))
            .unwrap_err();
        assert_eq!(error.code, "fixture_launch_failure");
        assert!(!error.message.contains("private"));
        f.assert_empty();
        assert_eq!(f.history.lock().unwrap().revision(), revision);
    }
    #[test]
    fn rich_html_keeps_bytes_and_no_watch_flag_while_url_and_files_do_not_export() {
        let mut f = Fixture::new();
        let html = "<html><b>中文</b></html>";
        let (id, revision) = f.add("rich_text", "fallback", Some(html), false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        assert_eq!(path(&request).extension().unwrap(), "html");
        assert_eq!(fs::read(path(&request)).unwrap(), html.as_bytes());
        let result = f.service.dispatch(request, |_, _| Ok(accepted())).unwrap();
        assert!(!result.edit_watch_required);
        f.service.close().unwrap();
        let mut f = Fixture::new();
        let file = f.scratch.0.join("原文件 %20 ' &.txt");
        fs::write(&file, b"source").unwrap();
        let (id, revision) = f.add("file", file.to_str().unwrap(), None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        assert_eq!(path(&request), file);
        assert!(request.export.is_none());
        f.service.dispatch(request, |_, _| Ok(accepted())).unwrap();
        f.assert_empty();
        assert_eq!(fs::read(&file).unwrap(), b"source");
        let (id, revision) = f.add("url", "www.example.test/?x=1&ver", None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        f.service
            .dispatch(request, |_, target| {
                assert!(matches!(target,Target::Url(v) if v=="https://www.example.test/?x=1&ver"));
                Ok(accepted())
            })
            .unwrap();
        f.assert_empty();
    }
    #[test]
    fn expiry_close_and_drop_remove_only_owned_exports() {
        let mut f = Fixture::new();
        let sentinel = f.service.temporary_parent.join("unrelated.txt");
        fs::write(&sentinel, b"keep").unwrap();
        let (id, revision) = f.add("text", "export", None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        let exported = path(&request).to_path_buf();
        f.service.dispatch(request, |_, _| Ok(accepted())).unwrap();
        assert_eq!(
            f.service
                .reap_at(Instant::now() + LIFETIME + Duration::from_secs(1))
                .unwrap(),
            1
        );
        assert!(!exported.exists());
        assert!(sentinel.exists());
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        let exported = path(&request).to_path_buf();
        f.service.dispatch(request, |_, _| Ok(accepted())).unwrap();
        let replacement = OpenService::new(f.history.clone(), f.scratch.0.clone()).unwrap();
        drop(std::mem::replace(&mut f.service, replacement));
        assert!(!exported.exists());
        assert_eq!(fs::read(sentinel).unwrap(), b"keep");
    }
    #[cfg(windows)]
    #[test]
    fn locked_cleanup_is_reported_tracked_and_retryable_after_close() {
        use std::os::windows::fs::OpenOptionsExt;
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "export", None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        let exported = path(&request).to_path_buf();
        f.service.dispatch(request, |_, _| Ok(accepted())).unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&exported)
            .unwrap();
        assert_eq!(f.service.close().unwrap_err().code, "open_cleanup");
        assert!(f.service.status().closed);
        assert_eq!(f.service.status().active_exports, 1);
        drop(locked);
        assert_eq!(f.service.close().unwrap(), 1);
        f.assert_empty();
    }
    #[test]
    fn file_limit_write_failure_and_replaced_namespace_do_not_discard_ownership() {
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "export", None, false, false);
        for _ in 0..MAX_EXPORTS {
            let r = f
                .service
                .prepare(id, revision, false, &BTreeMap::new())
                .unwrap();
            f.service.dispatch(r, |_, _| Ok(accepted())).unwrap();
        }
        assert!(matches!(
            f.service.prepare(id, revision, false, &BTreeMap::new()),
            Err(CoreError {
                code: "open_export_limit",
                ..
            })
        ));
        assert_eq!(f.service.close().unwrap(), MAX_EXPORTS);
        f.assert_empty();
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "export", None, false, false);
        f.service.temporary_parent = f.scratch.0.join("missing");
        assert!(matches!(
            f.service.prepare(id, revision, false, &BTreeMap::new()),
            Err(CoreError {
                code: "open_export",
                ..
            })
        ));
        assert_eq!(f.service.status().active_exports, 0);
        f.service.temporary_parent = f.scratch.0.join("exports");
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        let exported = path(&request).to_path_buf();
        let folder = exported.parent().unwrap();
        fs::remove_file(&exported).unwrap();
        fs::remove_dir(folder).unwrap();
        fs::write(folder, b"replacement").unwrap();
        assert_eq!(f.service.close().unwrap_err().code, "open_cleanup");
        assert_eq!(fs::read(folder).unwrap(), b"replacement");
        fs::remove_file(folder).unwrap();
        assert_eq!(f.service.close().unwrap(), 1);
        f.assert_empty();
    }
    #[test]
    fn actual_native_handler_failure_cleans_and_leaves_source_unchanged() {
        let mut f = Fixture::new();
        let (id, revision) = f.add("code", "  source\r\n ", None, false, false);
        let handler = f.scratch.0.join("missing-editor.exe");
        let values = BTreeMap::from([("app.text".into(), handler.to_str().unwrap().into())]);
        let error = f
            .service
            .open_at_revision(id, revision, false, &values)
            .unwrap_err();
        assert_eq!(error.code, "open_handler_missing");
        f.assert_empty();
        let history = f.history.lock().unwrap();
        assert_eq!(history.revision(), revision);
        assert_eq!(history.entry(id).unwrap().content, "  source\r\n ");
    }

    #[test]
    fn accepted_snapshot_does_not_hold_history_mutex_during_external_dispatch() {
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "accepted", None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        let history = f.history.clone();
        let result = f
            .service
            .dispatch(request, move |_, target| {
                assert!(matches!(target,Target::Path(p) if fs::read(p).unwrap()==b"accepted"));
                let mut history = history
                    .try_lock()
                    .expect("Windows dispatch must not hold the history lock");
                history.edit(id, "changed after acceptance".into()).unwrap();
                Ok(accepted())
            })
            .unwrap();
        assert_eq!(result.history_revision, revision);
        assert!(f.history.lock().unwrap().revision() > revision);
        assert_eq!(
            f.history.lock().unwrap().entry(id).unwrap().content,
            "changed after acceptance"
        );
    }
    #[cfg(windows)]
    #[test]
    fn rejected_dispatch_keeps_primary_error_when_locked_cleanup_needs_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let mut f = Fixture::new();
        let (id, revision) = f.add("text", "private original", None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        let exported = path(&request).to_path_buf();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&exported)
            .unwrap();
        f.history
            .lock()
            .unwrap()
            .set_tags(id, vec!["sensitive".into()])
            .unwrap();
        let failure = f
            .service
            .dispatch(request, |_, _| panic!("Rejected request must not launch"))
            .unwrap_err();
        assert_eq!(failure.code, "history_conflict");
        assert!(failure.message.contains("cleanup remains pending"));
        assert!(!failure.message.contains("private original"));
        assert!(!failure.message.contains(exported.to_str().unwrap()));
        assert_eq!(f.service.status().active_exports, 1);
        drop(locked);
        assert_eq!(f.service.close().unwrap(), 1);
        f.assert_empty();
    }
    #[test]
    fn image_export_preserves_original_encoded_bytes_and_history_identity() {
        let mut f = Fixture::new();
        let body="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
        let original: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 16, 84, 50,
            254, 15, 0, 2, 20, 1, 102, 4, 224, 222, 127, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
            130,
        ];
        let (id, revision) = f.add("image", body, None, false, false);
        let request = f
            .service
            .prepare(id, revision, false, &BTreeMap::new())
            .unwrap();
        assert_eq!(path(&request).extension().unwrap(), "png");
        assert_eq!(fs::read(path(&request)).unwrap(), original);
        let result = f.service.dispatch(request, |_, _| Ok(accepted())).unwrap();
        assert!(result.exported && result.edit_watch_required);
        let history = f.history.lock().unwrap();
        assert_eq!(history.revision(), revision);
        assert_eq!(history.entry(id).unwrap().content, body);
    }
}
