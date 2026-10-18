//! Core-owned capture lifecycle. Lock order: policy then history; never registry.
use crate::{CoreError, CoreResult, HistoryService, SettingsSnapshot};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiez_content::{capture, processing};
use tiez_native_platform::{source_app::ActiveAppInfo, ClipboardEvents, ClipboardListener};

#[derive(Clone)]
struct Policy {
    revision: u64,
    read: capture::ReadOptions,
    transform: processing::TransformOptions,
    deduplicate: bool,
}
impl Policy {
    fn from_snapshot(snapshot: &SettingsSnapshot) -> Self {
        let get = |key: &str| snapshot.values.get(key).map(String::as_str).unwrap_or("");
        Self {
            revision: snapshot.revision,
            read: capture::ReadOptions {
                files: get("app.capture_files") == "true",
                rich_text: get("app.capture_rich_text") == "true",
            },
            deduplicate: get("app.deduplicate") == "true",
            transform: processing::TransformOptions {
                persistent: get("app.persistent") == "true",
                privacy_protection: get("app.privacy_protection") == "true",
                privacy_kinds: get("app.privacy_protection_kinds")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect(),
                privacy_custom_rules: get("app.privacy_protection_custom_rules")
                    .lines()
                    .map(|s| s.trim().to_string())
                    .collect(),
                cleanup_rules: get("app.cleanup_rules").into(),
                app_cleanup_policies: get("app.app_cleanup_policies").into(),
            },
        }
    }
}
#[derive(Default, Clone, Debug, Serialize)]
pub struct CaptureStatus {
    pub running: bool,
    pub starts: u64,
    pub notifications: u64,
    pub accepted: u64,
    pub ignored: u64,
    pub failed: u64,
    pub policy_revision: u64,
    pub last_error: Option<String>,
}
struct Stop {
    flag: AtomicBool,
    wait: Mutex<()>,
    wake: Condvar,
}
impl Stop {
    fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
            wait: Mutex::new(()),
            wake: Condvar::new(),
        }
    }
    fn cancel(&self) {
        let _guard = self.wait.lock().unwrap_or_else(|e| e.into_inner());
        self.flag.store(true, Ordering::Release);
        self.wake.notify_all();
    }
    fn delay(&self, duration: Duration) -> bool {
        let guard = self.wait.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.wake
                .wait_timeout_while(guard, duration, |_| !self.flag.load(Ordering::Acquire))
                .unwrap_or_else(|e| e.into_inner()),
        );
        self.flag.load(Ordering::Acquire)
    }
}
trait Input: Send {
    fn sequence(&self) -> u32;
    fn next(&self, timeout: Duration) -> Result<Option<u32>, &'static str>;
    fn source(&self) -> ActiveAppInfo;
    fn read(
        &mut self,
        sequence: u32,
        source: ActiveAppInfo,
        options: capture::ReadOptions,
        stop: &AtomicBool,
    ) -> Result<Option<capture::CapturedSnapshot>, capture::ReadError>;
}
struct WindowsInput(ClipboardEvents);
impl Input for WindowsInput {
    fn sequence(&self) -> u32 {
        capture::sequence()
    }
    fn next(&self, timeout: Duration) -> Result<Option<u32>, &'static str> {
        match self.0.recv_timeout(timeout) {
            Ok(signal) => Ok(Some(signal.sequence)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("capture_input_disconnected"),
        }
    }
    fn source(&self) -> ActiveAppInfo {
        capture::source()
    }
    fn read(
        &mut self,
        sequence: u32,
        source: ActiveAppInfo,
        options: capture::ReadOptions,
        stop: &AtomicBool,
    ) -> Result<Option<capture::CapturedSnapshot>, capture::ReadError> {
        capture::read(sequence, source, options, stop)
    }
}
struct Worker {
    stop: Arc<Stop>,
    thread: JoinHandle<Result<(), &'static str>>,
}
pub struct CaptureService {
    history: Arc<Mutex<HistoryService>>,
    policy: Arc<Mutex<Policy>>,
    stats: Arc<Mutex<CaptureStatus>>,
    directory: PathBuf,
    echo: Arc<Mutex<Option<Echo>>>,
    listener: Option<ClipboardListener>,
    worker: Option<Worker>,
}
impl CaptureService {
    pub fn new(
        history: Arc<Mutex<HistoryService>>,
        directory: PathBuf,
        snapshot: &SettingsSnapshot,
    ) -> Self {
        capture::clear_echo();
        Self {
            history,
            directory,
            echo: Arc::new(Mutex::new(None)),
            policy: Arc::new(Mutex::new(Policy::from_snapshot(snapshot))),
            stats: Arc::new(Mutex::new(CaptureStatus {
                policy_revision: snapshot.revision,
                ..Default::default()
            })),
            listener: None,
            worker: None,
        }
    }
    pub fn apply_settings(&self, snapshot: &SettingsSnapshot) {
        *self.policy.lock().unwrap_or_else(|e| e.into_inner()) = Policy::from_snapshot(snapshot);
        self.stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .policy_revision = snapshot.revision;
    }
    pub fn status(&self) -> CaptureStatus {
        let mut status = self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        status.running = self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.thread.is_finished());
        status
    }
    pub fn start(&mut self) -> CoreResult<CaptureStatus> {
        if self.status().running {
            return Ok(self.status());
        }
        self.stop();
        let baseline = capture::sequence();
        let (listener, events) = ClipboardListener::start().map_err(|_| {
            CoreError::new(
                "capture_start",
                "Windows clipboard listener could not start",
            )
        })?;
        self.listener = Some(listener);
        if let Err(error) = self.start_input(Box::new(WindowsInput(events)), baseline) {
            self.stop();
            return Err(error);
        }
        Ok(self.status())
    }
    fn start_input(&mut self, input: Box<dyn Input>, baseline: u32) -> CoreResult<()> {
        let stop = Arc::new(Stop::new());
        let state = WorkerState {
            stop: Arc::clone(&stop),
            history: Arc::clone(&self.history),
            policy: Arc::clone(&self.policy),
            stats: Arc::clone(&self.stats),
            directory: self.directory.clone(),
            echo: Arc::clone(&self.echo),
        };
        let stats = Arc::clone(&self.stats);
        self.stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_error = None;
        let thread = thread::Builder::new()
            .name("TieZ-native-capture".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(input, baseline, &state)
                }))
                .unwrap_or(Err("capture_worker_panicked"));
                if let Err(code) = result {
                    let mut stats = stats.lock().unwrap_or_else(|e| e.into_inner());
                    stats.failed += 1;
                    stats.last_error = Some(code.into());
                }
                result
            })
            .map_err(|_| CoreError::new("capture_start", "Clipboard worker could not start"))?;
        self.worker = Some(Worker { stop, thread });
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.starts += 1;
        Ok(())
    }
    pub fn stop(&mut self) -> CaptureStatus {
        if let Some(worker) = &self.worker {
            worker.stop.cancel();
        }
        if let Some(mut listener) = self.listener.take() {
            if listener.stop().is_err() {
                self.stats
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last_error = Some("capture_listener_stop".into());
            }
        }
        if let Some(worker) = self.worker.take() {
            if worker.thread.join().is_err() {
                let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                stats.failed += 1;
                stats.last_error = Some("capture_worker_panicked".into());
            }
        }
        capture::clear_echo();
        *self.echo.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.status()
    }
}
impl Drop for CaptureService {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
struct Echo {
    expires: Instant,
    exact: u64,
    text: Option<u64>,
    image: Option<i64>,
    sequence: Option<u32>,
    aliases: Vec<u64>,
}
impl Echo {
    fn key(
        entry: &tiez_storage::domain::models::ClipboardEntry,
    ) -> (u64, Option<u64>, Option<i64>) {
        let text = tiez_content::utils::build_clipboard_text_fingerprint(
            &entry.content_type,
            &entry.content,
            entry.html_content.as_deref(),
        );
        (
            tiez_storage::database::calc_text_hash(&entry.content),
            if tiez_storage::database::is_text_type(&entry.content_type) && !text.is_empty() {
                Some(tiez_storage::database::calc_text_hash(&text))
            } else {
                None
            },
            if entry.content_type == "image" {
                tiez_storage::database::calc_image_hash(&entry.content)
            } else {
                None
            },
        )
    }
    fn matches(&self, entry: &tiez_storage::domain::models::ClipboardEntry, sequence: u32) -> bool {
        if Instant::now() >= self.expires
            || self.sequence.is_some_and(|expected| expected != sequence)
        {
            return false;
        }
        let (exact, text, image) = Self::key(entry);
        self.exact == exact
            || self.aliases.contains(&exact)
            || self.text.is_some() && self.text == text
            || self.image.is_some() && self.image == image
    }
}
struct WorkerState {
    stop: Arc<Stop>,
    history: Arc<Mutex<HistoryService>>,
    policy: Arc<Mutex<Policy>>,
    stats: Arc<Mutex<CaptureStatus>>,
    directory: PathBuf,
    echo: Arc<Mutex<Option<Echo>>>,
}
impl CaptureService {
    /// Record a successful native clipboard write; failed writes never call this.
    #[cfg(test)]
    fn remember_write(&self, entry: &tiez_storage::domain::models::ClipboardEntry) {
        let (exact, text, image) = Echo::key(entry);
        *self.echo.lock().unwrap_or_else(|e| e.into_inner()) = Some(Echo {
            expires: Instant::now() + Duration::from_secs(10),
            exact,
            text,
            image,
            sequence: None,
            aliases: Vec::new(),
        });
    }
}
#[derive(Debug, Serialize)]
pub struct ClipboardWriteResult {
    pub id: i64,
    pub clipboard_sequence: u32,
}
pub(crate) struct PreparedCopy {
    id: i64,
    revision: u64,
    payload: tiez_content::write::PreparedWrite,
    marker: Echo,
}
impl CaptureService {
    /// Copy an authoritative history snapshot; never accept a UI-provided body.
    pub fn copy_entry(
        &self,
        id: i64,
        plain_text: bool,
        allow_sensitive: bool,
    ) -> CoreResult<ClipboardWriteResult> {
        self.copy_entry_at_revision(id, plain_text, allow_sensitive, None)
    }
    pub fn copy_entry_at_revision(
        &self,
        id: i64,
        plain_text: bool,
        allow_sensitive: bool,
        expected_revision: Option<u64>,
    ) -> CoreResult<ClipboardWriteResult> {
        let prepared =
            self.prepare_copy_at_revision(id, plain_text, allow_sensitive, expected_revision)?;
        self.commit_copy(prepared, || Ok(()))
    }
    pub(crate) fn prepare_copy(
        &self,
        id: i64,
        plain_text: bool,
        allow_sensitive: bool,
    ) -> CoreResult<PreparedCopy> {
        self.prepare_copy_at_revision(id, plain_text, allow_sensitive, None)
    }
    fn prepare_copy_at_revision(
        &self,
        id: i64,
        plain_text: bool,
        allow_sensitive: bool,
        expected_revision: Option<u64>,
    ) -> CoreResult<PreparedCopy> {
        let (entry, revision) = {
            let history = self
                .history
                .lock()
                .map_err(|_| CoreError::new("history", "History unavailable"))?;
            if expected_revision.is_some_and(|expected| expected != history.revision()) {
                return Err(CoreError::new(
                    "history_conflict",
                    "History changed; refresh before copying",
                ));
            }
            (history.entry(id)?, history.revision())
        };
        if tiez_storage::database::has_sensitive_tag(&entry.tags) && !allow_sensitive {
            return Err(CoreError::new(
                "sensitive_confirmation",
                "Reveal sensitive content before copying it",
            ));
        }
        let mut roots = tiez_content::clipboard_image_roots(&self.directory);
        if entry.content_type == "image" && entry.is_external {
            let path = std::path::Path::new(&entry.content);
            if path.is_absolute() {
                if let Some(parent) = path.parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }
        let prepared = tiez_content::with_data_directory(&self.directory, || {
            tiez_content::write::PreparedWrite::prepare(&entry, plain_text, &roots)
        })
        .map_err(|code| {
            CoreError::new(
                code,
                "Clipboard payload could not be prepared; clipboard unchanged",
            )
        })?;
        let mut identity = entry.clone();
        if plain_text {
            identity.content_type = "text".into();
            identity.html_content = None;
        }
        let (exact, text, image) =
            tiez_content::with_data_directory(&self.directory, || Echo::key(&identity));
        let marker = Echo {
            expires: Instant::now() + Duration::from_secs(10),
            exact,
            text,
            image: prepared.image_visual_hash().or(image),
            sequence: None,
            aliases: Vec::new(),
        };
        Ok(PreparedCopy {
            id,
            revision,
            payload: prepared,
            marker,
        })
    }
    pub(crate) fn commit_copy(
        &self,
        prepared: PreparedCopy,
        before_write: impl FnOnce() -> CoreResult<()>,
    ) -> CoreResult<ClipboardWriteResult> {
        let PreparedCopy {
            id,
            revision,
            payload,
            mut marker,
        } = prepared;
        // The worker holds this same gate before accepting a read. Publish the marker
        // before releasing it, even if a multi-format OS write only partially succeeds.
        let _policy = self.policy.lock().unwrap_or_else(|e| e.into_inner());
        // Match the capture worker's policy -> history lock order. Keep history stable
        // through the OS write so edits/privacy changes cannot invalidate a prepared body.
        let history = self
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?;
        if history.revision() != revision {
            return Err(CoreError::new(
                "history_conflict",
                "History changed; refresh before copying or pasting",
            ));
        }
        before_write()?;
        marker.expires = Instant::now() + Duration::from_secs(10);
        match payload.commit() {
            Ok(receipt) => {
                marker.sequence = Some(receipt.sequence);
                if let Some(path) = receipt.temporary_image_path {
                    marker
                        .aliases
                        .push(tiez_storage::database::calc_text_hash(&path));
                }
                *self.echo.lock().unwrap_or_else(|e| e.into_inner()) = Some(marker);
                Ok(ClipboardWriteResult {
                    id,
                    clipboard_sequence: receipt.sequence,
                })
            }
            Err(error) => {
                if error.clipboard_changed {
                    marker.sequence = Some(capture::sequence());
                    *self.echo.lock().unwrap_or_else(|e| e.into_inner()) = Some(marker);
                    Err(CoreError::new(
                        "clipboard_partial",
                        format!("Clipboard changed before the write failed ({})", error.code),
                    ))
                } else {
                    Err(CoreError::new(error.code, "Clipboard write failed"))
                }
            }
        }
    }
}

fn ignored(state: &WorkerState) {
    state
        .stats
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ignored += 1;
}
fn run(mut input: Box<dyn Input>, baseline: u32, state: &WorkerState) -> Result<(), &'static str> {
    let mut last_sequence = baseline;
    let mut pending = None;
    let mut last_burst: Option<(u64, u64, Instant)> = None;
    while !state.stop.flag.load(Ordering::Acquire) {
        let sequence = if let Some(sequence) = pending.take() {
            sequence
        } else {
            let signal = match input.next(Duration::from_millis(100)) {
                Ok(signal) => signal,
                Err(_) if state.stop.flag.load(Ordering::Acquire) => break,
                Err(error) => return Err(error),
            };
            let current = input.sequence();
            if signal.is_none() && current == last_sequence {
                continue;
            }
            current
        };
        if sequence == last_sequence {
            continue;
        }
        state
            .stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .notifications += 1;
        let source = input.source();
        if state.stop.delay(capture::settle_delay(&source)) {
            break;
        }
        let policy = state
            .policy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let read = tiez_content::with_data_directory(&state.directory, || {
            input.read(sequence, source, policy.read, &state.stop.flag)
        });
        let snapshot = match read {
            Err(capture::ReadError::Cancelled) => break,
            Err(capture::ReadError::Changed) => {
                pending = Some(input.sequence());
                continue;
            }
            Ok(None) => {
                ignored(state);
                last_sequence = sequence;
                continue;
            }
            Ok(Some(snapshot)) => snapshot,
        };
        let burst = capture::fingerprint(&snapshot.data);
        if last_burst.is_some_and(|(hash, revision, when)| {
            hash == burst && revision == policy.revision && when.elapsed() < Duration::from_secs(2)
        }) {
            ignored(state);
            last_sequence = sequence;
            continue;
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let prepared = tiez_content::with_data_directory(&state.directory, || {
            let mut entry = processing::discover(
                &snapshot.data,
                &snapshot.source.app_name,
                snapshot.source.process_path.as_deref(),
                timestamp,
            );
            processing::transform(&mut entry, &policy.transform, &state.directory).then_some(entry)
        });
        if state.stop.flag.load(Ordering::Acquire) {
            break;
        }
        if input.sequence() != sequence {
            pending = Some(input.sequence());
            continue;
        }
        let current_policy = state.policy.lock().unwrap_or_else(|e| e.into_inner());
        // A copy can change the clipboard while this read waits for the policy gate.
        if input.sequence() != sequence {
            pending = Some(input.sequence());
            continue;
        }
        if current_policy.revision != policy.revision {
            pending = Some(sequence);
            continue;
        }
        // Holding this immutable policy through commit linearizes accepted saves
        // before a settings update's acknowledgement. No registry lock is needed.
        let Some(entry) = prepared else {
            ignored(state);
            last_sequence = sequence;
            last_burst = Some((burst, policy.revision, Instant::now()));
            continue;
        };
        let echo = state.echo.lock().unwrap_or_else(|e| e.into_inner()).take();
        if echo
            .as_ref()
            .is_some_and(|echo| echo.matches(&entry, snapshot.sequence))
        {
            ignored(state);
            last_sequence = sequence;
            last_burst = Some((burst, policy.revision, Instant::now()));
            continue;
        }
        // A nonmatching clipboard update must not consume the current paste marker.
        if echo
            .as_ref()
            .is_some_and(|echo| Instant::now() < echo.expires)
        {
            *state.echo.lock().unwrap_or_else(|e| e.into_inner()) = echo;
        }
        let result = state
            .history
            .lock()
            .map_err(|_| "capture_history_unavailable")?
            .record_capture(
                entry,
                policy.transform.persistent,
                policy.deduplicate,
                false,
            );
        last_sequence = sequence;
        let mut stats = state.stats.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(_) => {
                stats.accepted += 1;
                stats.last_error = None;
                last_burst = Some((burst, policy.revision, Instant::now()));
            }
            Err(error) => {
                stats.failed += 1;
                stats.last_error = Some(error.code.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SettingsService;
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc::{self, Receiver, Sender};
    use tiez_content::data::ClipboardData;

    struct FakeInput {
        events: Receiver<ClipboardData>,
        pending: Mutex<Option<ClipboardData>>,
        sequence: AtomicU32,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for FakeInput {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }
    impl Input for FakeInput {
        fn sequence(&self) -> u32 {
            self.sequence.load(Ordering::Acquire)
        }
        fn next(&self, timeout: Duration) -> Result<Option<u32>, &'static str> {
            match self.events.recv_timeout(timeout) {
                Ok(data) => {
                    *self.pending.lock().unwrap() = Some(data);
                    Ok(Some(self.sequence.fetch_add(1, Ordering::AcqRel) + 1))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(_) => Err("capture_input_disconnected"),
            }
        }
        fn source(&self) -> ActiveAppInfo {
            ActiveAppInfo {
                app_name: "NativeFixture".into(),
                process_path: Some("C:/fixture.exe".into()),
            }
        }
        fn read(
            &mut self,
            sequence: u32,
            source: ActiveAppInfo,
            _: capture::ReadOptions,
            _: &AtomicBool,
        ) -> Result<Option<capture::CapturedSnapshot>, capture::ReadError> {
            Ok(self
                .pending
                .lock()
                .unwrap()
                .take()
                .map(|data| capture::CapturedSnapshot {
                    sequence,
                    source,
                    data,
                }))
        }
    }
    fn input(service: &mut CaptureService) -> (Sender<ClipboardData>, Arc<AtomicBool>) {
        let (sender, events) = mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        service
            .start_input(
                Box::new(FakeInput {
                    events,
                    pending: Mutex::new(None),
                    sequence: AtomicU32::new(0),
                    dropped: dropped.clone(),
                }),
                0,
            )
            .unwrap();
        (sender, dropped)
    }
    fn fixture(test: impl FnOnce(&mut CaptureService, &mut SettingsService, &std::path::Path)) {
        let directory = std::env::temp_dir().join(format!(
            "TieZ-capture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fixture.db");
        {
            let history = Arc::new(Mutex::new(HistoryService::open(&path).unwrap()));
            let mut settings = SettingsService::open(&path).unwrap();
            let mut service =
                CaptureService::new(history, directory.clone(), &settings.snapshot().unwrap());
            test(&mut service, &mut settings, &path);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    fn wait(service: &CaptureService, done: impl Fn(&CaptureStatus) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = service.status();
            if done(&status) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "capture did not settle: {status:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn set(service: &CaptureService, settings: &mut SettingsService, key: &str, value: &str) {
        let revision = settings.snapshot().unwrap().revision;
        let change = settings
            .update(revision, [(key.into(), value.into())].into())
            .unwrap();
        service.apply_settings(&change.snapshot);
    }
    #[test]
    fn sequence_bound_marker_allows_later_intentional_identical_copy() {
        fixture(|service, _, _| {
            let entry =
                processing::discover(&ClipboardData::Text("same".into()), "fixture", None, 1);
            service.remember_write(&entry);
            let mut marker = service.echo.lock().unwrap().clone().unwrap();
            marker.sequence = Some(42);
            assert!(marker.matches(&entry, 42));
            assert!(!marker.matches(&entry, 43));
        });
    }
    #[test]
    fn clipboard_change_while_acquiring_commit_gate_rejects_stale_read() {
        struct RacingInput {
            checks: std::sync::atomic::AtomicU32,
        }
        impl Input for RacingInput {
            fn sequence(&self) -> u32 {
                if self.checks.fetch_add(1, Ordering::AcqRel) < 2 {
                    1
                } else {
                    2
                }
            }
            fn next(&self, timeout: Duration) -> Result<Option<u32>, &'static str> {
                if self.checks.load(Ordering::Acquire) == 0 {
                    Ok(Some(1))
                } else {
                    thread::sleep(timeout);
                    Ok(None)
                }
            }
            fn source(&self) -> ActiveAppInfo {
                ActiveAppInfo::default()
            }
            fn read(
                &mut self,
                sequence: u32,
                source: ActiveAppInfo,
                _: capture::ReadOptions,
                _: &AtomicBool,
            ) -> Result<Option<capture::CapturedSnapshot>, capture::ReadError> {
                Ok(Some(capture::CapturedSnapshot {
                    sequence,
                    source,
                    data: ClipboardData::Text(
                        if sequence == 1 {
                            "stale before copy"
                        } else {
                            "current after copy"
                        }
                        .into(),
                    ),
                }))
            }
        }
        fixture(|service, _, _| {
            service
                .start_input(
                    Box::new(RacingInput {
                        checks: Default::default(),
                    }),
                    0,
                )
                .unwrap();
            wait(service, |status| status.accepted == 1);
            service.stop();
            let history = service.history.lock().unwrap();
            let page = history.page(Default::default()).unwrap();
            assert_eq!(page.session_count, 1);
            assert_eq!(
                history.entry(page.items[0].id).unwrap().content,
                "current after copy"
            );
            assert_eq!(service.status().accepted, 1);
        });
    }
    #[test]
    fn rejected_precommit_guard_leaves_echo_and_history_unchanged() {
        fixture(|service, _, _| {
            let id = service
                .history
                .lock()
                .unwrap()
                .add_manual("text".into(), "  prepared ".into(), None, vec![])
                .unwrap()
                .id;
            let revision = service
                .history
                .lock()
                .unwrap()
                .page(Default::default())
                .unwrap()
                .revision;
            let prepared = service.prepare_copy(id, false, false).unwrap();
            let error = service
                .commit_copy(prepared, || {
                    Err(CoreError::new("target_changed", "fixture guard"))
                })
                .unwrap_err();
            assert_eq!(error.code, "target_changed");
            assert!(service.echo.lock().unwrap().is_none());
            let history = service.history.lock().unwrap();
            assert_eq!(history.page(Default::default()).unwrap().revision, revision);
            assert_eq!(history.entry(id).unwrap().content, "  prepared ");
        });
    }
    #[test]
    fn copy_revision_rejects_stale_requests_before_preparation() {
        fixture(|service, _, _| {
            let added = service
                .history
                .lock()
                .unwrap()
                .add_manual("text".into(), "  original\r\n ".into(), None, vec![])
                .unwrap();
            assert!(service
                .prepare_copy_at_revision(added.id, false, false, Some(added.revision))
                .is_ok());
            service
                .history
                .lock()
                .unwrap()
                .edit(added.id, "changed".into())
                .unwrap();
            assert_eq!(
                service
                    .copy_entry_at_revision(added.id, false, true, Some(added.revision))
                    .unwrap_err()
                    .code,
                "history_conflict"
            );
            assert!(service.echo.lock().unwrap().is_none());
            assert_eq!(
                service
                    .history
                    .lock()
                    .unwrap()
                    .entry(added.id)
                    .unwrap()
                    .content,
                "changed"
            );
            assert!(service.prepare_copy(added.id, false, false).is_ok());
        });
    }
    #[test]
    fn prepared_copy_rejects_edits_sensitive_changes_and_deletion_before_os_write() {
        for change in 0..3 {
            fixture(|service, _, _| {
                let added = service
                    .history
                    .lock()
                    .unwrap()
                    .add_manual("text".into(), "prepared public".into(), None, vec![])
                    .unwrap();
                let prepared = service
                    .prepare_copy_at_revision(added.id, false, false, Some(added.revision))
                    .unwrap();
                let revision = {
                    let mut history = service.history.lock().unwrap();
                    match change {
                        0 => {
                            history.edit(added.id, "changed body".into()).unwrap();
                        }
                        1 => {
                            history.set_tags(added.id, vec!["password".into()]).unwrap();
                        }
                        _ => {
                            history.delete(added.id).unwrap();
                        }
                    }
                    history.revision()
                };
                let mut called = false;
                let error = service
                    .commit_copy(prepared, || {
                        called = true;
                        Ok(())
                    })
                    .unwrap_err();
                assert_eq!(error.code, "history_conflict");
                assert!(!called);
                assert!(service.echo.lock().unwrap().is_none());
                assert_eq!(service.history.lock().unwrap().revision(), revision);
            });
        }
    }
    #[test]
    fn prepared_session_copy_holds_revision_until_precommit_guard() {
        fixture(|service, _, _| {
            let (sender, _) = input(service);
            sender
                .send(ClipboardData::Text("  session snapshot\r\n ".into()))
                .unwrap();
            wait(service, |status| status.accepted == 1);
            service.stop();
            let page = service
                .history
                .lock()
                .unwrap()
                .page(Default::default())
                .unwrap();
            let id = page.items[0].id;
            assert!(id < 0);
            let prepared = service
                .prepare_copy_at_revision(id, false, false, Some(page.revision))
                .unwrap();
            let error = service
                .commit_copy(prepared, || {
                    assert!(service.history.try_lock().is_err());
                    Err(CoreError::new("fixture_guard", "No OS write in unit test"))
                })
                .unwrap_err();
            assert_eq!(error.code, "fixture_guard");
            assert!(service.echo.lock().unwrap().is_none());
            assert_eq!(service.history.lock().unwrap().revision(), page.revision);
        });
    }
    #[test]
    fn copy_rejects_missing_sensitive_and_invalid_entries_before_os_write() {
        fixture(|service, _, _| {
            assert_eq!(
                service.copy_entry(99999, false, false).unwrap_err().code,
                "not_found"
            );
            let sensitive = service
                .history
                .lock()
                .unwrap()
                .add_manual(
                    "text".into(),
                    "private".into(),
                    None,
                    vec!["sensitive".into()],
                )
                .unwrap()
                .id;
            assert_eq!(
                service
                    .copy_entry(sensitive, false, false)
                    .unwrap_err()
                    .code,
                "sensitive_confirmation"
            );
            let invalid = service
                .history
                .lock()
                .unwrap()
                .add_manual(
                    "image".into(),
                    "data:image/png;base64,bad!".into(),
                    None,
                    vec![],
                )
                .unwrap()
                .id;
            assert_eq!(
                service.copy_entry(invalid, false, false).unwrap_err().code,
                "image_encoding"
            );
        });
    }
    #[test]
    fn worker_preserves_whitespace_source_and_privacy() {
        fixture(|service, _, _| {
            let (sender, _) = input(service);
            sender
                .send(ClipboardData::Text("  中\r\n user@example.com \t".into()))
                .unwrap();
            wait(service, |s| s.accepted == 1);
            let mut history = service.history.lock().unwrap();
            let page = history.page(Default::default()).unwrap();
            assert_eq!(page.session_count, 1);
            let entry = history.entry(page.items[0].id).unwrap();
            assert!(entry.id < 0);
            assert_eq!(entry.content, "  中\n user@example.com \t");
            assert_eq!(entry.source_app, "NativeFixture");
            assert_eq!(entry.source_app_path.as_deref(), Some("C:/fixture.exe"));
            assert!(tiez_storage::database::has_sensitive_tag(&entry.tags));
        });
    }
    #[test]
    fn settings_acknowledgement_changes_persistence_and_promotes_session() {
        fixture(|service, settings, _| {
            let (sender, _) = input(service);
            sender.send(ClipboardData::Text("promote".into())).unwrap();
            wait(service, |s| s.accepted == 1);
            let old = service
                .history
                .lock()
                .unwrap()
                .page(Default::default())
                .unwrap()
                .items[0]
                .id;
            set(service, settings, "app.persistent", "true");
            sender.send(ClipboardData::Text("promote".into())).unwrap();
            wait(service, |s| s.accepted == 2);
            let mut history = service.history.lock().unwrap();
            let page = history.page(Default::default()).unwrap();
            assert_eq!((page.session_count, page.stored_count), (0, 1));
            assert!(page.items[0].id > 0);
            assert!(history.entry(old).is_err());
            assert_eq!(
                history.changes(1).unwrap().changes[0].replaced_id,
                Some(old)
            );
            assert_eq!(
                service.status().policy_revision,
                settings.snapshot().unwrap().revision
            );
        });
    }
    #[test]
    fn bursts_suppress_exact_duplicates_but_keep_distinct_rich_payloads() {
        fixture(|service, settings, _| {
            set(service, settings, "app.deduplicate", "false");
            let (sender, _) = input(service);
            let rich = |html: &str| ClipboardData::RichText {
                text: "same".into(),
                html: html.into(),
            };
            sender.send(rich("<b>same</b>")).unwrap();
            wait(service, |s| s.accepted == 1);
            sender.send(rich("<b>same</b>")).unwrap();
            wait(service, |s| s.ignored == 1);
            sender.send(rich("<i>same</i>")).unwrap();
            wait(service, |s| s.accepted == 2);
            assert_eq!(
                service
                    .history
                    .lock()
                    .unwrap()
                    .page(Default::default())
                    .unwrap()
                    .session_count,
                2
            );
        });
    }
    #[test]
    fn echo_marker_survives_unrelated_nontext_and_suppresses_own_write() {
        fixture(|service, _, _| {
            let echo = processing::discover(
                &ClipboardData::Files(vec!["C:/own.txt".into()]),
                "fixture",
                None,
                1,
            );
            service.remember_write(&echo);
            let (sender, _) = input(service);
            sender
                .send(ClipboardData::Files(vec!["C:/other.txt".into()]))
                .unwrap();
            wait(service, |s| s.accepted == 1);
            sender
                .send(ClipboardData::Files(vec!["C:/own.txt".into()]))
                .unwrap();
            wait(service, |s| s.ignored == 1);
            assert_eq!(service.status().accepted, 1);
        });
    }
    #[test]
    fn persistence_failure_keeps_session_and_reports_only_error_code() {
        fixture(|service, settings, path| {
            let (sender, _) = input(service);
            sender
                .send(ClipboardData::Text("private body".into()))
                .unwrap();
            wait(service, |s| s.accepted == 1);
            let connection = rusqlite::Connection::open(path).unwrap();
            connection.execute_batch("CREATE TRIGGER reject_capture BEFORE INSERT ON clipboard_history BEGIN SELECT RAISE(ABORT,'private body'); END;").unwrap();
            set(service, settings, "app.persistent", "true");
            sender
                .send(ClipboardData::Text("private body".into()))
                .unwrap();
            wait(service, |s| s.failed == 1);
            assert_eq!(service.status().last_error.as_deref(), Some("history"));
            let page = service
                .history
                .lock()
                .unwrap()
                .page(Default::default())
                .unwrap();
            assert_eq!((page.session_count, page.stored_count), (1, 0));
        });
    }
    #[test]
    fn stop_is_idempotent_joins_input_and_allows_restart() {
        fixture(|service, _, _| {
            let (_sender, dropped) = input(service);
            assert!(service.start().unwrap().running); // Running fake input must not start Windows listener.
            assert_eq!(service.status().starts, 1);
            assert!(!service.stop().running);
            assert!(dropped.load(Ordering::Acquire));
            assert!(!service.stop().running);
            let (_sender, dropped) = input(service);
            assert_eq!(service.status().starts, 2);
            service.stop();
            assert!(dropped.load(Ordering::Acquire));
        });
    }
    #[test]
    fn disconnected_input_is_visible_and_drop_joins_worker() {
        fixture(|service, _, _| {
            let (sender, dropped) = input(service);
            drop(sender);
            wait(service, |s| !s.running && s.failed == 1);
            assert_eq!(
                service.status().last_error.as_deref(),
                Some("capture_input_disconnected")
            );
            service.stop();
            assert!(dropped.load(Ordering::Acquire));
        });
    }
}
