//! One runtime-owned, cancellable application scan. Windows calls never hold registry/state locks.
use crate::{CoreError, CoreResult};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use tiez_native_platform::apps::{self, Content, Kind, RecommendationReason, Unavailable};
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
#[derive(Serialize)]
pub struct Application {
    name: String,
    source_id: String,
    handler: Option<String>,
    kind: &'static str,
    unavailable: Option<&'static str>,
}
impl From<apps::Application> for Application {
    fn from(app: apps::Application) -> Self {
        Self {
            name: app.name,
            source_id: app.source_id,
            handler: app.handler,
            kind: match app.kind {
                Kind::Desktop => "desktop",
                Kind::Packaged => "packaged",
                Kind::Unavailable => "unavailable",
            },
            unavailable: app.unavailable.map(|reason| match reason {
                Unavailable::MissingTarget => "missing_target",
                Unavailable::InvalidIdentity => "invalid_identity",
                Unavailable::NotNativeExecutable => "not_native_executable",
            }),
        }
    }
}
#[derive(Serialize)]
struct Recommendation {
    application: Application,
    reason: &'static str,
}
#[derive(Serialize)]
struct Recommendations {
    applications: Vec<Recommendation>,
    association_error: Option<&'static str>,
    unreadable: usize,
    limited: bool,
    default_name: Option<String>,
}
#[derive(Serialize, Default)]
pub struct Snapshot {
    applications: Vec<Application>,
    scanned: usize,
    unreadable: usize,
    duplicates: usize,
    limited: bool,
    recommendations: BTreeMap<&'static str, Recommendations>,
}
type Outcome = Result<Arc<Snapshot>, &'static str>;
struct Job {
    id: u64,
    cached: bool,
    cancel: Arc<AtomicBool>,
    output: Arc<Mutex<Option<Outcome>>>,
    worker: Option<JoinHandle<()>>,
}
#[derive(Default)]
struct State {
    next_id: u64,
    closed: bool,
    job: Option<Job>,
    cache: Option<Arc<Snapshot>>,
}
#[derive(Default)]
pub struct CatalogService {
    state: Mutex<State>,
}
fn error(code: &'static str) -> CoreError {
    CoreError::new(
        code,
        match code {
            "apps_busy" => "A previous application scan is still stopping; retry shortly",
            "apps_cancelled" => "Application scan cancelled",
            "apps_job" => "Application scan has been replaced or is unknown",
            "apps_closed" => "Application catalog is closed",
            _ => "Installed application catalog unavailable",
        },
    )
}
fn validate_snapshot(snapshot: Snapshot) -> Outcome {
    let bytes = serde_json::to_vec(&snapshot).map_err(|_| "apps_response")?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err("apps_catalog_size");
    }
    Ok(Arc::new(snapshot))
}
fn discover(cancel: &AtomicBool) -> Result<Snapshot, &'static str> {
    let catalog = apps::scan(cancel)?;
    let mut recommendations = BTreeMap::new();
    for (name, kind) in [
        ("text", Content::Text),
        ("code", Content::Code),
        ("rich_text", Content::RichText),
        ("image", Content::Image),
        ("file", Content::File),
        ("video", Content::Video),
        ("url", Content::Url),
        ("link", Content::Link),
    ] {
        let result = apps::recommendations(kind, &catalog, cancel)?;
        recommendations.insert(
            name,
            Recommendations {
                applications: result
                    .applications
                    .into_iter()
                    .map(|item| Recommendation {
                        application: item.application.into(),
                        reason: match item.reason {
                            RecommendationReason::Association => "association",
                            RecommendationReason::Keyword => "keyword",
                        },
                    })
                    .collect(),
                association_error: result.association_error,
                unreadable: result.unreadable,
                limited: result.limited,
                default_name: result.default_name,
            },
        );
    }
    Ok(Snapshot {
        applications: catalog.applications.into_iter().map(Into::into).collect(),
        scanned: catalog.scanned,
        unreadable: catalog.unreadable,
        duplicates: catalog.duplicates,
        limited: catalog.limited,
        recommendations,
    })
}
impl CatalogService {
    pub fn start(&self, refresh: bool) -> CoreResult<Value> {
        self.start_with(refresh, discover)
    }
    fn start_with(
        &self,
        refresh: bool,
        operation: impl FnOnce(&AtomicBool) -> Result<Snapshot, &'static str> + Send + 'static,
    ) -> CoreResult<Value> {
        let mut state = self.state.lock().map_err(|_| error("apps_unavailable"))?;
        if state.closed {
            return Err(error("apps_closed"));
        }
        if state
            .job
            .as_ref()
            .and_then(|job| job.worker.as_ref())
            .is_some_and(|worker| !worker.is_finished())
        {
            return Err(error("apps_busy"));
        }
        if let Some(mut previous) = state.job.take() {
            if let Some(worker) = previous.worker.take() {
                let _ = worker.join();
            } // Already finished; no Windows wait.
            if !previous.cancel.load(Ordering::Acquire) {
                if let Some(Ok(snapshot)) = previous
                    .output
                    .lock()
                    .map_err(|_| error("apps_unavailable"))?
                    .as_ref()
                {
                    state.cache = Some(Arc::clone(snapshot));
                }
            }
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| error("apps_job_overflow"))?;
        let cached = (!refresh).then(|| state.cache.clone()).flatten();
        let cancel = Arc::new(AtomicBool::new(false));
        let output = Arc::new(Mutex::new(
            cached.as_ref().map(|snapshot| Ok(Arc::clone(snapshot))),
        ));
        let worker = if cached.is_some() {
            None
        } else {
            let output = Arc::clone(&output);
            let cancel = Arc::clone(&cancel);
            Some(
                std::thread::Builder::new()
                    .name("tiez-apps".into())
                    .spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            operation(&cancel).and_then(validate_snapshot)
                        }))
                        .unwrap_or(Err("apps_worker"));
                        let result = if cancel.load(Ordering::Acquire) {
                            Err("apps_cancelled")
                        } else {
                            result
                        };
                        *output
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
                    })
                    .map_err(|_| error("apps_worker"))?,
            )
        };
        state.next_id = id;
        state.job = Some(Job {
            id,
            cached: cached.is_some(),
            cancel,
            output,
            worker,
        });
        Ok(json!({"job_id":id,"cached":cached.is_some()}))
    }
    pub fn poll(&self, id: u64) -> CoreResult<Value> {
        let mut state = self.state.lock().map_err(|_| error("apps_unavailable"))?;
        if state.closed {
            return Err(error("apps_closed"));
        }
        let job = state
            .job
            .as_ref()
            .filter(|job| job.id == id)
            .ok_or_else(|| error("apps_job"))?;
        if job.cancel.load(Ordering::Acquire) {
            return Err(error("apps_cancelled"));
        }
        // A result is ready only after the worker has terminated, so an immediate cached
        // read cannot spuriously report busy during the result-publication/exit interval.
        if job
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Ok(json!({"job_id":id,"state":"running","cached":false,"snapshot":null}));
        }
        let output = job
            .output
            .lock()
            .map_err(|_| error("apps_unavailable"))?
            .clone();
        let cached = job.cached;
        match output {
            None => Ok(json!({"job_id":id,"state":"running","cached":false,"snapshot":null})),
            Some(Err(code)) => Err(error(code)),
            Some(Ok(snapshot)) => {
                state.cache = Some(Arc::clone(&snapshot));
                drop(state);
                Ok(
                    json!({"job_id":id,"state":"ready","cached":cached,"snapshot":snapshot.as_ref()}),
                )
            }
        }
    }
    pub fn cancel(&self, id: u64) -> CoreResult<Value> {
        let state = self.state.lock().map_err(|_| error("apps_unavailable"))?;
        if state.closed {
            return Err(error("apps_closed"));
        }
        let job = state
            .job
            .as_ref()
            .filter(|job| job.id == id)
            .ok_or_else(|| error("apps_job"))?;
        job.cancel.store(true, Ordering::Release);
        Ok(json!({"job_id":id,"cancelled":true}))
    }
    pub fn close(&self) {
        let job = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            state.cache = None;
            let job = state.job.take();
            if let Some(job) = &job {
                job.cancel.store(true, Ordering::Release);
            }
            job
        };
        if let Some(mut job) = job {
            if let Some(worker) = job.worker.take() {
                let _ = worker.join();
            }
        }
    }
}
impl Drop for CatalogService {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    fn wait(service: &CatalogService, id: u64) -> CoreResult<Value> {
        let start = Instant::now();
        loop {
            let value = service.poll(id)?;
            if value["state"] == "ready" {
                return Ok(value);
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn id(value: Value) -> u64 {
        value["job_id"].as_u64().unwrap()
    }
    fn idle(service: &CatalogService) {
        let start = Instant::now();
        loop {
            if service
                .state
                .lock()
                .unwrap()
                .job
                .as_ref()
                .and_then(|job| job.worker.as_ref())
                .is_none_or(JoinHandle::is_finished)
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn running_busy_wrong_ids_and_cancel_do_not_publish_or_spawn_twice() {
        let service = CatalogService::default();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let job = id(service
            .start_with(true, move |_| {
                ready_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(Snapshot::default())
            })
            .unwrap());
        ready_rx.recv().unwrap();
        assert_eq!(service.poll(job).unwrap()["state"], "running");
        assert_eq!(
            service
                .start_with(true, |_| panic!("Second worker"))
                .unwrap_err()
                .code,
            "apps_busy"
        );
        assert_eq!(service.cancel(job + 1).unwrap_err().code, "apps_job");
        assert_eq!(service.poll(job + 1).unwrap_err().code, "apps_job");
        service.cancel(job).unwrap();
        assert_eq!(service.poll(job).unwrap_err().code, "apps_cancelled");
        assert_eq!(
            service
                .start_with(false, |_| panic!("Detached worker"))
                .unwrap_err()
                .code,
            "apps_busy"
        );
        release_tx.send(()).unwrap();
        idle(&service);
        assert!(service.state.lock().unwrap().cache.is_none());
    }
    #[test]
    fn success_caches_without_rerunning_and_refresh_replaces_job_identity() {
        let service = CatalogService::default();
        let job = id(service
            .start_with(false, |_| {
                Ok(Snapshot {
                    scanned: 23,
                    ..Default::default()
                })
            })
            .unwrap());
        assert_eq!(wait(&service, job).unwrap()["snapshot"]["scanned"], 23);
        idle(&service);
        let cached = service
            .start_with(false, |_| panic!("Read-only cache rescanned"))
            .unwrap();
        assert_eq!(cached["cached"], true);
        let next = id(cached);
        assert_ne!(next, job);
        assert_eq!(service.poll(job).unwrap_err().code, "apps_job");
        assert_eq!(service.poll(next).unwrap()["snapshot"]["scanned"], 23);
        let refreshed = id(service
            .start_with(true, |_| {
                Ok(Snapshot {
                    scanned: 24,
                    ..Default::default()
                })
            })
            .unwrap());
        assert_eq!(
            wait(&service, refreshed).unwrap()["snapshot"]["scanned"],
            24
        );
    }
    #[test]
    fn failed_refresh_keeps_previous_cache_and_has_generic_error() {
        let service = CatalogService::default();
        let job = id(service
            .start_with(false, |_| {
                Ok(Snapshot {
                    scanned: 9,
                    ..Default::default()
                })
            })
            .unwrap());
        wait(&service, job).unwrap();
        idle(&service);
        let failed = id(service.start_with(true, |_| Err("apps_folder")).unwrap());
        assert_eq!(wait(&service, failed).unwrap_err().code, "apps_folder");
        idle(&service);
        let cached = id(service.start_with(false, |_| panic!("Lost cache")).unwrap());
        assert_eq!(service.poll(cached).unwrap()["snapshot"]["scanned"], 9);
    }
    #[test]
    fn cancellation_after_completion_cannot_adopt_that_snapshot() {
        let service = CatalogService::default();
        let job = id(service
            .start_with(true, |_| {
                Ok(Snapshot {
                    scanned: 99,
                    ..Default::default()
                })
            })
            .unwrap());
        idle(&service);
        service.cancel(job).unwrap();
        assert_eq!(service.poll(job).unwrap_err().code, "apps_cancelled");
        let next = id(service
            .start_with(false, |_| Ok(Snapshot::default()))
            .unwrap());
        assert_eq!(wait(&service, next).unwrap()["snapshot"]["scanned"], 0);
    }
    #[test]
    fn close_marks_closed_and_releases_state_while_joining_the_actual_worker() {
        let service = Arc::new(CatalogService::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let job = id(service
            .start_with(true, move |cancel| {
                ready_tx.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                cancel_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(Snapshot::default())
            })
            .unwrap());
        ready_rx.recv().unwrap();
        let closing = Arc::clone(&service);
        let worker = std::thread::spawn(move || closing.close());
        cancel_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(service.poll(job).unwrap_err().code, "apps_closed");
        assert_eq!(service.start(false).unwrap_err().code, "apps_closed");
        assert!(!worker.is_finished());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(service.state.lock().unwrap().job.is_none());
        service.close();
    }
    #[test]
    fn worker_panic_is_contained_and_recoverable() {
        let service = CatalogService::default();
        let job = id(service
            .start_with(true, |_| panic!("owned fixture panic"))
            .unwrap());
        assert_eq!(wait(&service, job).unwrap_err().code, "apps_worker");
        idle(&service);
        let next = id(service
            .start_with(true, |_| Ok(Snapshot::default()))
            .unwrap());
        assert_eq!(wait(&service, next).unwrap()["state"], "ready");
    }
    #[test]
    fn job_ids_never_wrap_or_reuse_zero() {
        let service = CatalogService::default();
        service.state.lock().unwrap().next_id = u64::MAX;
        assert_eq!(service.start(false).unwrap_err().code, "apps_job_overflow");
        assert!(service.state.lock().unwrap().job.is_none());
    }
    #[test]
    fn oversized_catalog_is_rejected_before_retaining_or_serializing_a_wire_snapshot() {
        let snapshot = Snapshot {
            applications: vec![Application {
                name: "x".repeat(MAX_SNAPSHOT_BYTES),
                source_id: String::new(),
                handler: None,
                kind: "unavailable",
                unavailable: Some("missing_target"),
            }],
            ..Default::default()
        };
        assert!(matches!(
            validate_snapshot(snapshot),
            Err("apps_catalog_size")
        ));
    }
    fn command(value: Value) -> CoreResult<Value> {
        crate::dispatch(&serde_json::to_vec(&value).unwrap())
    }
    struct RuntimeFixture {
        handle: u64,
        directory: std::path::PathBuf,
    }
    impl RuntimeFixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("TieZ-apps-abi-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let opened =
                command(json!({"op":"open","database_path":directory.join("settings.db")}))
                    .unwrap();
            Self {
                handle: opened["handle"].as_u64().unwrap(),
                directory,
            }
        }
        fn service(&self) -> Arc<CatalogService> {
            Arc::clone(&crate::REGISTRY.get().unwrap().lock().unwrap().services[&self.handle].apps)
        }
    }
    impl Drop for RuntimeFixture {
        fn drop(&mut self) {
            let _ = command(json!({"op":"close","handle":self.handle}));
            use std::os::windows::fs::MetadataExt;
            if self.directory.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .directory
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("TieZ-apps-abi-")
                && std::fs::symlink_metadata(&self.directory)
                    .unwrap()
                    .file_attributes()
                    & 0x400
                    == 0
            {
                std::fs::remove_dir_all(&self.directory).unwrap();
            }
        }
    }
    #[test]
    fn abi_catalog_schema_cache_unknown_fields_and_readonly_state() {
        let fixture = RuntimeFixture::new();
        let handle = fixture.handle;
        let service = fixture.service();
        assert_eq!(
            command(json!({"op":"apps_poll","handle":0,"job_id":1}))
                .unwrap_err()
                .code,
            "invalid_handle"
        );
        assert_eq!(
            command(json!({"op":"apps_start","handle":handle,"command":"not accepted"}))
                .unwrap_err()
                .code,
            "invalid_request"
        );
        assert!(service.state.lock().unwrap().job.is_none());
        let job = id(service
            .start_with(false, |_| {
                Ok(Snapshot {
                    scanned: 1,
                    applications: vec![Application {
                        name: "自有编辑器".into(),
                        source_id: "Opaque.Editor".into(),
                        handler: Some(r"C:\owned editor.exe".into()),
                        kind: "desktop",
                        unavailable: None,
                    }],
                    ..Default::default()
                })
            })
            .unwrap());
        wait(&service, job).unwrap();
        idle(&service);
        let cached = command(json!({"op":"apps_start","handle":handle})).unwrap();
        let next = id(cached.clone());
        assert_eq!(cached["cached"], true);
        let result = command(json!({"op":"apps_poll","handle":handle,"job_id":next})).unwrap();
        assert_eq!(
            result["snapshot"]["applications"][0]["source_id"],
            "Opaque.Editor"
        );
        assert_eq!(
            result["snapshot"]["applications"][0]["handler"],
            r"C:\owned editor.exe"
        );
        assert_eq!(result["snapshot"]["scanned"], 1);
        assert_eq!(result["cached"], true);
        assert_eq!(
            command(json!({"op":"snapshot","handle":handle})).unwrap()["snapshot"]["revision"],
            0
        );
        assert_eq!(
            command(json!({"op":"capture_status","handle":handle})).unwrap()["starts"],
            0
        );
        assert_eq!(
            command(json!({"op":"apps_poll","handle":handle,"job_id":job}))
                .unwrap_err()
                .code,
            "apps_job"
        );
        command(json!({"op":"close","handle":handle})).unwrap();
        assert_eq!(
            command(json!({"op":"apps_cancel","handle":handle,"job_id":next}))
                .unwrap_err()
                .code,
            "invalid_handle"
        );
        let opened =
            command(json!({"op":"open","database_path":fixture.directory.join("settings.db")}))
                .unwrap();
        let reopened = opened["handle"].as_u64().unwrap();
        assert!(
            crate::REGISTRY.get().unwrap().lock().unwrap().services[&reopened]
                .apps
                .state
                .lock()
                .unwrap()
                .cache
                .is_none()
        );
        command(json!({"op":"close","handle":reopened})).unwrap();
    }
    #[test]
    fn abi_catalog_releases_registry_and_close_cancels_before_joining() {
        let fixture = RuntimeFixture::new();
        let handle = fixture.handle;
        let service = fixture.service();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let job = id(service
            .start_with(true, move |cancel| {
                ready_tx.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                cancel_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(Snapshot::default())
            })
            .unwrap());
        ready_rx.recv().unwrap();
        assert_eq!(
            command(json!({"op":"snapshot","handle":handle})).unwrap()["snapshot"]["revision"],
            0
        );
        assert_eq!(
            command(json!({"op":"apps_start","handle":handle,"refresh":true}))
                .unwrap_err()
                .code,
            "apps_busy"
        );
        let close = std::thread::spawn(move || command(json!({"op":"close","handle":handle})));
        cancel_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            command(json!({"op":"apps_poll","handle":handle,"job_id":job}))
                .unwrap_err()
                .code,
            "core_closing"
        );
        assert_eq!(
            command(json!({"op":"open","database_path":fixture.directory.join("settings.db")}))
                .unwrap_err()
                .code,
            "already_open"
        );
        assert!(!close.is_finished());
        release_tx.send(()).unwrap();
        assert_eq!(close.join().unwrap().unwrap()["closed"], true);
        assert!(service.state.lock().unwrap().closed);
        assert!(service.state.lock().unwrap().job.is_none());
    }
    #[test]
    fn published_output_is_not_ready_until_worker_thread_termination() {
        let service = CatalogService::default();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || release_rx.recv().unwrap());
        service.state.lock().unwrap().job = Some(Job {
            id: 1,
            cached: false,
            cancel: Arc::new(AtomicBool::new(false)),
            output: Arc::new(Mutex::new(Some(Ok(Arc::new(Snapshot::default()))))),
            worker: Some(worker),
        });
        assert_eq!(service.poll(1).unwrap()["state"], "running");
        assert!(service.state.lock().unwrap().cache.is_none());
        release_tx.send(()).unwrap();
        idle(&service);
        assert_eq!(service.poll(1).unwrap()["state"], "ready");
        assert_eq!(
            service
                .start_with(false, |_| panic!("Ready worker was not cached"))
                .unwrap()["cached"],
            true
        );
    }
}
