//! Bounded, cancellable application icon jobs for the shared runtime.
//!
//! A request never names a path, display name, kind, launch target or pixels: it carries only the
//! catalog identity it was shown plus the catalog job that published it. [`crate::app_catalog`]
//! resolves that identity against its authoritative READY snapshot, so an unknown, mismatched,
//! unavailable, cancelled, non-ready or superseded identity is rejected before the Windows reader
//! runs. Publication re-checks the same catalog generation, so a stale worker that ignored
//! cancellation can never have its raster adopted.
//!
//! Acceptance and publication:
//!
//! - [`IconService::start`] is the acceptance point. It reads the generation through
//!   `CatalogService::accepted`, resolves the identity there, and records the request under one state
//!   lock. A request is therefore either accepted against exactly that generation or rejected
//!   (`icons_catalog`, `icons_identity`, `icons_unavailable`, `icons_edge`) without retaining a slot.
//! - [`IconService::poll`] is the publication point. A raster is returned only after the worker
//!   terminated, the raster was validated against the request edge, the job was not cancelled, and
//!   the same generation still resolves the same identity. Cancellation before or after completion —
//!   including a worker that ignored the flag — reports `icons_cancelled` and never publishes pixels.
//!
//! Runtime contract:
//!
//! - At most [`MAX_JOBS`] retained jobs: no unbounded queue and no extra workers. A third request
//!   reports `icons_busy`; a terminal slot (claimed, or cancelled once its worker terminated) is
//!   reaped deterministically, and job IDs are checked and never reused.
//! - The blocking Shell/COM/GDI read runs on its own worker and holds no service, catalog, history,
//!   settings or registry lock. `poll` never waits for the worker.
//! - Cancellation and close are separate signals: [`IconService::cancel_shutdown`] only signals and
//!   [`IconService::join_shutdown`] only joins, so a runtime can cancel every owned service before it
//!   blocks on any single worker.
//! - The caller must `poll` or `cancel` every job it started: a finished job keeps its slot until its
//!   result is claimed or it is cancelled. A finished but unclaimed result bound to a superseded
//!   catalog generation is reclaimed automatically, so a refresh cannot permanently fill both slots.
//! - Nothing is cached between requests, and no raster is retained after its slot is released.

use crate::app_catalog::{CatalogIdentity, CatalogService};
use crate::{CoreError, CoreResult};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use tiez_native_platform::app_icons::{self as shell, Error as ShellError, Raster};
use tiez_native_platform::apps::{Application, Kind};

/// Retained job slots. Two bounded workers keep one visible grid request and one prefetch alive.
const MAX_JOBS: usize = 2;
/// Platform raster contract: largest accepted/returned edge and largest raster byte payload.
const MAX_EDGE: u32 = shell::MAX_EDGE;
const MAX_PIXELS: usize = shell::MAX_BYTES;
/// Worst-case JSON wire form of one raster payload: every byte is at most three digits plus the
/// delimiter that follows it, and the array carries one bracket pair.
const MAX_PIXEL_WIRE: usize = MAX_PIXELS * 4 + 2;
/// Worst-case JSON wire form of the echoed identity: two bounded strings, each of which can grow to
/// twice its byte length once quotes and backslashes are escaped.
const MAX_IDENTITY_WIRE: usize = crate::app_catalog::MAX_IDENTITY * 2 * 2;
/// The declared bounded response: the largest valid raster, the largest accepted identity and the
/// fixed keys, numeric fields and enum/state strings of the response object.
const MAX_RESPONSE_BYTES: usize = MAX_PIXEL_WIRE + MAX_IDENTITY_WIRE + 4096;

type Outcome = Result<Arc<Raster>, &'static str>;
type Reader = Arc<dyn Fn(&Application, u32, &AtomicBool) -> Result<Raster, ShellError> + Send + Sync>;

struct Job {
    id: u64,
    catalog_job_id: u64,
    edge: u32,
    identity: CatalogIdentity,
    cancel: Arc<AtomicBool>,
    output: Arc<Mutex<Option<Outcome>>>,
    worker: Option<JoinHandle<()>>,
    claimed: bool,
}

impl Job {
    /// A slot is free again only once its worker terminated and its result was delivered or cancelled.
    fn reapable(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
            && (self.claimed || self.cancel.load(Ordering::Acquire))
    }

    fn join(self) {
        if let Some(worker) = self.worker {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
struct State {
    next_id: u64,
    closed: bool,
    jobs: Vec<Job>,
}

pub struct IconService {
    catalog: Arc<CatalogService>,
    reader: Mutex<Reader>,
    state: Mutex<State>,
    /// Workers already signalled by `cancel_shutdown` that `join_shutdown` still has to join.
    joining: Mutex<Vec<Job>>,
}

fn error(code: &'static str) -> CoreError {
    CoreError::new(
        code,
        match code {
            "icons_bounds" => "Application icon raster exceeds the requested edge or the bounded raster contract",
            "icons_busy" => "Both application icon slots are busy; retry after polling or cancelling",
            "icons_cancelled" => "Application icon request cancelled",
            "icons_catalog" => "Application catalog changed; restart from a current catalog snapshot",
            "icons_closed" => "Application icon service is closed",
            "icons_edge" => "Application icon edge must be 1..=128 physical pixels",
            "icons_identity" => "Requested application identity is not in the current catalog snapshot",
            "icons_job" => "Application icon job has been replaced or is unknown",
            "icons_overflow" => "Application icon job identity space exhausted",
            "icons_response" => "Application icon response exceeds the bounded wire contract",
            "icons_unavailable" => "This catalog entry cannot provide an application icon",
            "icons_windows" => "Windows Shell application icon lookup failed",
            "icons_worker" => "Application icon worker failed",
            _ => "Application icon service unavailable",
        },
    )
}

fn platform_code(failure: &ShellError) -> &'static str {
    match failure {
        ShellError::Cancelled => "icons_cancelled",
        ShellError::InvalidRequest => "icons_edge",
        ShellError::Unavailable(_) => "icons_unavailable",
        ShellError::InvalidIdentity => "icons_identity",
        ShellError::OutOfBounds => "icons_bounds",
        ShellError::Windows { .. } => "icons_windows",
    }
}

/// Validate a backend raster against the platform contract and the requested edge before it can be
/// retained or serialized as a response.
///
/// A backend that returns a larger raster than the requested edge is a contract violation, not a
/// larger icon: it is rejected here so the caller can never observe pixels it did not ask for.
fn bounded(raster: Raster, edge: u32) -> Outcome {
    let bytes = (raster.width as usize)
        .checked_mul(raster.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if raster.width == 0
        || raster.height == 0
        || raster.width > edge
        || raster.height > edge
        || raster.width > MAX_EDGE
        || raster.height > MAX_EDGE
        || bytes != Some(raster.rgba.len())
        || raster.rgba.len() > MAX_PIXELS
    {
        return Err("icons_bounds");
    }
    Ok(Arc::new(raster))
}

/// Collect every reaped slot so the caller can join them without holding the state lock.
fn take_reapable(state: &mut State) -> Vec<Job> {
    let mut reaped = Vec::new();
    let mut index = 0;
    while index < state.jobs.len() {
        if state.jobs[index].reapable() {
            reaped.push(state.jobs.remove(index));
        } else {
            index += 1;
        }
    }
    reaped
}

/// Run one synchronous Shell read on its own worker.
///
/// No service lock is held here: the worker only writes its own slot output, so a blocked
/// extraction can never stop `start`, `poll`, `cancel` or an unrelated runtime call.
fn run(
    reader: &Reader,
    application: &Application,
    edge: u32,
    cancel: &Arc<AtomicBool>,
    output: &Arc<Mutex<Option<Outcome>>>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reader(application, edge, cancel)
    }));
    let mut outcome = match result {
        Ok(Ok(raster)) => bounded(raster, edge),
        Ok(Err(failure)) => Err(platform_code(&failure)),
        Err(_) => Err("icons_worker"),
    };
    // A completion that arrives after cancellation is never published as a usable raster.
    if cancel.load(Ordering::Acquire) {
        outcome = Err("icons_cancelled");
    }
    *output.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
}

impl IconService {
    pub fn new(catalog: Arc<CatalogService>) -> Self {
        let reader: Reader = Arc::new(
            shell::read as fn(&Application, u32, &AtomicBool) -> Result<Raster, ShellError>,
        );
        Self {
            catalog,
            reader: Mutex::new(reader),
            state: Mutex::new(State::default()),
            joining: Mutex::new(Vec::new()),
        }
    }

    /// Accept one bounded icon request for an identity in the currently accepted catalog job.
    pub fn start(
        &self,
        catalog_job_id: u64,
        identity: &CatalogIdentity,
        edge: u32,
    ) -> CoreResult<Value> {
        if !(1..=MAX_EDGE).contains(&edge) {
            return Err(error("icons_edge"));
        }
        if !identity.bounded() {
            return Err(error("icons_identity"));
        }
        // Authoritative membership is resolved, and the catalog generation read, before a slot is
        // reserved or any Windows work is queued.
        let snapshot = self
            .catalog
            .accepted(catalog_job_id)
            .ok_or_else(|| error("icons_catalog"))?;
        let application = snapshot
            .resolve(identity)
            .ok_or_else(|| error("icons_identity"))?;
        if application.kind == Kind::Unavailable || application.unavailable.is_some() {
            return Err(error("icons_unavailable"));
        }
        let reader = Arc::clone(&*self.reader.lock().map_err(|_| error("icons_unavailable"))?);
        loop {
            let mut state = self.state.lock().map_err(|_| error("icons_unavailable"))?;
            if state.closed {
                return Err(error("icons_closed"));
            }
            let reaped = take_reapable(&mut state);
            if !reaped.is_empty() {
                drop(state);
                reaped.into_iter().for_each(Job::join);
                continue;
            }
            if state.jobs.len() >= MAX_JOBS {
                // A finished result whose catalog generation was superseded can never be adopted, so
                // reclaim its slot instead of letting a refresh permanently fill the bounded capacity.
                let finished: Vec<(u64, u64)> = state
                    .jobs
                    .iter()
                    .filter(|job| {
                        !job.claimed && job.worker.as_ref().is_none_or(JoinHandle::is_finished)
                    })
                    .map(|job| (job.id, job.catalog_job_id))
                    .collect();
                drop(state);
                let stale: Vec<u64> = finished
                    .into_iter()
                    .filter(|(_, generation)| self.catalog.accepted(*generation).is_none())
                    .map(|(id, _)| id)
                    .collect();
                if stale.is_empty() {
                    return Err(error("icons_busy"));
                }
                let mut state = self.state.lock().map_err(|_| error("icons_unavailable"))?;
                for job in &mut state.jobs {
                    if stale.contains(&job.id) {
                        job.claimed = true;
                    }
                }
                continue;
            }
            let id = state
                .next_id
                .checked_add(1)
                .ok_or_else(|| error("icons_overflow"))?;
            let cancel = Arc::new(AtomicBool::new(false));
            let output = Arc::new(Mutex::new(None));
            // Spawn and record under the same lock: a concurrent close can never miss a worker.
            let worker = {
                let application = application.clone();
                let reader = Arc::clone(&reader);
                let cancel = Arc::clone(&cancel);
                let output = Arc::clone(&output);
                std::thread::Builder::new()
                    .name("tiez-icons".into())
                    .spawn(move || run(&reader, &application, edge, &cancel, &output))
                    .map_err(|_| error("icons_worker"))?
            };
            state.next_id = id;
            state.jobs.push(Job {
                id,
                catalog_job_id,
                edge,
                identity: identity.clone(),
                cancel,
                output,
                worker: Some(worker),
                claimed: false,
            });
            return Ok(json!({
                "job_id": id, "catalog_job_id": catalog_job_id, "edge": edge, "state": "running"
            }));
        }
    }

    /// Non-blocking status/result read. A raster is ready only after its worker terminated.
    pub fn poll(&self, job_id: u64) -> CoreResult<Value> {
        let mut state = self.state.lock().map_err(|_| error("icons_unavailable"))?;
        if state.closed {
            return Err(error("icons_closed"));
        }
        let index = state
            .jobs
            .iter()
            .position(|job| job.id == job_id)
            .ok_or_else(|| error("icons_job"))?;
        if state.jobs[index].cancel.load(Ordering::Acquire) {
            state.jobs[index].claimed = true;
            return Err(error("icons_cancelled"));
        }
        if state.jobs[index]
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Ok(json!({
                "job_id": job_id, "catalog_job_id": state.jobs[index].catalog_job_id,
                "edge": state.jobs[index].edge, "state": "running"
            }));
        }
        let output = state.jobs[index]
            .output
            .lock()
            .map_err(|_| error("icons_unavailable"))?
            .clone();
        let (catalog_job_id, edge, identity) = {
            let job = &mut state.jobs[index];
            job.claimed = true;
            (job.catalog_job_id, job.edge, job.identity.clone())
        };
        // A result is published only after the worker's Windows/COM/GDI scopes are released.
        let Some(outcome) = output else {
            return Ok(json!({
                "job_id": job_id, "catalog_job_id": catalog_job_id, "edge": edge,
                "state": "running"
            }));
        };
        let raster = outcome.map_err(error)?;
        drop(state);
        // Recheck the catalog generation at publication. A catalog replacement, cancellation or
        // failed refresh invalidates the request, so an old icon is never adopted across a change.
        if self
            .catalog
            .accepted(catalog_job_id)
            .is_none_or(|snapshot| snapshot.resolve(&identity).is_none())
        {
            return Err(error("icons_catalog"));
        }
        let value = json!({
            "job_id": job_id, "catalog_job_id": catalog_job_id, "state": "ready", "edge": edge,
            "identity": identity, "width": raster.width, "height": raster.height,
            "pixels": &raster.rgba
        });
        if serde_json::to_vec(&value)
            .map_err(|_| error("icons_response"))?
            .len()
            > MAX_RESPONSE_BYTES
        {
            return Err(error("icons_response"));
        }
        Ok(value)
    }

    /// Cancel one owned job. The blocking phase already in flight completes; its result is discarded.
    pub fn cancel(&self, job_id: u64) -> CoreResult<Value> {
        let mut state = self.state.lock().map_err(|_| error("icons_unavailable"))?;
        if state.closed {
            return Err(error("icons_closed"));
        }
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.id == job_id)
            .ok_or_else(|| error("icons_job"))?;
        job.cancel.store(true, Ordering::Release);
        Ok(json!({ "job_id": job_id, "cancelled": true }))
    }

    /// Stop accepting work and signal cancellation for every owned job, without joining any worker.
    ///
    /// The state lock is released before the call returns, and the signalled jobs are parked for
    /// [`Self::join_shutdown`], so a runtime can cancel every owned service before it blocks.
    pub fn cancel_shutdown(&self) {
        let jobs = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.closed = true;
            let jobs = std::mem::take(&mut state.jobs);
            for job in &jobs {
                job.cancel.store(true, Ordering::Release);
            }
            jobs
        };
        if !jobs.is_empty() {
            self.joining
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(jobs);
        }
    }

    /// Join the workers signalled by [`Self::cancel_shutdown`], outside every state lock.
    ///
    /// Safe to call repeatedly or concurrently: each worker is joined by the one caller that takes it.
    pub fn join_shutdown(&self) {
        let jobs = std::mem::take(
            &mut *self
                .joining
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        jobs.into_iter().for_each(Job::join);
    }

    /// Stop accepting requests, cancel every owned job and join the actual workers.
    ///
    /// The state lock is released before any join, so a blocked Shell extraction cannot delay
    /// unrelated runtime calls or the registry. Repeated calls are safe.
    pub fn close(&self) {
        self.cancel_shutdown();
        self.join_shutdown();
    }
}

impl Drop for IconService {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
impl IconService {
    fn with_reader(catalog: Arc<CatalogService>, reader: Reader) -> Self {
        Self {
            catalog,
            reader: Mutex::new(reader),
            state: Mutex::new(State::default()),
            joining: Mutex::new(Vec::new()),
        }
    }

    /// Swap the Windows reader for a controlled fixture reader; test builds only.
    fn replace_reader(&self, reader: Reader) {
        *self.reader.lock().unwrap() = reader;
    }

    /// Force the job identity space to its final value to exercise overflow reporting.
    fn exhaust_ids(&self) {
        self.state.lock().unwrap().next_id = u64::MAX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_catalog::{
        fixtures, CatalogIdentity, CatalogService, IdentityKind, IdentityReason, MAX_IDENTITY,
    };
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::Condvar;
    use std::time::{Duration, Instant};

    /// Catalog handler shape used by the fixtures: never an existing file, so a real reader refuses
    /// it with `InvalidIdentity` before any Shell/COM phase exists.
    const EXE: &str = r"C:\owned icons\editor.exe";
    const AUMID: &str = "Owned.Package_1!App";
    const RECOMMENDED: &str = r"C:\owned icons\recommended.exe";
    /// Upper bound for every controlled observation in these tests.
    const WAIT: Duration = Duration::from_secs(5);

    fn identity(
        source_id: &str,
        handler: Option<&str>,
        kind: IdentityKind,
        unavailable: Option<IdentityReason>,
    ) -> CatalogIdentity {
        CatalogIdentity {
            source_id: source_id.into(),
            handler: handler.map(Into::into),
            kind,
            unavailable,
        }
    }

    fn desktop() -> CatalogIdentity {
        identity("Opaque.Editor", Some(EXE), IdentityKind::Desktop, None)
    }

    fn packaged() -> CatalogIdentity {
        identity(AUMID, Some(AUMID), IdentityKind::Packaged, None)
    }

    fn missing() -> CatalogIdentity {
        identity(
            "Opaque.Missing",
            None,
            IdentityKind::Unavailable,
            Some(IdentityReason::MissingTarget),
        )
    }

    /// An authoritative catalog generation that also has one recommendation-only application.
    fn catalog() -> (Arc<CatalogService>, u64) {
        let service = Arc::new(CatalogService::default());
        let generation = fixtures::ready(
            &service,
            vec![
                fixtures::app("Editor", "Opaque.Editor", Some(EXE), "desktop", None),
                fixtures::app("Package", AUMID, Some(AUMID), "packaged", None),
                fixtures::app(
                    "Missing",
                    "Opaque.Missing",
                    None,
                    "unavailable",
                    Some("missing_target"),
                ),
            ],
            vec![fixtures::app(
                "Recommended",
                "Opaque.Recommended",
                Some(RECOMMENDED),
                "desktop",
                None,
            )],
        );
        (service, generation)
    }

    /// What a controlled worker reply does.
    enum Reply {
        Raster(u32, u32, Vec<u8>),
        Fail(ShellError),
        Panic,
    }

    #[derive(Default)]
    struct GateQueue {
        queued: VecDeque<(Option<String>, Reply)>,
        open: bool,
    }

    /// Hands controlled replies to blocked fixture readers, addressed by catalog source ID.
    ///
    /// A fixture reader blocks until a reply addressed to its own row arrives, exactly like a
    /// blocking Shell phase, and production cancellation is deliberately not observed here: a
    /// delayed completion that ignores the flag stays reproducible.
    struct Gate {
        queue: Mutex<GateQueue>,
        ready: Condvar,
    }

    impl Gate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                queue: Mutex::new(GateQueue {
                    queued: VecDeque::new(),
                    open: true,
                }),
                ready: Condvar::new(),
            })
        }

        fn push(&self, target: &str, reply: Reply) {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.queued.push_back((Some(target.to_string()), reply));
            self.ready.notify_all();
        }

        /// Unblock every pending reader without a reply; they report a cancelled blocking phase.
        fn release(&self) {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.open = false;
            queue.queued.clear();
            self.ready.notify_all();
        }

        fn take(&self, source: &str) -> Result<Reply, ()> {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if let Some(index) = queue
                    .queued
                    .iter()
                    .position(|(target, _)| target.as_deref().is_none_or(|value| value == source))
                {
                    let (_, reply) = queue.queued.remove(index).expect("indexed entry");
                    return Ok(reply);
                }
                if !queue.open {
                    return Err(());
                }
                queue = self.ready.wait(queue).unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    #[derive(Default)]
    struct Calls {
        observed: Mutex<Vec<(String, u32)>>,
    }

    /// The controlled reader plus the observations and replies of one fixture service.
    struct Controlled {
        gate: Arc<Gate>,
        calls: Arc<Calls>,
        reader: Reader,
    }

    impl Controlled {
        fn new() -> Self {
            let gate = Gate::new();
            let calls = Arc::new(Calls::default());
            let reader: Reader = {
                let gate = Arc::clone(&gate);
                let calls = Arc::clone(&calls);
                Arc::new(
                    move |application: &Application, edge: u32, _cancel: &AtomicBool| {
                        calls
                            .observed
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push((application.source_id.clone(), edge));
                        match gate.take(&application.source_id) {
                            Ok(Reply::Raster(width, height, rgba)) => {
                                Ok(Raster { width, height, rgba })
                            }
                            Ok(Reply::Fail(failure)) => Err(failure),
                            Ok(Reply::Panic) => panic!("controlled icon reader panic"),
                            // The fixture released the gate: behave like an aborted blocking phase.
                            Err(()) => Err(ShellError::Cancelled),
                        }
                    },
                )
            };
            Self {
                gate,
                calls,
                reader,
            }
        }

        fn reply(&self, source: &str, reply: Reply) {
            self.gate.push(source, reply);
        }

        fn release(&self) {
            self.gate.release();
        }

        /// Wait until exactly the expected `(source_id, edge)` calls were made, in any order.
        ///
        /// Worker arrival order is not deterministic, so the comparison is deliberately unordered
        /// while still requiring the exact call multiset.
        fn expect(&self, expected: &[(&str, u32)]) {
            let start = Instant::now();
            loop {
                if self.calls.observed.lock().unwrap().len() >= expected.len() {
                    break;
                }
                assert!(
                    start.elapsed() < WAIT,
                    "icon reader calls: {:?}",
                    self.calls.observed.lock().unwrap()
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            let mut observed = self.calls.observed.lock().unwrap().clone();
            observed.sort();
            let mut wanted: Vec<(String, u32)> = expected
                .iter()
                .map(|(source, edge)| (source.to_string(), *edge))
                .collect();
            wanted.sort();
            assert_eq!(observed, wanted);
        }

        fn silent(&self) {
            assert!(
                self.calls.observed.lock().unwrap().is_empty(),
                "the Windows reader was reached"
            );
        }
    }

    /// Unblocks every controlled reader on unwind, so a failing assertion can never leave a worker
    /// waiting behind a dropped reply sender.
    impl Drop for Controlled {
        fn drop(&mut self) {
            self.gate.release();
        }
    }

    /// A controlled service that always releases its fixture readers before the service joins them,
    /// including while a panic unwinds.
    struct Harness {
        controlled: Controlled,
        icons: Arc<IconService>,
    }

    impl Harness {
        fn new(catalog: Arc<CatalogService>) -> Self {
            let controlled = Controlled::new();
            let icons = Arc::new(IconService::with_reader(
                catalog,
                Arc::clone(&controlled.reader),
            ));
            Self { controlled, icons }
        }

        fn start(
            &self,
            generation: u64,
            identity: &CatalogIdentity,
            edge: u32,
        ) -> CoreResult<Value> {
            self.icons.start(generation, identity, edge)
        }

        fn release(&self) {
            self.controlled.release();
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            // Independent of production job cancellation: the fixture release only ends the blocked
            // read, it cannot turn a failing or missing assertion into a pass.
            self.controlled.release();
            self.icons.close();
        }
    }

    /// Sends one release signal when dropped, including during unwinding.
    struct Unblock(Option<mpsc::Sender<()>>);

    impl Unblock {
        fn new(sender: mpsc::Sender<()>) -> Self {
            Self(Some(sender))
        }

        fn fire(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    impl Drop for Unblock {
        fn drop(&mut self) {
            self.fire();
        }
    }

    fn job_id(value: &Value) -> u64 {
        value["job_id"].as_u64().unwrap()
    }

    /// Poll until the job leaves `running`, bounded so a stuck worker fails instead of hanging.
    fn settled(icons: &IconService, id: u64) -> CoreResult<Value> {
        let start = Instant::now();
        loop {
            match icons.poll(id) {
                Ok(value) if value["state"] == "ready" => return Ok(value),
                Ok(_) => {}
                Err(failure) => return Err(failure),
            }
            assert!(
                start.elapsed() < WAIT,
                "icon job did not settle"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Wait until the job's worker terminated, without asserting any timing.
    fn idle(icons: &IconService, id: u64) {
        let start = Instant::now();
        loop {
            let done = {
                let state = icons.state.lock().unwrap();
                state
                    .jobs
                    .iter()
                    .find(|job| job.id == id)
                    .is_none_or(|job| job.worker.as_ref().is_none_or(JoinHandle::is_finished))
            };
            if done {
                return;
            }
            assert!(start.elapsed() < WAIT, "icon worker stuck");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn await_closed(icons: &IconService) {
        let start = Instant::now();
        while !icons.state.lock().unwrap().closed {
            assert!(
                start.elapsed() < WAIT,
                "icon service did not close"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn membership_is_exact_and_covers_recommendation_only_rows() {
        let (catalog, generation) = catalog();
        let snapshot = catalog.accepted(generation).unwrap();
        let resolved = snapshot.resolve(&desktop()).unwrap();
        assert_eq!(resolved.source_id, "Opaque.Editor");
        assert_eq!(resolved.handler.as_deref(), Some(EXE));
        assert_eq!(resolved.kind, Kind::Desktop);
        assert_eq!(resolved.unavailable, None);
        // The catalog's own display name is used, never anything supplied by the caller.
        assert_eq!(resolved.name, "Editor");
        assert_eq!(snapshot.resolve(&packaged()).unwrap().kind, Kind::Packaged);
        // A recommendation may carry an application absent from the main list.
        let recommended = identity(
            "Opaque.Recommended",
            Some(RECOMMENDED),
            IdentityKind::Desktop,
            None,
        );
        assert_eq!(
            snapshot.resolve(&recommended).unwrap().source_id,
            "Opaque.Recommended"
        );
        // A catalog row that exists but is marked unavailable still resolves; `start` refuses it.
        assert_eq!(
            snapshot.resolve(&missing()).unwrap().unavailable,
            Some(tiez_native_platform::apps::Unavailable::MissingTarget)
        );
        for candidate in [
            identity("Opaque.Unknown", Some(EXE), IdentityKind::Desktop, None),
            identity(
                "Opaque.Editor",
                Some(r"C:\owned icons\other.exe"),
                IdentityKind::Desktop,
                None,
            ),
            identity("Opaque.Editor", Some(EXE), IdentityKind::Packaged, None),
            identity("Opaque.Editor", None, IdentityKind::Unavailable, None),
            identity(
                "Opaque.Editor",
                None,
                IdentityKind::Unavailable,
                Some(IdentityReason::NotNativeExecutable),
            ),
            identity(
                "Opaque.Missing",
                None,
                IdentityKind::Unavailable,
                Some(IdentityReason::NotNativeExecutable),
            ),
            identity(
                "Opaque.Missing",
                Some(EXE),
                IdentityKind::Unavailable,
                Some(IdentityReason::MissingTarget),
            ),
        ] {
            assert!(snapshot.resolve(&candidate).is_none(), "{candidate:?}");
        }
        // Identity strings are bounded before any Windows call.
        let long = identity(
            &"x".repeat(MAX_IDENTITY + 1),
            Some(EXE),
            IdentityKind::Desktop,
            None,
        );
        assert!(!long.bounded());
        assert!(desktop().bounded());
        assert!(missing().bounded());
        assert!(!identity("Opaque.Editor", None, IdentityKind::Desktop, None).bounded());
        assert!(!identity(
            "Opaque.Editor",
            Some(EXE),
            IdentityKind::Desktop,
            Some(IdentityReason::MissingTarget)
        )
        .bounded());
        assert!(!identity("Opaque.Editor", None, IdentityKind::Unavailable, None).bounded());
    }

    #[test]
    fn rejected_requests_never_reach_the_reader_or_retain_a_slot() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(catalog);
        for edge in [0, MAX_EDGE + 1, u32::MAX] {
            assert_eq!(
                harness.start(generation, &desktop(), edge).unwrap_err().code,
                "icons_edge"
            );
        }
        for candidate in [
            identity("Opaque.Unknown", Some(EXE), IdentityKind::Desktop, None),
            identity(
                "Opaque.Editor",
                Some(r"C:\owned icons\other.exe"),
                IdentityKind::Desktop,
                None,
            ),
            identity("Opaque.Editor", Some(EXE), IdentityKind::Packaged, None),
            identity("Opaque.Editor", None, IdentityKind::Desktop, None),
        ] {
            assert_eq!(
                harness.start(generation, &candidate, 32).unwrap_err().code,
                "icons_identity",
                "{candidate:?}"
            );
        }
        let long = identity(
            &"x".repeat(MAX_IDENTITY + 1),
            Some(EXE),
            IdentityKind::Desktop,
            None,
        );
        assert_eq!(
            harness.start(generation, &long, 32).unwrap_err().code,
            "icons_identity"
        );
        // A catalog-marked unavailable row is refused before the reader as well.
        assert_eq!(
            harness.start(generation, &missing(), 32).unwrap_err().code,
            "icons_unavailable"
        );
        // Unknown, zero and superseded catalog generations are explicit, never guessed.
        assert_eq!(
            harness
                .start(generation + 1, &desktop(), 32)
                .unwrap_err()
                .code,
            "icons_catalog"
        );
        assert_eq!(
            harness.start(0, &desktop(), 32).unwrap_err().code,
            "icons_catalog"
        );
        harness.controlled.silent();
        assert!(harness.icons.state.lock().unwrap().jobs.is_empty());
        assert_eq!(
            harness.icons.poll(1).unwrap_err().code,
            "icons_job",
            "rejected requests created a job"
        );
    }

    #[test]
    fn catalog_generation_must_be_current_ready_and_supersession_rejects_results() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(Arc::clone(&catalog));
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]));
        let id = job_id(&harness.start(generation, &desktop(), 32).unwrap());
        harness.controlled.expect(&[("Opaque.Editor", 32)]);
        let ready = settled(&harness.icons, id).unwrap();
        assert_eq!(ready["state"], "ready");
        assert_eq!(ready["identity"]["source_id"], "Opaque.Editor");
        assert_eq!(ready["width"], 2);
        assert_eq!(ready["height"], 1);
        assert_eq!(ready["edge"], 32);
        // An accepted start replaces the generation even when it only reads the cache.
        let next = fixtures::ready(
            &catalog,
            vec![fixtures::app(
                "Editor",
                "Opaque.Editor",
                Some(EXE),
                "desktop",
                None,
            )],
            vec![],
        );
        assert!(next > generation);
        assert_eq!(harness.icons.poll(id).unwrap_err().code, "icons_catalog");
        assert_eq!(
            harness
                .start(generation, &desktop(), 32)
                .unwrap_err()
                .code,
            "icons_catalog"
        );
        // The new generation is usable, including its own recommendation-free membership.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![0, 0, 0, 255]));
        let fresh = job_id(&harness.start(next, &desktop(), 16).unwrap());
        harness
            .controlled
            .expect(&[("Opaque.Editor", 32), ("Opaque.Editor", 16)]);
        assert_eq!(settled(&harness.icons, fresh).unwrap()["width"], 1);
    }

    #[test]
    fn non_ready_or_cancelled_catalog_generation_rejects_requests() {
        let catalog = Arc::new(CatalogService::default());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut unblock = Unblock::new(release_tx);
        let snapshot = fixtures::snapshot(
            vec![fixtures::app(
                "Editor",
                "Opaque.Editor",
                Some(EXE),
                "desktop",
                None,
            )],
            vec![],
        );
        let generation = job_id(
            &catalog
                .start_with(true, move |_| {
                    let _ = entered_tx.send(());
                    release_rx.recv().unwrap();
                    Ok(snapshot)
                })
                .unwrap(),
        );
        entered_rx.recv_timeout(WAIT).unwrap();
        let harness = Harness::new(Arc::clone(&catalog));
        // A running scan is not an accepted READY generation.
        assert_eq!(
            harness
                .start(generation, &desktop(), 32)
                .unwrap_err()
                .code,
            "icons_catalog"
        );
        harness.controlled.silent();
        unblock.fire();
        let start = Instant::now();
        while catalog
            .poll(generation)
            .map(|value| value["state"] != "ready")
            .unwrap_or(true)
        {
            assert!(
                start.elapsed() < WAIT,
                "catalog did not publish"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        // The now-ready generation is accepted.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![7, 7, 7, 255]));
        let id = job_id(&harness.start(generation, &desktop(), 32).unwrap());
        assert_eq!(settled(&harness.icons, id).unwrap()["state"], "ready");
        // Cancelling the catalog job invalidates every icon request bound to it.
        catalog.cancel(generation).unwrap();
        assert_eq!(
            harness
                .start(generation, &desktop(), 32)
                .unwrap_err()
                .code,
            "icons_catalog"
        );
    }

    #[test]
    fn cancellation_before_and_after_completion_never_delivers_a_raster() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(catalog);
        let icons = Arc::clone(&harness.icons);
        let id = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        harness.controlled.expect(&[("Opaque.Editor", 32)]);
        icons.cancel(id).unwrap();
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_cancelled");
        // A delayed completion that ignored cancellation is still rejected.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![9, 9, 9, 255]));
        idle(&icons, id);
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_cancelled");
        // Cancellation after the worker published, but before adoption, also prevents the raster.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![1, 2, 3, 255]));
        let second = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        harness
            .controlled
            .expect(&[("Opaque.Editor", 32), ("Opaque.Editor", 32)]);
        idle(&icons, second);
        icons.cancel(second).unwrap();
        assert_eq!(icons.poll(second).unwrap_err().code, "icons_cancelled");
        // The slot is reusable afterwards.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![4, 5, 6, 255]));
        let third = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        assert_eq!(settled(&icons, third).unwrap()["state"], "ready");
    }

    #[test]
    fn two_bounded_slots_report_busy_and_reap_released_work() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(catalog);
        let icons = Arc::clone(&harness.icons);
        let first = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        let second = job_id(&icons.start(generation, &packaged(), 16).unwrap());
        assert!(second > first);
        assert_eq!(
            icons.start(generation, &desktop(), 32).unwrap_err().code,
            "icons_busy"
        );
        assert_eq!(icons.state.lock().unwrap().jobs.len(), MAX_JOBS);
        harness
            .controlled
            .expect(&[("Opaque.Editor", 32), (AUMID, 16)]);
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![255, 0, 0, 255]));
        assert_eq!(settled(&icons, first).unwrap()["state"], "ready");
        harness
            .controlled
            .reply(AUMID, Reply::Raster(2, 2, vec![0; 16]));
        assert_eq!(settled(&icons, second).unwrap()["width"], 2);
        // Both delivered slots are reaped, so capacity returns without leaking workers.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![0, 255, 0, 255]));
        let third = job_id(&icons.start(generation, &desktop(), 8).unwrap());
        assert_eq!(icons.state.lock().unwrap().jobs.len(), 1);
        assert_eq!(settled(&icons, third).unwrap()["state"], "ready");
        assert_eq!(icons.poll(third).unwrap()["edge"], 8);
    }

    #[test]
    fn busy_or_cancelled_slots_do_not_wrap_job_ids() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(catalog);
        let icons = Arc::clone(&harness.icons);
        let id = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        harness.controlled.expect(&[("Opaque.Editor", 32)]);
        icons.cancel(id).unwrap();
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_cancelled");
        // The cancelled slot still owns its blocked worker, so the icon identity space is what fails.
        icons.exhaust_ids();
        assert_eq!(
            icons.start(generation, &desktop(), 32).unwrap_err().code,
            "icons_overflow"
        );
        assert_eq!(icons.state.lock().unwrap().jobs.len(), 1);
        // Releasing the fixture lets the cancelled worker terminate and its slot be reaped.
        harness.release();
        idle(&icons, id);
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_cancelled");
        icons.close();
        icons.close();
        assert!(icons.state.lock().unwrap().jobs.is_empty());
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_closed");
    }

    #[test]
    fn worker_panic_and_invalid_rasters_are_contained_and_recoverable() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(catalog);
        let icons = Arc::clone(&harness.icons);
        let failures = vec![
            (Reply::Panic, "icons_worker"),
            // A backend raster larger than the requested edge is a contract violation.
            (Reply::Raster(64, 64, vec![0; 64 * 64 * 4]), "icons_bounds"),
            (Reply::Raster(129, 1, vec![0; 516]), "icons_bounds"),
            (Reply::Raster(1, 1, vec![0; 3]), "icons_bounds"),
            (Reply::Raster(0, 0, vec![]), "icons_bounds"),
            (Reply::Fail(ShellError::OutOfBounds), "icons_bounds"),
            (Reply::Fail(ShellError::InvalidIdentity), "icons_identity"),
            (Reply::Fail(ShellError::Cancelled), "icons_cancelled"),
            (
                Reply::Fail(ShellError::Windows {
                    phase: "icons_image",
                    code: -2147467259,
                }),
                "icons_windows",
            ),
        ];
        let mut calls = 0usize;
        for (reply, code) in failures {
            let id = job_id(&icons.start(generation, &desktop(), 32).unwrap());
            calls += 1;
            let expected = vec![("Opaque.Editor", 32); calls];
            harness.controlled.expect(&expected);
            harness.controlled.reply("Opaque.Editor", reply);
            assert_eq!(settled(&icons, id).unwrap_err().code, code);
        }
        // The requested edge is enforced at native acceptance, and the slot stays reusable.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(32, 32, vec![0; 32 * 32 * 4]));
        let exact = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        assert_eq!(settled(&icons, exact).unwrap()["width"], 32);
        // A fully transparent raster is a valid bounded result, not a fabricated placeholder.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(64, 64, vec![0; 64 * 64 * 4]));
        let full = job_id(&icons.start(generation, &desktop(), 64).unwrap());
        let value = settled(&icons, full).unwrap();
        assert_eq!(value["width"], 64);
        assert_eq!(value["height"], 64);
        assert_eq!(value["pixels"].as_array().unwrap().len(), 64 * 64 * 4);
    }

    #[test]
    fn close_rejects_new_work_cancels_and_joins_blocked_workers() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(catalog);
        let icons = Arc::clone(&harness.icons);
        let id = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        harness.controlled.expect(&[("Opaque.Editor", 32)]);
        let closing = {
            let icons = Arc::clone(&icons);
            std::thread::spawn(move || icons.close())
        };
        await_closed(&icons);
        // New work and status reads are refused while the close waits for the blocked worker.
        assert_eq!(
            icons.start(generation, &desktop(), 32).unwrap_err().code,
            "icons_closed"
        );
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_closed");
        assert_eq!(icons.cancel(id).unwrap_err().code, "icons_closed");
        assert!(
            !closing.is_finished(),
            "close returned without joining the blocked worker"
        );
        // The state lock is free while close waits: readers see a closed, already-owned service.
        assert!(icons.state.lock().unwrap().closed);
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(1, 1, vec![1, 2, 3, 255]));
        closing.join().unwrap();
        assert!(icons.state.lock().unwrap().jobs.is_empty());
        icons.close();
    }

    #[test]
    fn close_is_idempotent_and_race_free_with_start() {
        for _ in 0..4 {
            let (catalog, generation) = catalog();
            let harness = Harness::new(catalog);
            let icons = Arc::clone(&harness.icons);
            let starter = {
                let icons = Arc::clone(&icons);
                std::thread::spawn(move || icons.start(generation, &desktop(), 32))
            };
            let closer = {
                let icons = Arc::clone(&icons);
                std::thread::spawn(move || icons.close())
            };
            let started = starter.join().unwrap();
            match started {
                Ok(value) => {
                    // The accepted request is owned and cancelled by the concurrent close.
                    let id = job_id(&value);
                    harness.controlled.expect(&[("Opaque.Editor", 32)]);
                    let _ = icons.poll(id);
                }
                // Losing the race is a documented rejection, never a leaked worker.
                Err(failure) => assert_eq!(failure.code, "icons_closed"),
            }
            // A close blocked on a fixture reader is unblocked like any other blocked worker.
            harness.release();
            closer.join().unwrap();
            icons.close();
            icons.close();
            assert!(icons.state.lock().unwrap().closed);
            assert!(icons.state.lock().unwrap().jobs.is_empty());
            assert_eq!(icons.poll(1).unwrap_err().code, "icons_closed");
        }
    }

    #[test]
    fn superseded_generations_invalidate_results_and_reclaim_finished_slots() {
        let (catalog, generation) = catalog();
        let harness = Harness::new(Arc::clone(&catalog));
        let icons = Arc::clone(&harness.icons);
        // Two finished, unclaimed results bound to the old generation fill both bounded slots.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(8, 8, vec![1; 8 * 8 * 4]));
        harness
            .controlled
            .reply(AUMID, Reply::Raster(4, 4, vec![2; 4 * 4 * 4]));
        let first = job_id(&icons.start(generation, &desktop(), 32).unwrap());
        let second = job_id(&icons.start(generation, &packaged(), 16).unwrap());
        harness
            .controlled
            .expect(&[("Opaque.Editor", 32), (AUMID, 16)]);
        idle(&icons, first);
        idle(&icons, second);
        assert_eq!(icons.state.lock().unwrap().jobs.len(), MAX_JOBS);
        // Nothing was adopted yet, and an unclaimed result still occupies its slot.
        assert_eq!(
            icons.start(generation, &desktop(), 32).unwrap_err().code,
            "icons_busy"
        );
        // A cached accepted start installs a new generation even though it runs no worker.
        assert_eq!(catalog.poll(generation).unwrap()["state"], "ready");
        let cached = catalog
            .start_with(false, |_| panic!("cached read rescanned"))
            .unwrap();
        let next = job_id(&cached);
        assert!(next > generation);
        assert_eq!(cached["cached"], true);
        // Both finished results are now unusable, so their slots are reclaimed instead of blocking.
        harness
            .controlled
            .reply("Opaque.Editor", Reply::Raster(2, 2, vec![3; 2 * 2 * 4]));
        let fresh = job_id(&icons.start(next, &desktop(), 32).unwrap());
        assert_eq!(settled(&icons, fresh).unwrap()["width"], 2);
        assert_eq!(icons.poll(first).unwrap_err().code, "icons_job");
        assert_eq!(icons.poll(second).unwrap_err().code, "icons_job");
        // A failed refresh publishes no generation: requests for it are rejected, not guessed.
        let failed = job_id(&catalog.start_with(true, |_| Err("apps_folder")).unwrap());
        let start = Instant::now();
        loop {
            match catalog.poll(failed) {
                Ok(_) => {}
                Err(failure) => {
                    assert_eq!(failure.code, "apps_folder");
                    break;
                }
            }
            assert!(
                start.elapsed() < WAIT,
                "catalog refresh did not fail"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            icons.start(failed, &desktop(), 32).unwrap_err().code,
            "icons_catalog"
        );
        assert_eq!(
            icons.start(next, &desktop(), 32).unwrap_err().code,
            "icons_catalog"
        );
    }

    #[test]
    fn maximum_raster_with_bounded_identity_stays_inside_the_declared_response() {
        // Worst-case JSON escaping for the bounded identity: quotes double when serialized.
        let quoted = "\"".repeat(MAX_IDENTITY);
        let service = Arc::new(CatalogService::default());
        let generation = fixtures::ready(
            &service,
            vec![fixtures::app(
                "Quoted",
                &quoted,
                Some(quoted.as_str()),
                "desktop",
                None,
            )],
            vec![],
        );
        let harness = Harness::new(service);
        harness.controlled.reply(
            &quoted,
            Reply::Raster(MAX_EDGE, MAX_EDGE, vec![255; MAX_PIXELS]),
        );
        let requested = identity(&quoted, Some(quoted.as_str()), IdentityKind::Desktop, None);
        let id = job_id(&harness.start(generation, &requested, MAX_EDGE).unwrap());
        let value = settled(&harness.icons, id).unwrap();
        assert_eq!(value["width"], MAX_EDGE);
        assert_eq!(value["height"], MAX_EDGE);
        assert_eq!(value["pixels"].as_array().unwrap().len(), MAX_PIXELS);
        let wire = serde_json::to_vec(&value).unwrap().len();
        assert!(
            wire <= MAX_RESPONSE_BYTES,
            "response {wire} exceeds the declared {MAX_RESPONSE_BYTES}"
        );
        assert!(
            wire > MAX_PIXEL_WIRE,
            "the maximum raster must dominate the declared response"
        );
    }

    struct RuntimeFixture {
        handle: u64,
        directory: PathBuf,
    }

    impl RuntimeFixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("TieZ-icons-abi-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let opened =
                command(json!({"op":"open","database_path":directory.join("settings.db")}))
                    .unwrap();
            Self {
                handle: opened["handle"].as_u64().unwrap(),
                directory,
            }
        }

        fn catalog(&self) -> Arc<CatalogService> {
            Arc::clone(&crate::REGISTRY.get().unwrap().lock().unwrap().services[&self.handle].apps)
        }

        fn icons(&self) -> Arc<IconService> {
            Arc::clone(
                &crate::REGISTRY.get().unwrap().lock().unwrap().services[&self.handle]
                    .icons,
            )
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
                    .starts_with("TieZ-icons-abi-")
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

    fn command(value: Value) -> CoreResult<Value> {
        crate::dispatch(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn abi_strict_schema_membership_and_readonly_state() {
        let fixture = RuntimeFixture::new();
        let handle = fixture.handle;
        let catalog = fixture.catalog();
        let generation = fixtures::ready(
            &catalog,
            vec![
                fixtures::app(
                    "Owned",
                    "Opaque.Owned",
                    Some(r"C:\owned icons\missing editor.exe"),
                    "desktop",
                    None,
                ),
                fixtures::app(
                    "Missing",
                    "Opaque.Missing",
                    None,
                    "unavailable",
                    Some("not_native_executable"),
                ),
            ],
            vec![fixtures::app(
                "Recommended",
                "Opaque.Recommended",
                Some(RECOMMENDED),
                "desktop",
                None,
            )],
        );
        let owned = json!({"source_id":"Opaque.Owned","handler":r"C:\owned icons\missing editor.exe","kind":"desktop","unavailable":null});
        assert_eq!(
            command(json!({"op":"apps_icon_poll","handle":0,"job_id":1}))
                .unwrap_err()
                .code,
            "invalid_handle"
        );
        // Unknown request fields, unknown identity fields and unknown enum values are rejected by
        // the schema, before any catalog or Windows work.
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":32,"pixels":[]}))
                .unwrap_err()
                .code,
            "invalid_request"
        );
        let mut extra = owned.clone();
        extra["extra"] = json!(1);
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":extra,"edge":32}))
                .unwrap_err()
                .code,
            "invalid_request"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":{"source_id":"Opaque.Owned","handler":r"C:\owned icons\missing editor.exe","kind":"shortcut"},"edge":32}))
                .unwrap_err()
                .code,
            "invalid_request"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":0}))
                .unwrap_err()
                .code,
            "icons_edge"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":{"source_id":"Opaque.Unknown","handler":r"C:\owned icons\missing editor.exe","kind":"desktop"},"edge":32}))
                .unwrap_err()
                .code,
            "icons_identity"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":{"source_id":"Opaque.Missing","handler":null,"kind":"unavailable","unavailable":"not_native_executable"},"edge":32}))
                .unwrap_err()
                .code,
            "icons_unavailable"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation + 7,"identity":owned,"edge":32}))
                .unwrap_err()
                .code,
            "icons_catalog"
        );
        assert!(fixture.icons().state.lock().unwrap().jobs.is_empty());
        // An accepted identity reaches the real reader, which refuses the non-executable handler
        // without any Shell/COM phase.
        let started = command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":32}))
            .unwrap();
        let id = job_id(&started);
        assert_eq!(started["state"], "running");
        assert_eq!(started["catalog_job_id"].as_u64().unwrap(), generation);
        assert_eq!(started["edge"], 32);
        assert_eq!(
            settled(&fixture.icons(), id).unwrap_err().code,
            "icons_identity"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_poll","handle":handle,"job_id":id + 1}))
                .unwrap_err()
                .code,
            "icons_job"
        );
        assert_eq!(
            command(json!({"op":"apps_icon_cancel","handle":handle,"job_id":id + 1}))
                .unwrap_err()
                .code,
            "icons_job"
        );
        // Requests never touch settings, capture or history.
        assert_eq!(
            command(json!({"op":"snapshot","handle":handle})).unwrap()["snapshot"]["revision"],
            0
        );
        assert_eq!(
            command(json!({"op":"capture_status","handle":handle})).unwrap()["starts"],
            0
        );
        command(json!({"op":"close","handle":handle})).unwrap();
        assert_eq!(
            command(json!({"op":"apps_icon_poll","handle":handle,"job_id":id}))
                .unwrap_err()
                .code,
            "invalid_handle"
        );
        let reopened =
            command(json!({"op":"open","database_path":fixture.directory.join("settings.db")}))
                .unwrap();
        let reopened = reopened["handle"].as_u64().unwrap();
        assert!(crate::REGISTRY.get().unwrap().lock().unwrap().services[&reopened]
            .icons
            .state
            .lock()
            .unwrap()
            .jobs
            .is_empty());
        command(json!({"op":"close","handle":reopened})).unwrap();
    }

    #[test]
    fn abi_close_joins_two_blocked_workers_and_keeps_registry_and_services_free() {
        let fixture = RuntimeFixture::new();
        let handle = fixture.handle;
        let icons = fixture.icons();
        let catalog = fixture.catalog();
        let controlled = Controlled::new();
        icons.replace_reader(Arc::clone(&controlled.reader));
        let generation = fixtures::ready(
            &catalog,
            vec![fixtures::app(
                "Owned",
                "Opaque.Owned",
                Some(r"C:\owned icons\missing editor.exe"),
                "desktop",
                None,
            )],
            vec![],
        );
        let owned = json!({"source_id":"Opaque.Owned","handler":r"C:\owned icons\missing editor.exe","kind":"desktop","unavailable":null});
        let id = job_id(
            &command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":64})).unwrap(),
        );
        controlled.expect(&[("Opaque.Owned", 64)]);
        // The blocked worker holds no registry, catalog, history or settings lock.
        assert_eq!(
            command(json!({"op":"apps_icon_poll","handle":handle,"job_id":id})).unwrap()["state"],
            "running"
        );
        assert_eq!(
            command(json!({"op":"snapshot","handle":handle})).unwrap()["snapshot"]["revision"],
            0
        );
        assert_eq!(
            command(json!({"op":"history_page","handle":handle,"query":{"limit":1}})).unwrap()
                ["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        // Both bounded slots may be owned, and only a third request is refused instead of queued.
        let second = job_id(
            &command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":64})).unwrap(),
        );
        assert!(second > id);
        controlled.expect(&[("Opaque.Owned", 64), ("Opaque.Owned", 64)]);
        assert_eq!(
            command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":64}))
                .unwrap_err()
                .code,
            "icons_busy"
        );
        let close = std::thread::spawn(move || command(json!({"op":"close","handle":handle})));
        await_closed(&icons);
        assert_eq!(
            command(json!({"op":"apps_icon_poll","handle":handle,"job_id":id}))
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
        assert!(!close.is_finished(), "close skipped a blocked icon worker");
        // The close cancels both jobs; releasing each blocked worker lets both joins finish.
        controlled.reply("Opaque.Owned", Reply::Raster(1, 1, vec![1, 2, 3, 255]));
        controlled.reply("Opaque.Owned", Reply::Raster(1, 1, vec![1, 2, 3, 255]));
        assert_eq!(close.join().unwrap().unwrap()["closed"], true);
        assert!(icons.state.lock().unwrap().jobs.is_empty());
        assert_eq!(
            icons.poll(second).unwrap_err().code,
            "icons_closed"
        );
    }

    #[test]
    fn runtime_close_cancels_scan_and_icon_workers_before_joining_either() {
        let fixture = RuntimeFixture::new();
        let handle = fixture.handle;
        let icons = fixture.icons();
        let catalog = fixture.catalog();
        let generation = fixtures::ready(
            &catalog,
            vec![fixtures::app(
                "Owned",
                "Opaque.Owned",
                Some(r"C:\owned icons\missing editor.exe"),
                "desktop",
                None,
            )],
            vec![],
        );
        // The icon worker observes production cancellation, then waits for the fixture release.
        let (icon_entered_tx, icon_entered_rx) = mpsc::channel::<()>();
        let (icon_cancel_tx, icon_cancel_rx) = mpsc::channel::<()>();
        let (icon_release_tx, icon_release_rx) = mpsc::channel::<()>();
        let icon_release_rx = Mutex::new(icon_release_rx);
        let mut icon_release = Unblock::new(icon_release_tx);
        let reader: Reader = Arc::new(
            move |_application: &Application, _edge: u32, cancel: &AtomicBool| {
                let _ = icon_entered_tx.send(());
                let start = Instant::now();
                while !cancel.load(Ordering::Acquire) {
                    assert!(
                        start.elapsed() < WAIT,
                        "icon worker never observed cancellation"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                let _ = icon_cancel_tx.send(());
                icon_release_rx
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .recv()
                    .unwrap();
                Ok(Raster {
                    width: 1,
                    height: 1,
                    rgba: vec![0, 0, 0, 255],
                })
            },
        );
        icons.replace_reader(reader);
        let owned = json!({"source_id":"Opaque.Owned","handler":r"C:\owned icons\missing editor.exe","kind":"desktop","unavailable":null});
        let id = job_id(
            &command(json!({"op":"apps_icon_start","handle":handle,"catalog_job_id":generation,"identity":owned,"edge":32})).unwrap(),
        );
        icon_entered_rx.recv_timeout(WAIT).unwrap();
        // A second, blocking scan replaces the catalog generation the icon job was bound to.
        let (scan_entered_tx, scan_entered_rx) = mpsc::channel::<()>();
        let (scan_cancel_tx, scan_cancel_rx) = mpsc::channel::<()>();
        let (scan_release_tx, scan_release_rx) = mpsc::channel::<()>();
        let mut scan_release = Unblock::new(scan_release_tx);
        let rows = fixtures::snapshot(
            vec![fixtures::app(
                "Owned",
                "Opaque.Owned",
                Some(r"C:\owned icons\missing editor.exe"),
                "desktop",
                None,
            )],
            vec![],
        );
        let scan = catalog
            .start_with(true, move |cancel| {
                let _ = scan_entered_tx.send(());
                let start = Instant::now();
                while !cancel.load(Ordering::Acquire) {
                    assert!(
                        start.elapsed() < WAIT,
                        "catalog worker never observed cancellation"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                let _ = scan_cancel_tx.send(());
                scan_release_rx.recv().unwrap();
                Ok(rows)
            })
            .unwrap();
        assert!(job_id(&scan) > generation);
        scan_entered_rx.recv_timeout(WAIT).unwrap();
        let close = std::thread::spawn(move || command(json!({"op":"close","handle":handle})));
        // Both workers observe cancellation before either one is joined: the close signals every
        // owned service first, so a blocked scan can never delay the icon workers' cancellation.
        icon_cancel_rx.recv_timeout(WAIT).unwrap();
        scan_cancel_rx.recv_timeout(WAIT).unwrap();
        assert!(
            !close.is_finished(),
            "close returned without joining the blocked workers"
        );
        // The registry is free while both joins are pending.
        assert_eq!(
            command(json!({"op":"snapshot","handle":handle}))
                .unwrap_err()
                .code,
            "core_closing"
        );
        icon_release.fire();
        scan_release.fire();
        assert_eq!(close.join().unwrap().unwrap()["closed"], true);
        assert!(icons.state.lock().unwrap().jobs.is_empty());
        assert!(catalog.accepted(generation).is_none());
        assert_eq!(icons.poll(id).unwrap_err().code, "icons_closed");
    }
}
