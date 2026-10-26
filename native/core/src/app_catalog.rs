//! One runtime-owned, cancellable application scan. Windows calls never hold registry/state locks.
//!
//! The accepted READY snapshot is also the authority for icon requests: [`Snapshot::resolve`] is the
//! only way another service may turn a caller-supplied identity back into a platform application.
//!
//! Generation contract: every accepted [`CatalogService::start`] installs a new job ID — including the
//! cached read that runs no worker — so a consumer holding an older ID is told the catalog changed
//! rather than silently using a snapshot the user can no longer see. [`CatalogService::accepted`] is
//! the read side of that contract: it returns the published snapshot of the current, terminal,
//! uncancelled READY job, and observing it never installs or supersedes a generation on its own.
use crate::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use tiez_native_platform::apps::{self, Content, Kind, RecommendationReason, Unavailable};
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
/// Largest accepted identity string; the icon response bound is derived from this same value.
pub(crate) const MAX_IDENTITY: usize = 4096;
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
impl Application {
    /// Exact authoritative membership: every catalog-owned field must match, including availability.
    fn matches(&self, identity: &CatalogIdentity) -> bool {
        self.source_id == identity.source_id
            && self.handler == identity.handler
            && self.kind == identity.kind.as_str()
            && self.unavailable == identity.unavailable.map(IdentityReason::as_str)
    }
    /// Rebuild the internal row for the platform reader from catalog-owned fields only.
    fn platform(&self) -> apps::Application {
        apps::Application {
            name: self.name.clone(),
            source_id: self.source_id.clone(),
            handler: self.handler.clone(),
            kind: match self.kind {
                "desktop" => Kind::Desktop,
                "packaged" => Kind::Packaged,
                _ => Kind::Unavailable,
            },
            unavailable: self.unavailable.map(|reason| match reason {
                "invalid_identity" => Unavailable::InvalidIdentity,
                "not_native_executable" => Unavailable::NotNativeExecutable,
                _ => Unavailable::MissingTarget,
            }),
        }
    }
}
/// A catalog row identity supplied by a caller, for example when requesting an icon.
///
/// Nothing here is trusted: only exact membership in the accepted READY snapshot makes it usable, so
/// a caller cannot name an arbitrary path, display name, launch target or kind.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CatalogIdentity {
    pub source_id: String,
    #[serde(default)]
    pub handler: Option<String>,
    pub kind: IdentityKind,
    #[serde(default)]
    pub unavailable: Option<IdentityReason>,
}
impl CatalogIdentity {
    /// Bound every identity string and require the published kind/availability pairing.
    pub fn bounded(&self) -> bool {
        let text = |value: &str| value.len() <= MAX_IDENTITY && !value.chars().any(char::is_control);
        text(&self.source_id)
            && self.handler.as_deref().is_none_or(text)
            && match (self.kind, self.unavailable) {
                (IdentityKind::Unavailable, Some(_)) => self.handler.is_none(),
                (IdentityKind::Unavailable, None) => false,
                (_, Some(_)) => false,
                (_, None) => self.handler.is_some(),
            }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    Desktop,
    Packaged,
    Unavailable,
}
impl IdentityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Packaged => "packaged",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityReason {
    MissingTarget,
    InvalidIdentity,
    NotNativeExecutable,
}
impl IdentityReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingTarget => "missing_target",
            Self::InvalidIdentity => "invalid_identity",
            Self::NotNativeExecutable => "not_native_executable",
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
impl Snapshot {
    /// Resolve one caller-supplied identity against the authoritative catalog membership.
    ///
    /// A recommendation may carry an application that is absent from the main list, so both accepted
    /// sets are searched. The returned row is rebuilt from catalog-owned fields; the caller's values
    /// are only ever compared, never used as a path, name or launch target.
    pub fn resolve(&self, identity: &CatalogIdentity) -> Option<apps::Application> {
        self.applications
            .iter()
            .chain(
                self.recommendations
                    .values()
                    .flat_map(|group| group.applications.iter().map(|item| &item.application)),
            )
            .find(|entry| entry.matches(identity))
            .map(Application::platform)
    }
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
    /// A job already cancelled by [`CatalogService::cancel_shutdown`] that still has to be joined.
    joining: Mutex<Option<Job>>,
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
    /// The snapshot of the currently accepted READY job, or `None`.
    ///
    /// `job_id` must be the live, uncancelled, terminal job whose publication was already observed, so
    /// any accepted start — even a cached read that runs no worker — supersedes every request bound to
    /// the previous generation. A worker that is still running, a cancelled job, a failed scan and a
    /// replaced job all report `None`; nothing is cached, published, consumed or re-pointed here. The
    /// state lock is held only for these field reads; no Windows work happens here.
    pub fn accepted(&self, job_id: u64) -> Option<Arc<Snapshot>> {
        let state = self.state.lock().ok()?;
        if state.closed {
            return None;
        }
        let job = state.job.as_ref().filter(|job| job.id == job_id)?;
        if job.cancel.load(Ordering::Acquire) {
            return None;
        }
        if job
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return None;
        }
        let published = job.output.lock().ok()?.clone();
        match published {
            Some(Ok(snapshot)) => Some(snapshot),
            _ => None,
        }
    }
    pub fn start(&self, refresh: bool) -> CoreResult<Value> {
        self.start_with(refresh, discover)
    }
    pub(crate) fn start_with(
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
    /// Stop accepting scans and signal cancellation without waiting for the worker.
    ///
    /// Splitting the signal from the join lets a runtime cancel every owned service before it blocks
    /// on any single worker. The cancelled job is parked in `joining` until [`Self::join_shutdown`].
    pub fn cancel_shutdown(&self) {
        let job = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return;
            }
            state.closed = true;
            state.cache = None;
            let job = state.job.take();
            if let Some(job) = &job {
                job.cancel.store(true, Ordering::Release);
            }
            job
        };
        if let Some(job) = job {
            *self
                .joining
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(job);
        }
    }

    /// Join the worker cancelled by [`Self::cancel_shutdown`], outside every state/registry lock.
    ///
    /// Safe to call repeatedly or concurrently: the parked job is taken by exactly one caller.
    pub fn join_shutdown(&self) {
        let job = self
            .joining
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(mut job) = job {
            if let Some(worker) = job.worker.take() {
                let _ = worker.join();
            }
        }
    }

    pub fn close(&self) {
        self.cancel_shutdown();
        self.join_shutdown();
    }
}
impl Drop for CatalogService {
    fn drop(&mut self) {
        self.close();
    }
}

/// Deterministic catalog fixtures for the cross-module runtime tests.
///
/// Nothing here touches the Shell, the registry or a database: a snapshot is built from explicit rows
/// and installed as the exact state `poll` reports as ready. Only use [`ready`] while no catalog
/// worker is running, because it replaces the retained job.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    /// One catalog row exactly as a scan would publish it.
    pub(crate) fn app(
        name: &str,
        source_id: &str,
        handler: Option<&str>,
        kind: &'static str,
        unavailable: Option<&'static str>,
    ) -> Application {
        Application {
            name: name.into(),
            source_id: source_id.into(),
            handler: handler.map(Into::into),
            kind,
            unavailable,
        }
    }
    /// A snapshot whose `recommended` rows exist only in the "text" recommendation group.
    pub(crate) fn snapshot(applications: Vec<Application>, recommended: Vec<Application>) -> Snapshot {
        let scanned = applications.len();
        let mut recommendations = BTreeMap::new();
        recommendations.insert(
            "text",
            Recommendations {
                applications: recommended
                    .into_iter()
                    .map(|application| Recommendation {
                        application,
                        reason: "association",
                    })
                    .collect(),
                association_error: None,
                unreadable: 0,
                limited: false,
                default_name: None,
            },
        );
        Snapshot {
            applications,
            scanned,
            unreadable: 0,
            duplicates: 0,
            limited: false,
            recommendations,
        }
    }
    /// Install an already-published catalog job and return its generation ID.
    pub(crate) fn ready(
        catalog: &CatalogService,
        applications: Vec<Application>,
        recommended: Vec<Application>,
    ) -> u64 {
        let snapshot = Arc::new(snapshot(applications, recommended));
        let mut state = catalog.state.lock().unwrap();
        let id = state
            .next_id
            .checked_add(1)
            .expect("fixture catalog job identity space");
        state.next_id = id;
        state.job = Some(Job {
            id,
            cached: true,
            cancel: Arc::new(AtomicBool::new(false)),
            output: Arc::new(Mutex::new(Some(Ok(snapshot)))),
            worker: None,
        });
        id
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
    fn cancellation_is_signalled_before_any_worker_is_joined() {
        let service = Arc::new(CatalogService::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        id(service
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
        // Cancellation is its own step: it is observable before anything blocks on the worker.
        service.cancel_shutdown();
        cancel_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(service.state.lock().unwrap().closed);
        assert!(service.state.lock().unwrap().job.is_none());
        assert_eq!(service.start(false).unwrap_err().code, "apps_closed");
        // Joining stays a separate step and still waits for the blocked worker.
        let joining = Arc::clone(&service);
        let join = std::thread::spawn(move || joining.join_shutdown());
        assert!(!join.is_finished());
        release_tx.send(()).unwrap();
        join.join().unwrap();
        // Repeated and post-join shutdown calls are idempotent.
        service.join_shutdown();
        service.close();
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
        _registry_guard: std::sync::MutexGuard<'static, ()>,
    }
    impl RuntimeFixture {
        fn new() -> Self {
            let registry_guard = crate::ABI_TEST_LOCK.lock().unwrap();
            let directory =
                std::env::temp_dir().join(format!("TieZ-apps-abi-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let opened =
                command(json!({"op":"open","database_path":directory.join("settings.db")}))
                    .unwrap();
            Self {
                handle: opened["handle"].as_u64().unwrap(),
                directory,
                _registry_guard: registry_guard,
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
