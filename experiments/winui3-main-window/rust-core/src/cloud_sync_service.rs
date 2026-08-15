//! WinUI-owned lifecycle for the shared WebDAV synchronization runner.
//!
//! The service owns one worker thread and one Tokio runtime. C++ only starts,
//! requests, stops, and polls sanitized status; credentials never cross the
//! native ABI.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiez_core::cloud_sync_runner::{run_webdav_once, CloudSyncHostEvent, CloudSyncRunStatus};
use tiez_core::cloud_sync_settings::CloudSyncSettings;
use tiez_core::cloud_sync_sqlite::{ensure_cloud_sync_device_id, SqliteCloudSyncHost};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeCloudSyncStatus {
    pub state: String,
    pub service_started: bool,
    pub syncing: bool,
    pub automatic: bool,
    pub last_sync_at: Option<i64>,
    pub next_sync_at: Option<i64>,
    pub last_error: Option<String>,
    pub uploaded_items: usize,
    pub received_items: usize,
    pub settings_revision: u64,
}

impl NativeCloudSyncStatus {
    fn unavailable() -> Self {
        Self {
            state: "unavailable".to_owned(),
            service_started: false,
            syncing: false,
            automatic: false,
            last_sync_at: None,
            next_sync_at: None,
            last_error: None,
            uploaded_items: 0,
            received_items: 0,
            settings_revision: 0,
        }
    }
}

#[derive(Default)]
struct ServiceControl {
    stop: bool,
    wake: bool,
    manual: bool,
    force_snapshot: bool,
}

#[derive(Clone, Copy)]
struct RunTrigger {
    manual: bool,
    force_snapshot: bool,
}

pub struct NativeCloudSyncService {
    database_path: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    read_only: bool,
    cancelled: Arc<AtomicBool>,
    control: Arc<(Mutex<ServiceControl>, Condvar)>,
    status: Arc<Mutex<NativeCloudSyncStatus>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    on_history_changed: Arc<dyn Fn() + Send + Sync>,
}

impl NativeCloudSyncService {
    pub fn unavailable() -> Self {
        Self {
            database_path: None,
            data_dir: None,
            read_only: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            control: Arc::new((Mutex::new(ServiceControl::default()), Condvar::new())),
            status: Arc::new(Mutex::new(NativeCloudSyncStatus::unavailable())),
            worker: Mutex::new(None),
            on_history_changed: Arc::new(|| {}),
        }
    }

    pub fn new(
        database_path: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        read_only: bool,
        on_history_changed: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let mut service = Self::unavailable();
        service.database_path = Some(database_path.into());
        service.data_dir = Some(data_dir.into());
        service.read_only = read_only;
        service.on_history_changed = Arc::new(on_history_changed);
        if let Ok(mut status) = service.status.lock() {
            status.state = if read_only { "read_only" } else { "stopped" }.to_owned();
        }
        service
    }

    pub fn status(&self) -> NativeCloudSyncStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    pub fn start(&self) -> Result<(), String> {
        self.start_with_trigger(false, false)
    }

    pub fn request_now(&self) -> Result<(), String> {
        self.start_with_trigger(true, true)
    }

    fn start_with_trigger(&self, manual: bool, force_snapshot: bool) -> Result<(), String> {
        let database_path = self
            .database_path
            .clone()
            .ok_or_else(|| "云同步仅在 WinUI 生产数据模式下可用".to_owned())?;
        let data_dir = self
            .data_dir
            .clone()
            .ok_or_else(|| "云同步数据目录不可用".to_owned())?;
        if self.read_only {
            return Err("当前数据库为只读，不能启动云同步".to_owned());
        }

        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "cloud-sync worker lock is poisoned".to_owned())?;
        if worker.as_ref().is_some_and(|worker| worker.is_finished()) {
            if let Some(finished) = worker.take() {
                let _ = finished.join();
            }
        }
        if worker.is_some() {
            let (control, signal) = &*self.control;
            let mut control = control
                .lock()
                .map_err(|_| "cloud-sync control lock is poisoned".to_owned())?;
            control.wake = true;
            control.manual |= manual;
            control.force_snapshot |= force_snapshot;
            signal.notify_all();
            return Ok(());
        }

        self.cancelled.store(false, Ordering::Release);
        {
            let (control, _) = &*self.control;
            let mut control = control
                .lock()
                .map_err(|_| "cloud-sync control lock is poisoned".to_owned())?;
            *control = ServiceControl {
                wake: true,
                manual,
                force_snapshot,
                ..ServiceControl::default()
            };
        }
        update_status(&self.status, |status| {
            status.state = "starting".to_owned();
            status.service_started = true;
            status.syncing = false;
            status.next_sync_at = None;
            status.last_error = None;
        });

        let cancelled = Arc::clone(&self.cancelled);
        let control = Arc::clone(&self.control);
        let status = Arc::clone(&self.status);
        let on_history_changed = Arc::clone(&self.on_history_changed);
        let thread_status = Arc::clone(&status);
        let spawn_result = thread::Builder::new()
            .name("tiez-winui-cloud-sync".to_owned())
            .spawn(move || {
                worker_loop(
                    database_path,
                    data_dir,
                    cancelled,
                    control,
                    thread_status,
                    on_history_changed,
                );
            });
        match spawn_result {
            Ok(handle) => {
                *worker = Some(handle);
                Ok(())
            }
            Err(error) => {
                update_status(&status, |status| {
                    status.state = "error".to_owned();
                    status.service_started = false;
                    status.last_error = Some(format!("无法创建云同步线程：{error}"));
                });
                Err(format!("无法创建云同步线程：{error}"))
            }
        }
    }

    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        let (control, signal) = &*self.control;
        if let Ok(mut control) = control.lock() {
            control.stop = true;
            control.wake = true;
            signal.notify_all();
        }
        let handle = self.worker.lock().ok().and_then(|mut worker| worker.take());
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        update_status(&self.status, |status| {
            status.state = "stopped".to_owned();
            status.service_started = false;
            status.syncing = false;
            status.next_sync_at = None;
        });
    }
}

impl Drop for NativeCloudSyncService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(
    database_path: PathBuf,
    data_dir: PathBuf,
    cancelled: Arc<AtomicBool>,
    control: Arc<(Mutex<ServiceControl>, Condvar)>,
    status: Arc<Mutex<NativeCloudSyncStatus>>,
    on_history_changed: Arc<dyn Fn() + Send + Sync>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            update_status(&status, |status| {
                status.state = "error".to_owned();
                status.service_started = false;
                status.last_error = Some(format!("无法启动云同步运行时：{error}"));
            });
            return;
        }
    };

    let mut next_run = None;
    loop {
        let Some(trigger) = wait_for_trigger(&control, next_run) else {
            break;
        };
        if cancelled.load(Ordering::Acquire) {
            break;
        }

        let next_delay = run_sync_pass(
            &runtime,
            &database_path,
            &data_dir,
            Arc::clone(&cancelled),
            Arc::clone(&status),
            Arc::clone(&on_history_changed),
            trigger,
        );
        next_run = next_delay.map(|delay| Instant::now() + delay);
        let next_sync_at = next_delay
            .map(|delay| now_ms().saturating_add(delay.as_millis().min(i64::MAX as u128) as i64));
        update_status(&status, |status| {
            status.next_sync_at = next_sync_at;
        });
    }

    update_status(&status, |status| {
        status.service_started = false;
        status.syncing = false;
        status.next_sync_at = None;
    });
}

fn wait_for_trigger(
    shared: &Arc<(Mutex<ServiceControl>, Condvar)>,
    next_run: Option<Instant>,
) -> Option<RunTrigger> {
    let (control, signal) = &**shared;
    let mut control = control
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if control.stop {
            return None;
        }
        if control.wake {
            let trigger = RunTrigger {
                manual: control.manual,
                force_snapshot: control.force_snapshot,
            };
            control.wake = false;
            control.manual = false;
            control.force_snapshot = false;
            return Some(trigger);
        }
        match next_run {
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    return Some(RunTrigger {
                        manual: false,
                        force_snapshot: false,
                    });
                }
                let waited = signal
                    .wait_timeout(control, deadline.saturating_duration_since(now))
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                control = waited.0;
                if waited.1.timed_out() {
                    return Some(RunTrigger {
                        manual: false,
                        force_snapshot: false,
                    });
                }
            }
            None => {
                control = signal
                    .wait(control)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
    }
}

fn run_sync_pass(
    runtime: &tokio::runtime::Runtime,
    database_path: &PathBuf,
    data_dir: &PathBuf,
    cancelled: Arc<AtomicBool>,
    status: Arc<Mutex<NativeCloudSyncStatus>>,
    on_history_changed: Arc<dyn Fn() + Send + Sync>,
    trigger: RunTrigger,
) -> Option<Duration> {
    let settings = match CloudSyncSettings::open_sqlite(database_path, false) {
        Ok(settings) => settings,
        Err(error) => {
            set_failure(&status, format!("无法读取云同步设置：{error}"), false);
            return Some(Duration::from_secs(120));
        }
    };
    let snapshot = match settings.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            set_failure(&status, format!("无法读取云同步设置：{error}"), false);
            return Some(Duration::from_secs(120));
        }
    };
    if !snapshot.enabled {
        update_status(&status, |status| {
            status.state = "disabled".to_owned();
            status.syncing = false;
            status.automatic = false;
            status.next_sync_at = None;
            status.last_error = None;
        });
        return None;
    }
    if !trigger.manual && !snapshot.auto_sync {
        update_status(&status, |status| {
            status.state = "waiting".to_owned();
            status.syncing = false;
            status.automatic = false;
            status.next_sync_at = None;
            status.last_error = None;
        });
        return None;
    }

    let automatic = snapshot.auto_sync;
    let interval = Duration::from_secs(snapshot.interval_secs);
    let device_id = match ensure_cloud_sync_device_id(database_path) {
        Ok(device_id) => device_id,
        Err(error) => {
            set_failure(
                &status,
                format!("无法创建云同步设备标识：{error}"),
                automatic,
            );
            return automatic.then_some(interval);
        }
    };
    let config = match settings.runner_config(device_id) {
        Ok(Some(config)) => config,
        Ok(None) => return None,
        Err(error) => {
            set_failure(&status, format!("云同步设置无效：{error}"), automatic);
            return automatic.then_some(interval);
        }
    };

    update_status(&status, |status| {
        status.state = "syncing".to_owned();
        status.syncing = true;
        status.automatic = automatic;
        status.next_sync_at = None;
        status.last_error = None;
    });
    let event_status = Arc::clone(&status);
    let event_history = Arc::clone(&on_history_changed);
    let mut host = match SqliteCloudSyncHost::new(database_path, data_dir, cancelled).map(|host| {
        host.with_event_sink(move |event| match event {
            CloudSyncHostEvent::Status(run_status) => {
                apply_runner_status(&event_status, &run_status, automatic);
            }
            CloudSyncHostEvent::HistoryChanged => event_history(),
            CloudSyncHostEvent::SettingsChanged => {
                update_status(&event_status, |status| {
                    status.settings_revision = status.settings_revision.saturating_add(1);
                });
            }
        })
    }) {
        Ok(host) => host,
        Err(error) => {
            set_failure(
                &status,
                format!("无法打开云同步数据库宿主：{error}"),
                automatic,
            );
            return automatic.then_some(interval);
        }
    };

    if let Err(error) =
        runtime.block_on(run_webdav_once(&mut host, &config, trigger.force_snapshot))
    {
        set_failure(&status, error.to_string(), automatic);
    }
    automatic.then_some(interval)
}

fn apply_runner_status(
    status: &Arc<Mutex<NativeCloudSyncStatus>>,
    run_status: &CloudSyncRunStatus,
    automatic: bool,
) {
    update_status(status, |status| {
        status.state = run_status.state.clone();
        status.syncing = run_status.state == "syncing";
        status.automatic = automatic;
        status.last_sync_at = run_status.last_sync_at.or(status.last_sync_at);
        status.last_error = run_status.last_error.clone();
        status.uploaded_items = run_status.uploaded_items;
        status.received_items = run_status.received_items;
    });
}

fn set_failure(status: &Arc<Mutex<NativeCloudSyncStatus>>, message: String, automatic: bool) {
    update_status(status, |status| {
        status.state = "error".to_owned();
        status.syncing = false;
        status.automatic = automatic;
        status.last_error = Some(message);
    });
}

fn update_status(
    status: &Arc<Mutex<NativeCloudSyncStatus>>,
    update: impl FnOnce(&mut NativeCloudSyncStatus),
) {
    match status.lock() {
        Ok(mut status) => update(&mut status),
        Err(poisoned) => update(&mut poisoned.into_inner()),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiez_core::database_bootstrap::open_database_with_decrypt;

    #[test]
    fn disabled_database_starts_without_network_and_stops_cleanly() {
        let root = std::env::temp_dir().join(format!(
            "tiez-winui-cloud-service-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database_path = root.join("clipboard.db");
        open_database_with_decrypt(&database_path, tiez_core::encryption::decrypt_value).unwrap();
        let service = NativeCloudSyncService::new(&database_path, &root, false, || {});

        service.start().unwrap();
        for _ in 0..100 {
            if service.status().state == "disabled" {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(service.status().state, "disabled");
        assert!(service.status().service_started);

        service.stop();
        assert_eq!(service.status().state, "stopped");
        assert!(!service.status().service_started);
        std::fs::remove_dir_all(root).unwrap();
    }
}
