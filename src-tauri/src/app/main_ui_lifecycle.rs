use crate::app::window_manager::{restore_last_focus, toggle_window};
use crate::app_state::SessionHistory;
use crate::database::DbState;
use crate::global_state::{
    CURRENT_DOCK, IS_HIDDEN, IS_MAIN_WINDOW_FOCUSED, LAST_SHOW_TIMESTAMP, NAVIGATION_ENABLED,
    NAVIGATION_MODE_ACTIVE,
};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const MAIN_LABEL: &str = "main";
const LIFECYCLE_ENV: &str = "TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE";
const TRACE_LIMIT: usize = 1024;
const OPERATION_OUTCOME_LIMIT: usize = 128;
const WAIT_STEP: Duration = Duration::from_millis(10);
const DESTROY_TIMEOUT: Duration = Duration::from_secs(10);
const OPERATION_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15);
const FRONTEND_READY_TIMEOUT: Duration = Duration::from_secs(15);
const TEST_DOWN_SETTLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleMode {
    Default,
    Hidden,
    Destroyed,
}

impl LifecycleMode {
    fn from_env() -> Self {
        Self::from_value(std::env::var(LIFECYCLE_ENV).ok().as_deref())
    }

    fn from_value(value: Option<&str>) -> Self {
        match value
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "hidden" => Self::Hidden,
            "destroyed" => Self::Destroyed,
            _ => Self::Default,
        }
    }

    pub fn is_experimental(self) -> bool {
        self != Self::Default
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Hidden => "hidden",
            Self::Destroyed => "destroyed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Hidden,
    Destroying,
    Destroyed,
    Recreating,
    AwaitingFrontend,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeIntent {
    Main,
    Search,
    Tray,
    Test,
}

impl WakeIntent {
    fn requires_search(self) -> bool {
        matches!(self, Self::Search | Self::Test)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiReadyPhase {
    ReactMounted,
    Hydrated,
    SearchReady,
    SearchResultsSettled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetVisibility {
    Hidden,
    Visible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideReason {
    Toggle,
    CloseRequested,
    Blur,
    FrontendCommand,
    PasteFocusRestore,
    AfterPaste,
    Test,
}

impl HideReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::CloseRequested => "close_requested",
            Self::Blur => "blur",
            Self::FrontendCommand => "frontend_command",
            Self::PasteFocusRestore => "paste_focus_restore",
            Self::AfterPaste => "after_paste",
            Self::Test => "test",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WakeSource {
    Toggle,
    Explicit,
    SearchShortcut,
    TrayMenu,
    TrayClick,
    Test,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleTrace {
    pub request_id: u64,
    pub generation: u64,
    pub mode: LifecycleMode,
    pub intent: WakeIntent,
    pub phase: String,
    pub timestamp_unix_ms: u64,
    pub elapsed_ms: u64,
    pub main_window_count: usize,
    pub clipboard_event_count: u64,
    pub persisted_history_count: Option<i64>,
    pub session_history_count: Option<usize>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleSnapshot {
    pub enabled: bool,
    pub mode: LifecycleMode,
    pub phase: LifecyclePhase,
    pub generation: u64,
    pub active_request_id: Option<u64>,
    pub completed_request_id: Option<u64>,
    pub failed_request_id: Option<u64>,
    pub active_intent: Option<WakeIntent>,
    pub react_mounted: bool,
    pub hydrated: bool,
    pub search_ready: bool,
    pub search_results_settled: bool,
    pub focused: bool,
    pub requested_visible_focused_hydrated_search_ready_ms: Option<u64>,
    pub main_window_count: usize,
    pub main_window_present: bool,
    pub main_window_visible: bool,
    pub main_window_focused: bool,
    pub clipboard_event_count: u64,
    pub persisted_history_count: Option<i64>,
    pub session_history_count: Option<usize>,
    pub history_probe_available: bool,
    pub explicit_exit_requested: bool,
    pub worker_running: bool,
    pub in_flight_target: Option<TargetVisibility>,
    pub pending_target: Option<TargetVisibility>,
    pub trace_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleClipboardProbe {
    pub token: String,
    pub clipboard_event_count: u64,
    pub clipboard_event_count_before: u64,
    pub clipboard_event_delta: u64,
    pub listener_event_count_increased: bool,
    pub persisted_entry_id: Option<i64>,
    pub session_entry_id: Option<i64>,
    pub exact_history_match: bool,
    pub exact_history_match_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainUiReadyReport {
    pub request_id: u64,
    pub generation: u64,
    pub phase: UiReadyPhase,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MainUiBootstrap {
    pub enabled: bool,
    pub mode: LifecycleMode,
    pub generation: u64,
    pub request_id: Option<u64>,
    pub intent: Option<WakeIntent>,
}

#[derive(Clone, Debug)]
struct ActiveWake {
    request_id: u64,
    generation: u64,
    intent: WakeIntent,
    requested_at: Instant,
    react_mounted: bool,
    hydrated: bool,
    search_ready: bool,
    search_results_settled: bool,
    focused: bool,
    requires_focus: bool,
    search_focus_sent: bool,
    usable_ready_recorded: bool,
}

impl ActiveWake {
    fn frontend_ready(&self) -> bool {
        self.hydrated && (!self.intent.requires_search() || self.search_ready)
    }

    fn usable_ready(&self) -> bool {
        self.frontend_ready() && (!self.requires_focus || self.focused)
    }
}

#[derive(Clone, Debug)]
struct LifecycleOperation {
    request_id: u64,
    intent: WakeIntent,
    requested_at: Instant,
    target: TargetVisibility,
    source: WakeSource,
    hide_reason: Option<HideReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OperationOutcome {
    Succeeded,
    Failed(String),
    Superseded,
}

fn completion_outcome(
    target: TargetVisibility,
    result: &Result<(), String>,
    pending_target: Option<TargetVisibility>,
) -> OperationOutcome {
    match result {
        Err(error) => OperationOutcome::Failed(error.clone()),
        Ok(()) if pending_target.is_some_and(|pending| pending != target) => {
            OperationOutcome::Superseded
        }
        Ok(()) => OperationOutcome::Succeeded,
    }
}

fn canonical_request_id(
    new_request_id: u64,
    target: TargetVisibility,
    in_flight: Option<(u64, TargetVisibility)>,
    pending: Option<(u64, TargetVisibility)>,
) -> u64 {
    if let Some((request_id, in_flight_target)) = in_flight {
        if in_flight_target == target {
            return request_id;
        }
    }
    if let Some((request_id, pending_target)) = pending {
        if pending_target == target {
            return request_id;
        }
    }
    new_request_id
}

#[derive(Clone, Debug)]
struct PendingTrace {
    request_id: u64,
    generation: u64,
    intent: WakeIntent,
    requested_at: Instant,
    requested_unix_ms: u64,
    phase: &'static str,
    detail: Option<String>,
}

#[derive(Debug)]
struct LifecycleInner {
    phase: LifecyclePhase,
    generation: u64,
    active_wake: Option<ActiveWake>,
    completed_request_id: Option<u64>,
    failed_request_id: Option<u64>,
    operation_outcomes: VecDeque<(u64, OperationOutcome)>,
    traces: VecDeque<LifecycleTrace>,
    pending_traces: VecDeque<PendingTrace>,
    pending_operation: Option<LifecycleOperation>,
    in_flight_target: Option<TargetVisibility>,
    in_flight_request_id: Option<u64>,
    worker_running: bool,
    last_usable_wake_ms: Option<u64>,
}

pub struct MainUiLifecycle {
    mode: LifecycleMode,
    inner: Mutex<LifecycleInner>,
    next_request_id: AtomicU64,
    clipboard_event_count: AtomicU64,
    explicit_exit_requested: AtomicBool,
}

impl MainUiLifecycle {
    pub fn from_environment() -> Self {
        Self::new(LifecycleMode::from_env())
    }

    fn new(mode: LifecycleMode) -> Self {
        Self {
            mode,
            inner: Mutex::new(LifecycleInner {
                phase: LifecyclePhase::Ready,
                generation: 1,
                active_wake: None,
                completed_request_id: None,
                failed_request_id: None,
                operation_outcomes: VecDeque::new(),
                traces: VecDeque::new(),
                pending_traces: VecDeque::new(),
                pending_operation: None,
                in_flight_target: None,
                in_flight_request_id: None,
                worker_running: false,
                last_usable_wake_ms: None,
            }),
            next_request_id: AtomicU64::new(1),
            clipboard_event_count: AtomicU64::new(0),
            explicit_exit_requested: AtomicBool::new(false),
        }
    }

    pub fn mode(&self) -> LifecycleMode {
        self.mode
    }

    pub fn enabled(&self) -> bool {
        self.mode.is_experimental()
    }

    fn lock_inner(&self) -> MutexGuard<'_, LifecycleInner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn set_initial_visibility(&self, visible: bool) {
        let mut inner = self.lock_inner();
        inner.phase = if visible {
            LifecyclePhase::Ready
        } else {
            LifecyclePhase::Hidden
        };
    }

    pub fn note_clipboard_event(&self) {
        self.clipboard_event_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn mark_explicit_exit(&self) {
        self.explicit_exit_requested.store(true, Ordering::SeqCst);
    }

    pub fn should_prevent_exit(&self, code: Option<i32>) -> bool {
        let phase = self.lock_inner().phase;
        self.mode == LifecycleMode::Destroyed
            && code.is_none()
            && !self.explicit_exit_requested.load(Ordering::SeqCst)
            && matches!(
                phase,
                LifecyclePhase::Destroying | LifecyclePhase::Destroyed | LifecyclePhase::Recreating
            )
    }

    pub fn note_native_focus(&self, app: &AppHandle, focused: bool) {
        if !self.enabled() {
            return;
        }

        let (wake, became_ready, phase) = {
            let mut inner = self.lock_inner();
            let visible = main_window_is_visible(app);
            let Some(wake) = inner.active_wake.as_mut() else {
                return;
            };
            if wake.focused == focused {
                return;
            }
            wake.focused = focused;
            let became_ready = wake.usable_ready() && !wake.usable_ready_recorded && visible;
            if became_ready {
                wake.usable_ready_recorded = true;
            }
            let wake = wake.clone();
            if became_ready {
                inner.phase = LifecyclePhase::Ready;
                inner.last_usable_wake_ms = Some(wake.requested_at.elapsed().as_millis() as u64);
            }
            (
                wake,
                became_ready,
                if focused { "focused" } else { "focus_lost" },
            )
        };
        self.record_wake_trace(app, &wake, phase, None);
        if became_ready {
            self.record_wake_trace(app, &wake, "ready", None);
        }
    }

    pub fn note_main_destroyed(&self, app: &AppHandle) {
        if self.mode != LifecycleMode::Destroyed {
            return;
        }

        let wake = {
            let mut inner = self.lock_inner();
            if inner.phase != LifecyclePhase::Destroying {
                return;
            }
            inner.phase = LifecyclePhase::Destroyed;
            inner.active_wake.clone()
        };
        if let Some(wake) = wake {
            self.record_wake_trace(app, &wake, "destroyed_event", None);
        }
    }

    pub fn snapshot(&self, app: &AppHandle) -> LifecycleSnapshot {
        let inner = self.lock_inner();
        let active = inner.active_wake.as_ref();
        let main_window = app.get_webview_window(MAIN_LABEL);
        LifecycleSnapshot {
            enabled: self.enabled(),
            mode: self.mode,
            phase: inner.phase,
            generation: inner.generation,
            active_request_id: active.map(|wake| wake.request_id),
            completed_request_id: inner.completed_request_id,
            failed_request_id: inner.failed_request_id,
            active_intent: active.map(|wake| wake.intent),
            react_mounted: active.is_some_and(|wake| wake.react_mounted),
            hydrated: active.is_some_and(|wake| wake.hydrated),
            search_ready: active.is_some_and(|wake| wake.search_ready),
            search_results_settled: active.is_some_and(|wake| wake.search_results_settled),
            focused: active.is_some_and(|wake| wake.focused),
            requested_visible_focused_hydrated_search_ready_ms: inner.last_usable_wake_ms,
            main_window_count: main_window_count(app),
            main_window_present: main_window.is_some(),
            main_window_visible: main_window
                .as_ref()
                .is_some_and(|window| window.is_visible().unwrap_or(false)),
            main_window_focused: main_window
                .as_ref()
                .is_some_and(|window| window.is_focused().unwrap_or(false)),
            clipboard_event_count: self.clipboard_event_count.load(Ordering::Relaxed),
            persisted_history_count: persisted_history_count(app),
            session_history_count: session_history_count(app),
            history_probe_available: persisted_history_count(app).is_some()
                && session_history_count(app).is_some(),
            explicit_exit_requested: self.explicit_exit_requested.load(Ordering::SeqCst),
            worker_running: inner.worker_running,
            in_flight_target: inner.in_flight_target,
            pending_target: inner
                .pending_operation
                .as_ref()
                .map(|operation| operation.target),
            trace_count: inner.traces.len(),
        }
    }

    pub fn traces(&self) -> Vec<LifecycleTrace> {
        self.lock_inner().traces.iter().cloned().collect()
    }

    fn set_phase(&self, phase: LifecyclePhase) {
        self.lock_inner().phase = phase;
    }

    fn push_operation_outcome(
        inner: &mut LifecycleInner,
        request_id: u64,
        outcome: OperationOutcome,
    ) {
        inner.operation_outcomes.push_back((request_id, outcome));
        while inner.operation_outcomes.len() > OPERATION_OUTCOME_LIMIT {
            inner.operation_outcomes.pop_front();
        }
    }

    fn complete_operation_inner(
        inner: &mut LifecycleInner,
        operation: &LifecycleOperation,
        result: &Result<(), String>,
    ) {
        let outcome = completion_outcome(
            operation.target,
            result,
            inner
                .pending_operation
                .as_ref()
                .map(|pending| pending.target),
        );
        match &outcome {
            OperationOutcome::Succeeded => {
                inner.completed_request_id = Some(operation.request_id);
                inner.failed_request_id = None;
            }
            OperationOutcome::Failed(_) => {
                inner.failed_request_id = Some(operation.request_id);
            }
            OperationOutcome::Superseded => {}
        }
        Self::push_operation_outcome(inner, operation.request_id, outcome);
        inner.in_flight_target = None;
        inner.in_flight_request_id = None;
    }

    fn record_operation_completion(
        &self,
        operation: &LifecycleOperation,
        result: &Result<(), String>,
    ) {
        let mut inner = self.lock_inner();
        Self::complete_operation_inner(&mut inner, operation, result);
    }

    fn current_generation(&self) -> u64 {
        self.lock_inner().generation
    }

    fn begin_wake(&self, operation: &LifecycleOperation, generation: u64) -> ActiveWake {
        let wake = ActiveWake {
            request_id: operation.request_id,
            generation,
            intent: operation.intent,
            requested_at: operation.requested_at,
            react_mounted: false,
            hydrated: false,
            search_ready: false,
            search_results_settled: false,
            focused: false,
            requires_focus: source_requires_focus(operation.source),
            search_focus_sent: false,
            usable_ready_recorded: false,
        };
        let mut inner = self.lock_inner();
        inner.generation = generation;
        inner.active_wake = Some(wake.clone());
        inner.last_usable_wake_ms = None;
        wake
    }

    fn record_wake_trace(
        &self,
        app: &AppHandle,
        wake: &ActiveWake,
        phase: impl Into<String>,
        detail: Option<String>,
    ) {
        self.record_trace(
            app,
            wake.request_id,
            wake.generation,
            wake.intent,
            wake.requested_at,
            None,
            phase,
            detail,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_trace(
        &self,
        app: &AppHandle,
        request_id: u64,
        generation: u64,
        intent: WakeIntent,
        requested_at: Instant,
        timestamp_override: Option<u64>,
        phase: impl Into<String>,
        detail: Option<String>,
    ) {
        let phase = phase.into();
        let trace = LifecycleTrace {
            request_id,
            generation,
            mode: self.mode,
            intent,
            phase: phase.clone(),
            timestamp_unix_ms: timestamp_override.unwrap_or_else(unix_ms),
            elapsed_ms: if phase == "requested" {
                0
            } else {
                requested_at.elapsed().as_millis() as u64
            },
            main_window_count: main_window_count(app),
            clipboard_event_count: self.clipboard_event_count.load(Ordering::Relaxed),
            persisted_history_count: persisted_history_count(app),
            session_history_count: session_history_count(app),
            detail,
        };

        crate::info!(
            ">>> [UI_LIFECYCLE] request={} generation={} mode={} intent={:?} phase={} elapsed_ms={} main_windows={} clipboard_events={}",
            trace.request_id,
            trace.generation,
            trace.mode.as_str(),
            trace.intent,
            trace.phase,
            trace.elapsed_ms,
            trace.main_window_count,
            trace.clipboard_event_count
        );

        let mut inner = self.lock_inner();
        inner.traces.push_back(trace);
        while inner.traces.len() > TRACE_LIMIT {
            inner.traces.pop_front();
        }
    }

    fn flush_pending_traces(&self, app: &AppHandle) {
        let traces = {
            let mut inner = self.lock_inner();
            inner.pending_traces.drain(..).collect::<Vec<_>>()
        };
        for trace in traces {
            self.record_trace(
                app,
                trace.request_id,
                trace.generation,
                trace.intent,
                trace.requested_at,
                Some(trace.requested_unix_ms),
                trace.phase,
                trace.detail,
            );
        }
    }
}

pub fn initialize(app: &mut tauri::App) {
    let lifecycle = MainUiLifecycle::from_environment();
    if lifecycle.enabled() {
        crate::info!(
            ">>> [UI_LIFECYCLE] Issue #154 prototype enabled with mode={}",
            lifecycle.mode().as_str()
        );
    }
    app.manage(lifecycle);
}

pub fn initialize_after_setup(app: &tauri::App) {
    let lifecycle = app.state::<MainUiLifecycle>();
    let visible = app
        .get_webview_window(MAIN_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    lifecycle.set_initial_visibility(visible);

    #[cfg(target_os = "windows")]
    if lifecycle.mode == LifecycleMode::Destroyed {
        if let Err(error) =
            crate::infrastructure::windows_api::drag_drop::replace_emoji_drag_drop_on_current_thread(
                app.handle().clone(),
            )
        {
            crate::error!(
                ">>> [UI_LIFECYCLE] initial replacement OLE drag/drop registration failed: {}",
                error
            );
        }
    }
}

pub fn is_experimental(app: &AppHandle) -> bool {
    app.try_state::<MainUiLifecycle>()
        .is_some_and(|lifecycle| lifecycle.enabled())
}

pub fn note_clipboard_event(app: &AppHandle) {
    if let Some(lifecycle) = app.try_state::<MainUiLifecycle>() {
        lifecycle.note_clipboard_event();
    }
}

pub fn request_toggle(app: &AppHandle, intent: WakeIntent) {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        toggle_window(app);
        return;
    };
    if !lifecycle.enabled() {
        toggle_window(app);
        return;
    }
    let _ = enqueue_operation(app, &lifecycle, None, WakeSource::Toggle, intent, None);
}

pub fn request_show(app: &AppHandle, intent: WakeIntent) {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        show_explicit_default(app);
        return;
    };
    if !lifecycle.enabled() {
        show_explicit_default(app);
        return;
    }
    let _ = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Visible),
        WakeSource::Explicit,
        intent,
        None,
    );
}

pub fn request_search(app: &AppHandle) {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        search_shortcut_default(app);
        return;
    };
    if !lifecycle.enabled() {
        search_shortcut_default(app);
        return;
    }
    let _ = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Visible),
        WakeSource::SearchShortcut,
        WakeIntent::Search,
        None,
    );
}

pub fn request_tray_menu_show(app: &AppHandle) {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        show_tray_menu_default(app);
        return;
    };
    if !lifecycle.enabled() {
        show_tray_menu_default(app);
        return;
    }
    let _ = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Visible),
        WakeSource::TrayMenu,
        WakeIntent::Tray,
        None,
    );
}

pub fn request_tray_click_show(app: &AppHandle) {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        show_tray_click_default(app);
        return;
    };
    if !lifecycle.enabled() {
        show_tray_click_default(app);
        return;
    }
    let _ = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Visible),
        WakeSource::TrayClick,
        WakeIntent::Tray,
        None,
    );
}

pub fn request_hide(app: &AppHandle, reason: HideReason) -> bool {
    let had_window = app.get_webview_window(MAIN_LABEL).is_some();
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        hide_default(app, reason);
        return had_window;
    };
    if !lifecycle.enabled() {
        hide_default(app, reason);
        return had_window;
    }
    let _ = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Hidden),
        if reason == HideReason::Test {
            WakeSource::Test
        } else {
            WakeSource::Explicit
        },
        if reason == HideReason::Test {
            WakeIntent::Test
        } else {
            WakeIntent::Main
        },
        Some(reason),
    );
    had_window
}

pub async fn request_hide_and_wait(app: &AppHandle, reason: HideReason) -> Result<bool, String> {
    let had_window = app.get_webview_window(MAIN_LABEL).is_some();
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        hide_default(app, reason);
        return Ok(had_window);
    };
    if !lifecycle.enabled() {
        hide_default(app, reason);
        return Ok(had_window);
    }

    let request_id = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Hidden),
        WakeSource::Explicit,
        WakeIntent::Main,
        Some(reason),
    );
    wait_for_hide_completion(app, &lifecycle, request_id, OPERATION_COMPLETION_TIMEOUT).await?;
    Ok(had_window)
}

async fn wait_for_hide_completion(
    app: &AppHandle,
    lifecycle: &MainUiLifecycle,
    request_id: u64,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let outcome = lifecycle
            .lock_inner()
            .operation_outcomes
            .iter()
            .rev()
            .find_map(|(completed_request_id, outcome)| {
                (*completed_request_id == request_id).then(|| outcome.clone())
            });
        match outcome {
            Some(OperationOutcome::Succeeded) => {
                let (phase, native_down) = match lifecycle.mode {
                    LifecycleMode::Destroyed => (
                        LifecyclePhase::Destroyed,
                        app.get_webview_window(MAIN_LABEL).is_none(),
                    ),
                    LifecycleMode::Hidden => (
                        LifecyclePhase::Hidden,
                        app.get_webview_window(MAIN_LABEL)
                            .is_some_and(|window| !window.is_visible().unwrap_or(true)),
                    ),
                    LifecycleMode::Default => (LifecyclePhase::Hidden, true),
                };
                if native_down && lifecycle.lock_inner().phase == phase {
                    return Ok(());
                }
                return Err(format!(
                    "lifecycle hide request {request_id} completed without the requested native down state"
                ));
            }
            Some(OperationOutcome::Failed(error)) => {
                return Err(format!(
                    "lifecycle hide request {request_id} failed: {error}"
                ));
            }
            Some(OperationOutcome::Superseded) => {
                return Err(format!(
                    "lifecycle hide request {request_id} was superseded"
                ));
            }
            None => {}
        }
        if started.elapsed() >= timeout {
            return Err("timed out waiting for lifecycle hide completion".to_string());
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
}

fn enqueue_operation(
    app: &AppHandle,
    lifecycle: &MainUiLifecycle,
    explicit_target: Option<TargetVisibility>,
    source: WakeSource,
    intent: WakeIntent,
    hide_reason: Option<HideReason>,
) -> u64 {
    let new_request_id = lifecycle.next_request_id.fetch_add(1, Ordering::Relaxed);
    let mut selected_request_id = new_request_id;
    let requested_at = Instant::now();
    let requested_unix_ms = unix_ms();
    let visible = main_window_is_visible(app);
    let mut should_spawn = false;

    {
        let mut inner = lifecycle.lock_inner();
        let target = explicit_target.unwrap_or_else(|| {
            if let Some(target) = inner
                .pending_operation
                .as_ref()
                .map(|operation| operation.target)
                .or(inner.in_flight_target)
            {
                // Repeated toggles during an in-flight transition are one logical request.
                // This prevents show-then-hide races from duplicate hooks or hotkey delivery.
                target
            } else if visible {
                TargetVisibility::Hidden
            } else {
                TargetVisibility::Visible
            }
        });
        let generation = if target == TargetVisibility::Visible
            && lifecycle.mode == LifecycleMode::Destroyed
            && (app.get_webview_window(MAIN_LABEL).is_none()
                || inner.in_flight_target == Some(TargetVisibility::Hidden)
                || inner
                    .pending_operation
                    .as_ref()
                    .is_some_and(|operation| operation.target == TargetVisibility::Hidden))
        {
            inner.generation.saturating_add(1)
        } else {
            inner.generation
        };
        let operation = LifecycleOperation {
            request_id: new_request_id,
            intent,
            requested_at,
            target,
            source,
            hide_reason: if target == TargetVisibility::Hidden {
                Some(hide_reason.unwrap_or(HideReason::Toggle))
            } else {
                None
            },
        };

        inner.pending_traces.push_back(PendingTrace {
            request_id: new_request_id,
            generation,
            intent,
            requested_at,
            requested_unix_ms,
            phase: "requested",
            detail: Some(format!("target={target:?};source={source:?}")),
        });

        let mut coalesced = false;
        let in_flight_request_id = inner.in_flight_request_id;
        if inner.in_flight_target == Some(target) {
            coalesced = true;
            if target == TargetVisibility::Visible && intent.requires_search() {
                if let Some(wake) = inner.active_wake.as_mut() {
                    wake.intent = WakeIntent::Search;
                }
            }
            selected_request_id = canonical_request_id(
                new_request_id,
                target,
                in_flight_request_id.zip(inner.in_flight_target),
                inner
                    .pending_operation
                    .as_ref()
                    .map(|pending| (pending.request_id, pending.target)),
            );
            if let Some(replaced) = inner.pending_operation.take() {
                MainUiLifecycle::push_operation_outcome(
                    &mut inner,
                    replaced.request_id,
                    OperationOutcome::Superseded,
                );
            }
        } else if let Some(pending) = inner.pending_operation.as_mut() {
            if pending.target == target {
                coalesced = true;
                selected_request_id = canonical_request_id(
                    new_request_id,
                    target,
                    None,
                    Some((pending.request_id, pending.target)),
                );
                if intent.requires_search() {
                    pending.intent = WakeIntent::Search;
                    pending.source = source;
                }
            } else {
                let replaced = inner
                    .pending_operation
                    .replace(operation)
                    .expect("pending operation");
                MainUiLifecycle::push_operation_outcome(
                    &mut inner,
                    replaced.request_id,
                    OperationOutcome::Superseded,
                );
            }
        } else {
            inner.pending_operation = Some(operation);
        }

        if coalesced {
            inner.pending_traces.push_back(PendingTrace {
                request_id: new_request_id,
                generation,
                intent,
                requested_at,
                requested_unix_ms,
                phase: "coalesced",
                detail: Some(format!(
                    "target={target:?};canonical_request_id={selected_request_id}"
                )),
            });
        }

        if !inner.worker_running {
            inner.worker_running = true;
            should_spawn = true;
        }
    }

    if should_spawn {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            run_worker(app).await;
        });
    }
    selected_request_id
}

pub fn request_test_hide(app: &AppHandle) -> Result<(u64, u64), String> {
    let lifecycle = app.state::<MainUiLifecycle>();
    if !lifecycle.enabled() {
        return Err(format!(
            "{LIFECYCLE_ENV}=hidden or destroyed is required for lifecycle_test_hide"
        ));
    }
    let generation_before = lifecycle.current_generation();
    let request_id = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Hidden),
        WakeSource::Test,
        WakeIntent::Test,
        Some(HideReason::Test),
    );
    Ok((request_id, generation_before))
}

pub fn request_test_show(app: &AppHandle) -> Result<(u64, u64, u64), String> {
    let lifecycle = app.state::<MainUiLifecycle>();
    if !lifecycle.enabled() {
        return Err(format!(
            "{LIFECYCLE_ENV}=hidden or destroyed is required for lifecycle_test_show"
        ));
    }
    let generation_before = lifecycle.current_generation();
    let expected_generation = expected_show_generation(
        lifecycle.mode,
        app.get_webview_window(MAIN_LABEL).is_some(),
        generation_before,
    );
    let request_id = enqueue_operation(
        app,
        &lifecycle,
        Some(TargetVisibility::Visible),
        WakeSource::Test,
        WakeIntent::Test,
        None,
    );
    Ok((request_id, generation_before, expected_generation))
}

fn expected_show_generation(
    mode: LifecycleMode,
    main_window_present: bool,
    generation_before: u64,
) -> u64 {
    if mode == LifecycleMode::Destroyed && !main_window_present {
        generation_before.saturating_add(1)
    } else {
        generation_before
    }
}

async fn run_worker(app: AppHandle) {
    loop {
        let lifecycle = app.state::<MainUiLifecycle>();
        lifecycle.flush_pending_traces(&app);

        let operation = {
            let mut inner = lifecycle.lock_inner();
            let operation = inner.pending_operation.take();
            if let Some(operation) = operation.as_ref() {
                inner.in_flight_target = Some(operation.target);
                inner.in_flight_request_id = Some(operation.request_id);
            } else if inner.pending_traces.is_empty() {
                inner.in_flight_target = None;
                inner.in_flight_request_id = None;
                inner.worker_running = false;
                return;
            }
            operation
        };

        let Some(operation) = operation else {
            continue;
        };

        let result = match operation.target {
            TargetVisibility::Visible => run_show_operation(&app, &lifecycle, &operation).await,
            TargetVisibility::Hidden => run_hide_operation(&app, &lifecycle, &operation).await,
        };

        if let Err(error) = &result {
            lifecycle.record_trace(
                &app,
                operation.request_id,
                lifecycle.current_generation(),
                operation.intent,
                operation.requested_at,
                None,
                "failed",
                Some(error.clone()),
            );
            crate::error!(
                ">>> [UI_LIFECYCLE] request={} target={:?} failed: {}",
                operation.request_id,
                operation.target,
                error
            );
            lifecycle.set_phase(phase_from_native_state(&app, lifecycle.mode));
        }

        lifecycle.record_operation_completion(&operation, &result);
    }
}

async fn run_show_operation(
    app: &AppHandle,
    lifecycle: &MainUiLifecycle,
    operation: &LifecycleOperation,
) -> Result<(), String> {
    let must_recreate =
        lifecycle.mode == LifecycleMode::Destroyed && app.get_webview_window(MAIN_LABEL).is_none();
    let generation = if must_recreate {
        lifecycle.current_generation().saturating_add(1)
    } else {
        lifecycle.current_generation()
    };
    let wake = lifecycle.begin_wake(operation, generation);

    if must_recreate {
        lifecycle.set_phase(LifecyclePhase::Recreating);
        lifecycle.record_wake_trace(app, &wake, "recreating", None);
        recreate_main_window(app).await?;
        lifecycle.set_phase(LifecyclePhase::AwaitingFrontend);
        lifecycle.record_wake_trace(app, &wake, "window_created_hidden", None);
        let ready = wait_for_frontend_ready(app, lifecycle, &wake).await?;
        if lifecycle.pending_target() == Some(TargetVisibility::Hidden) {
            lifecycle.record_wake_trace(app, &wake, "show_superseded", None);
            return Ok(());
        }
        if !ready {
            lifecycle.record_wake_trace(
                app,
                &wake,
                "frontend_ready_timeout",
                Some(format!("timeout_ms={}", FRONTEND_READY_TIMEOUT.as_millis())),
            );
        }
    } else {
        lifecycle.set_phase(LifecyclePhase::AwaitingFrontend);
        emit_wake(app, lifecycle, &wake);
    }

    if lifecycle.pending_target() == Some(TargetVisibility::Hidden) {
        lifecycle.record_wake_trace(app, &wake, "show_superseded", None);
        return Ok(());
    }

    let window = app
        .get_webview_window(MAIN_LABEL)
        .ok_or_else(|| "main window is unavailable after wake".to_string())?;
    show_using_legacy_path(app, &window, operation.source)?;
    lifecycle.record_wake_trace(app, &wake, "visible", None);
    if source_requires_focus(operation.source) {
        crate::app::window_manager::activate_window_focus(app.clone())?;
    }

    let (usable_ready, should_focus_search) = {
        let mut inner = lifecycle.lock_inner();
        let (usable_ready, became_ready, should_focus_search, elapsed_ms) = {
            let active = inner
                .active_wake
                .as_mut()
                .filter(|active| active.request_id == wake.request_id);
            let usable_ready = active.as_ref().is_some_and(|active| active.usable_ready());
            let became_ready = active
                .as_ref()
                .is_some_and(|active| usable_ready && !active.usable_ready_recorded);
            let should_focus_search = active.as_ref().is_some_and(|active| {
                active.intent.requires_search() && active.search_ready && !active.search_focus_sent
            });
            let elapsed_ms = active
                .as_ref()
                .map(|active| active.requested_at.elapsed().as_millis() as u64);
            if let Some(active) = active {
                if should_focus_search {
                    active.search_focus_sent = true;
                }
                if became_ready {
                    active.usable_ready_recorded = true;
                }
            }
            (usable_ready, became_ready, should_focus_search, elapsed_ms)
        };
        inner.phase = if usable_ready {
            LifecyclePhase::Ready
        } else {
            LifecyclePhase::AwaitingFrontend
        };
        if became_ready {
            inner.last_usable_wake_ms = elapsed_ms;
        }
        (became_ready, should_focus_search)
    };

    if should_focus_search {
        let _ = app.emit_to(MAIN_LABEL, "focus-search-input", ());
    }
    if usable_ready {
        lifecycle.record_wake_trace(app, &wake, "ready", None);
    }
    Ok(())
}

fn source_requires_focus(source: WakeSource) -> bool {
    source != WakeSource::TrayMenu
}

impl MainUiLifecycle {
    fn pending_target(&self) -> Option<TargetVisibility> {
        self.lock_inner()
            .pending_operation
            .as_ref()
            .map(|operation| operation.target)
    }
}

async fn run_hide_operation(
    app: &AppHandle,
    lifecycle: &MainUiLifecycle,
    operation: &LifecycleOperation,
) -> Result<(), String> {
    let reason = operation.hide_reason.unwrap_or(HideReason::Toggle);
    let Some(window) = app.get_webview_window(MAIN_LABEL) else {
        lifecycle.set_phase(if lifecycle.mode == LifecycleMode::Destroyed {
            LifecyclePhase::Destroyed
        } else {
            LifecyclePhase::Hidden
        });
        lifecycle.record_trace(
            app,
            operation.request_id,
            lifecycle.current_generation(),
            operation.intent,
            operation.requested_at,
            None,
            if lifecycle.mode == LifecycleMode::Destroyed {
                "destroyed"
            } else {
                "hidden"
            },
            Some(format!("{};window_already_absent", reason.as_str())),
        );
        return Ok(());
    };

    hide_default(app, reason);
    reset_experimental_hidden_globals();

    if lifecycle.mode != LifecycleMode::Destroyed {
        lifecycle.set_phase(LifecyclePhase::Hidden);
        lifecycle.record_trace(
            app,
            operation.request_id,
            lifecycle.current_generation(),
            operation.intent,
            operation.requested_at,
            None,
            "hidden",
            Some(reason.as_str().to_string()),
        );
        return Ok(());
    }

    lifecycle.set_phase(LifecyclePhase::Destroying);
    lifecycle.record_trace(
        app,
        operation.request_id,
        lifecycle.current_generation(),
        operation.intent,
        operation.requested_at,
        None,
        "destroying",
        Some(reason.as_str().to_string()),
    );

    let _ = app.emit_to("compact-preview", "force-hide-compact-preview", ());
    if let Some(preview) = app.get_webview_window("compact-preview") {
        preview.destroy().map_err(|error| {
            format!("failed to destroy compact preview before main teardown: {error}")
        })?;
        wait_until_window_removed(app, "compact-preview", DESTROY_TIMEOUT).await?;
    }

    #[cfg(target_os = "windows")]
    revoke_recreated_native_resources(app).await?;

    window.destroy().map_err(|error| error.to_string())?;
    wait_until_main_removed(app, DESTROY_TIMEOUT).await?;
    lifecycle.set_phase(LifecyclePhase::Destroyed);
    lifecycle.record_trace(
        app,
        operation.request_id,
        lifecycle.current_generation(),
        operation.intent,
        operation.requested_at,
        None,
        "destroyed",
        Some(reason.as_str().to_string()),
    );
    Ok(())
}

fn show_using_legacy_path(
    app: &AppHandle,
    window: &WebviewWindow,
    source: WakeSource,
) -> Result<(), String> {
    match source {
        WakeSource::TrayMenu => show_tray_menu_default(app),
        WakeSource::TrayClick => show_tray_click_default(app),
        WakeSource::Toggle
        | WakeSource::Explicit
        | WakeSource::SearchShortcut
        | WakeSource::Test => {
            if !window.is_visible().unwrap_or(false) || IS_HIDDEN.load(Ordering::Relaxed) {
                toggle_window(app);
            }
        }
    }
    if window.is_visible().unwrap_or(false) {
        Ok(())
    } else {
        Err("legacy show path did not make the main window visible".to_string())
    }
}

fn show_explicit_default(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        if !window.is_visible().unwrap_or(false) || IS_HIDDEN.load(Ordering::Relaxed) {
            toggle_window(app);
        }
    }
}

fn search_shortcut_default(app: &AppHandle) {
    toggle_window(app);
    let _ = app.emit("focus-search-input", ());
}

fn show_tray_menu_default(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.show();
        let _ = app.emit("main-window-opened", ());
    }
}

fn show_tray_click_default(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit("main-window-opened", ());
        LAST_SHOW_TIMESTAMP.store(unix_ms(), Ordering::Relaxed);
    }
}

fn hide_default(app: &AppHandle, reason: HideReason) {
    let Some(window) = app.get_webview_window(MAIN_LABEL) else {
        return;
    };

    match reason {
        HideReason::Toggle | HideReason::Test => toggle_window(app),
        HideReason::CloseRequested => {
            let _ = window.hide();
            NAVIGATION_ENABLED.store(false, Ordering::SeqCst);
            NAVIGATION_MODE_ACTIVE.store(false, Ordering::SeqCst);
        }
        HideReason::Blur => {
            let _ = window.hide();
            NAVIGATION_ENABLED.store(false, Ordering::SeqCst);
            crate::app::window_manager::release_win_keys();
            let _ = restore_last_focus(app.clone());
        }
        HideReason::FrontendCommand => {
            #[cfg(target_os = "windows")]
            crate::infrastructure::windows_ext::WindowExt::release_win_keys();
            let _ = window.set_focusable(false);
            let _ = window.hide();
            NAVIGATION_ENABLED.store(false, Ordering::SeqCst);
            NAVIGATION_MODE_ACTIVE.store(false, Ordering::SeqCst);
            let _ = restore_last_focus(app.clone());
        }
        HideReason::PasteFocusRestore => {
            let _ = window.hide();
            IS_HIDDEN.store(false, Ordering::Relaxed);
            crate::app::window_manager::release_win_keys();
        }
        HideReason::AfterPaste => {
            let _ = window.set_focusable(false);
            let _ = window.hide();
            IS_HIDDEN.store(false, Ordering::Relaxed);
            NAVIGATION_ENABLED.store(false, Ordering::Relaxed);
            crate::app::window_manager::release_win_keys();
        }
    }
}

fn reset_experimental_hidden_globals() {
    IS_HIDDEN.store(false, Ordering::Relaxed);
    IS_MAIN_WINDOW_FOCUSED.store(false, Ordering::Relaxed);
    NAVIGATION_ENABLED.store(false, Ordering::SeqCst);
    NAVIGATION_MODE_ACTIVE.store(false, Ordering::SeqCst);
    CURRENT_DOCK.store(0, Ordering::Relaxed);
}

async fn wait_until_main_removed(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    wait_until_window_removed(app, MAIN_LABEL, timeout).await
}

async fn wait_until_window_removed(
    app: &AppHandle,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    while app.get_webview_window(label).is_some() {
        if started.elapsed() >= timeout {
            return Err(format!(
                "timed out waiting for the old '{label}' label to be removed"
            ));
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    Ok(())
}

async fn wait_for_frontend_ready(
    app: &AppHandle,
    lifecycle: &MainUiLifecycle,
    wake: &ActiveWake,
) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        if lifecycle.pending_target() == Some(TargetVisibility::Hidden) {
            return Ok(false);
        }
        if app.get_webview_window(MAIN_LABEL).is_none() {
            return Err("main window disappeared while awaiting frontend readiness".to_string());
        }
        let ready = {
            let inner = lifecycle.lock_inner();
            inner
                .active_wake
                .as_ref()
                .filter(|active| {
                    active.request_id == wake.request_id && active.generation == wake.generation
                })
                .is_some_and(ActiveWake::frontend_ready)
        };
        if ready {
            return Ok(true);
        }
        if started.elapsed() >= FRONTEND_READY_TIMEOUT {
            return Ok(false);
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
}

async fn recreate_main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == MAIN_LABEL)
        .cloned()
        .ok_or_else(|| "tauri.conf.json has no main window config".to_string())?;

    config.visible = false;
    config.focus = false;

    let builder_app = app.clone();
    let window = tauri::async_runtime::spawn_blocking(move || {
        tauri::WebviewWindowBuilder::from_config(&builder_app, &config)
            .and_then(|builder| builder.build())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("main window build task failed: {error}"))??;

    prepare_recreated_native_resources(app, &window).await?;
    Ok(window)
}

async fn prepare_recreated_native_resources(
    app: &AppHandle,
    window: &WebviewWindow,
) -> Result<(), String> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let app = app.clone();
    let window = window.clone();
    window
        .clone()
        .run_on_main_thread(move || {
            let result =
                crate::app::setup::prepare_recreated_main_window_on_ui_thread(&app, &window);
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    receiver
        .await
        .map_err(|_| "UI thread dropped native resource install acknowledgement".to_string())?
}

#[cfg(target_os = "windows")]
async fn revoke_recreated_native_resources(app: &AppHandle) -> Result<(), String> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let app = app.clone();
    app.run_on_main_thread(move || {
        crate::infrastructure::windows_api::drag_drop::revoke_emoji_drag_drop_on_current_thread();
        let _ = sender.send(());
    })
    .map_err(|error| error.to_string())?;
    receiver
        .await
        .map_err(|_| "UI thread dropped drag/drop revoke acknowledgement".to_string())
}

fn emit_wake(app: &AppHandle, lifecycle: &MainUiLifecycle, wake: &ActiveWake) {
    let _ = app.emit_to(
        MAIN_LABEL,
        "main-ui-lifecycle-wake",
        MainUiBootstrap {
            enabled: true,
            mode: lifecycle.mode,
            generation: wake.generation,
            request_id: Some(wake.request_id),
            intent: Some(wake.intent),
        },
    );
}

pub fn note_native_window_event(app: &AppHandle, label: &str, event: &tauri::WindowEvent) {
    if label != MAIN_LABEL {
        return;
    }
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        return;
    };
    match event {
        tauri::WindowEvent::Focused(focused) => lifecycle.note_native_focus(app, *focused),
        tauri::WindowEvent::Destroyed => lifecycle.note_main_destroyed(app),
        _ => {}
    }
}

pub fn handle_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        return;
    };
    if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
        if lifecycle.should_prevent_exit(*code) {
            api.prevent_exit();
            crate::info!(
                ">>> [UI_LIFECYCLE] prevented last-window exit during an expected destroyed transition"
            );
        }
    }
}

pub fn mark_explicit_exit(app: &AppHandle) {
    if let Some(lifecycle) = app.try_state::<MainUiLifecycle>() {
        lifecycle.mark_explicit_exit();
    }
}

#[tauri::command]
pub fn get_main_ui_lifecycle_bootstrap(app: AppHandle) -> Result<MainUiBootstrap, String> {
    let lifecycle = app.state::<MainUiLifecycle>();
    let inner = lifecycle.lock_inner();
    Ok(MainUiBootstrap {
        enabled: lifecycle.enabled(),
        mode: lifecycle.mode,
        generation: inner.generation,
        request_id: inner.active_wake.as_ref().map(|wake| wake.request_id),
        intent: inner.active_wake.as_ref().map(|wake| wake.intent),
    })
}

#[tauri::command]
pub fn report_main_ui_ready(app: AppHandle, report: MainUiReadyReport) -> Result<(), String> {
    let lifecycle = app.state::<MainUiLifecycle>();
    if !lifecycle.enabled() {
        return Ok(());
    }

    let (wake, should_focus_search, became_ready) = {
        let mut inner = lifecycle.lock_inner();
        let Some(wake) = inner.active_wake.as_mut() else {
            return Err("no active lifecycle wake".to_string());
        };
        if wake.request_id != report.request_id || wake.generation != report.generation {
            return Err("stale lifecycle ready report".to_string());
        }
        match report.phase {
            UiReadyPhase::ReactMounted => wake.react_mounted = true,
            UiReadyPhase::Hydrated => wake.hydrated = true,
            UiReadyPhase::SearchReady => wake.search_ready = true,
            UiReadyPhase::SearchResultsSettled => wake.search_results_settled = true,
        }
        let should_focus_search = wake.intent.requires_search()
            && wake.search_ready
            && !wake.search_focus_sent
            && main_window_is_visible(&app);
        if should_focus_search {
            wake.search_focus_sent = true;
        }
        let native_focused = main_window_is_focused(&app);
        wake.focused = native_focused;
        let usable_ready = wake.usable_ready()
            && main_window_is_visible(&app)
            && (!wake.requires_focus || native_focused);
        let became_ready = usable_ready && !wake.usable_ready_recorded;
        if became_ready {
            wake.usable_ready_recorded = true;
        }
        let wake = wake.clone();
        if became_ready && main_window_is_visible(&app) {
            inner.phase = LifecyclePhase::Ready;
        }
        if became_ready {
            inner.last_usable_wake_ms = Some(wake.requested_at.elapsed().as_millis() as u64);
        }
        (wake, should_focus_search, became_ready)
    };

    lifecycle.record_wake_trace(
        &app,
        &wake,
        match report.phase {
            UiReadyPhase::ReactMounted => "react_mounted",
            UiReadyPhase::Hydrated => "hydrated",
            UiReadyPhase::SearchReady => "search_ready",
            UiReadyPhase::SearchResultsSettled => "search_results_settled",
        },
        report.detail,
    );
    if should_focus_search {
        let _ = app.emit_to(MAIN_LABEL, "focus-search-input", ());
    }
    if became_ready && main_window_is_visible(&app) {
        lifecycle.record_wake_trace(&app, &wake, "ready", None);
    }
    Ok(())
}

pub fn get_main_ui_lifecycle_snapshot(app: AppHandle) -> Result<LifecycleSnapshot, String> {
    Ok(app.state::<MainUiLifecycle>().snapshot(&app))
}

pub fn get_main_ui_lifecycle_traces(app: AppHandle) -> Result<Vec<LifecycleTrace>, String> {
    Ok(app.state::<MainUiLifecycle>().traces())
}

pub fn get_main_ui_lifecycle_clipboard_probe(
    app: &AppHandle,
    token: &str,
    clipboard_event_count_before: u64,
) -> Result<LifecycleClipboardProbe, String> {
    if token.is_empty() || token.len() > 512 {
        return Err("clipboard probe token must contain 1 to 512 bytes".to_string());
    }

    let db_state = app
        .try_state::<DbState>()
        .ok_or_else(|| "clipboard database state is unavailable".to_string())?;
    let persisted_entry_ids = db_state.repo.find_exact_text_entry_ids(token)?;
    let persisted_entry_id = persisted_entry_ids.first().copied();
    let session_entry_ids = app
        .try_state::<SessionHistory>()
        .ok_or_else(|| "session history state is unavailable".to_string())?
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .filter(|entry| entry.content_type == "text" && entry.content == token)
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    let session_entry_id = session_entry_ids.first().copied();
    let exact_history_match_count = persisted_entry_ids.len() + session_entry_ids.len();

    let clipboard_event_count = app
        .state::<MainUiLifecycle>()
        .clipboard_event_count
        .load(Ordering::Relaxed);

    Ok(LifecycleClipboardProbe {
        token: token.to_string(),
        clipboard_event_count,
        clipboard_event_count_before,
        clipboard_event_delta: clipboard_event_count.saturating_sub(clipboard_event_count_before),
        listener_event_count_increased: clipboard_event_count > clipboard_event_count_before,
        exact_history_match: exact_history_match_count == 1,
        exact_history_match_count,
        persisted_entry_id,
        session_entry_id,
    })
}

pub fn lifecycle_test_toggle(app: AppHandle) -> Result<(), String> {
    if !app.state::<MainUiLifecycle>().enabled() {
        return Err(format!(
            "{LIFECYCLE_ENV}=hidden or destroyed is required for lifecycle_test_toggle"
        ));
    }
    request_hide(&app, HideReason::Test);
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        loop {
            let phase = app.state::<MainUiLifecycle>().lock_inner().phase;
            if matches!(phase, LifecyclePhase::Hidden | LifecyclePhase::Destroyed) {
                tokio::time::sleep(TEST_DOWN_SETTLE).await;
                let _ = enqueue_operation(
                    &app,
                    &app.state::<MainUiLifecycle>(),
                    Some(TargetVisibility::Visible),
                    WakeSource::Test,
                    WakeIntent::Test,
                    None,
                );
                return;
            }
            if started.elapsed() >= DESTROY_TIMEOUT {
                crate::error!(
                    ">>> [UI_LIFECYCLE] lifecycle_test_toggle timed out waiting for down phase"
                );
                return;
            }
            tokio::time::sleep(WAIT_STEP).await;
        }
    });
    Ok(())
}

fn main_window_is_visible(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
        && !IS_HIDDEN.load(Ordering::Relaxed)
}

fn main_window_is_focused(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_LABEL)
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}

fn phase_from_native_state(app: &AppHandle, mode: LifecycleMode) -> LifecyclePhase {
    if app.get_webview_window(MAIN_LABEL).is_none() && mode == LifecycleMode::Destroyed {
        LifecyclePhase::Destroyed
    } else if main_window_is_visible(app) {
        LifecyclePhase::Ready
    } else {
        LifecyclePhase::Hidden
    }
}

fn main_window_count(app: &AppHandle) -> usize {
    app.webview_windows()
        .keys()
        .filter(|label| label.as_str() == MAIN_LABEL)
        .count()
}

fn persisted_history_count(app: &AppHandle) -> Option<i64> {
    app.try_state::<DbState>()
        .and_then(|state| state.repo.get_count().ok())
}

fn session_history_count(app: &AppHandle) -> Option<usize> {
    app.try_state::<SessionHistory>().map(|history| {
        history
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    })
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        expected_show_generation, ActiveWake, HideReason, LifecycleMode, LifecycleOperation,
        LifecyclePhase, MainUiLifecycle, OperationOutcome, TargetVisibility, WakeIntent,
        WakeSource,
    };
    use std::time::Instant;

    fn wake(intent: WakeIntent) -> ActiveWake {
        ActiveWake {
            request_id: 1,
            generation: 1,
            intent,
            requested_at: Instant::now(),
            react_mounted: false,
            hydrated: false,
            search_ready: false,
            search_results_settled: false,
            focused: false,
            requires_focus: true,
            search_focus_sent: false,
            usable_ready_recorded: false,
        }
    }

    #[test]
    fn lifecycle_mode_accepts_only_internal_experiment_values() {
        assert_eq!(LifecycleMode::from_value(None), LifecycleMode::Default);
        assert_eq!(
            LifecycleMode::from_value(Some(" hidden ")),
            LifecycleMode::Hidden
        );
        assert_eq!(
            LifecycleMode::from_value(Some("DESTROYED")),
            LifecycleMode::Destroyed
        );
        assert_eq!(
            LifecycleMode::from_value(Some("anything-else")),
            LifecycleMode::Default
        );
    }

    #[test]
    fn show_generation_only_advances_for_destroyed_window_recreation() {
        assert_eq!(expected_show_generation(LifecycleMode::Hidden, true, 7), 7);
        assert_eq!(
            expected_show_generation(LifecycleMode::Destroyed, true, 7),
            7
        );
        assert_eq!(
            expected_show_generation(LifecycleMode::Destroyed, false, 7),
            8
        );
    }

    #[test]
    fn exit_prevention_is_limited_to_expected_window_absence_phases() {
        let default = MainUiLifecycle::new(LifecycleMode::Default);
        assert!(!default.should_prevent_exit(None));

        let hidden = MainUiLifecycle::new(LifecycleMode::Hidden);
        assert!(!hidden.should_prevent_exit(None));

        let destroyed = MainUiLifecycle::new(LifecycleMode::Destroyed);
        assert!(!destroyed.should_prevent_exit(None));
        destroyed.set_phase(LifecyclePhase::Destroying);
        assert!(destroyed.should_prevent_exit(None));
        destroyed.set_phase(LifecyclePhase::Destroyed);
        assert!(destroyed.should_prevent_exit(None));
        destroyed.set_phase(LifecyclePhase::Recreating);
        assert!(destroyed.should_prevent_exit(None));
        destroyed.set_phase(LifecyclePhase::AwaitingFrontend);
        assert!(!destroyed.should_prevent_exit(None));
        assert!(!destroyed.should_prevent_exit(Some(0)));
        destroyed.set_phase(LifecyclePhase::Destroyed);
        destroyed.mark_explicit_exit();
        assert!(!destroyed.should_prevent_exit(None));
    }

    #[test]
    fn search_wake_requires_hydration_search_ready_and_native_focus() {
        let mut wake = wake(WakeIntent::Search);
        wake.hydrated = true;
        assert!(!wake.usable_ready());
        wake.search_ready = true;
        assert!(!wake.usable_ready());
        wake.focused = true;
        assert!(wake.usable_ready());
    }

    #[test]
    fn ordinary_wake_requires_hydration_and_native_focus_but_not_search_ready() {
        let mut wake = wake(WakeIntent::Main);
        assert!(!wake.usable_ready());
        wake.hydrated = true;
        assert!(!wake.usable_ready());
        wake.focused = true;
        assert!(wake.usable_ready());
    }

    #[test]
    fn tray_menu_show_preserves_no_focus_baseline() {
        assert!(!super::source_requires_focus(WakeSource::TrayMenu));
        assert!(super::source_requires_focus(WakeSource::TrayClick));
        assert!(super::source_requires_focus(WakeSource::Toggle));
        assert!(super::source_requires_focus(WakeSource::Explicit));
        assert!(super::source_requires_focus(WakeSource::SearchShortcut));
        assert!(super::source_requires_focus(WakeSource::Test));

        let mut tray_wake = wake(WakeIntent::Tray);
        tray_wake.requires_focus = false;
        tray_wake.hydrated = true;
        assert!(tray_wake.usable_ready());

        let mut tray_click_wake = wake(WakeIntent::Tray);
        tray_click_wake.hydrated = true;
        assert!(!tray_click_wake.usable_ready());
        tray_click_wake.focused = true;
        assert!(tray_click_wake.usable_ready());
    }

    #[test]
    fn usable_ready_recorded_distinguishes_first_settlement() {
        let mut wake = wake(WakeIntent::Test);
        wake.hydrated = true;
        wake.search_ready = true;
        wake.focused = true;
        assert!(wake.usable_ready() && !wake.usable_ready_recorded);
        wake.usable_ready_recorded = true;
        assert!(wake.usable_ready() && wake.usable_ready_recorded);
    }

    #[test]
    fn failed_operation_is_not_reported_as_completed() {
        let lifecycle = MainUiLifecycle::new(LifecycleMode::Destroyed);

        let failed = LifecycleOperation {
            request_id: 7,
            intent: WakeIntent::Main,
            requested_at: Instant::now(),
            target: TargetVisibility::Hidden,
            source: WakeSource::Explicit,
            hide_reason: Some(HideReason::AfterPaste),
        };
        lifecycle.record_operation_completion(&failed, &Err("failed".to_string()));
        {
            let inner = lifecycle.lock_inner();
            assert_eq!(inner.completed_request_id, None);
            assert_eq!(inner.failed_request_id, Some(7));
        }

        let succeeded = LifecycleOperation {
            request_id: 8,
            ..failed
        };
        lifecycle.record_operation_completion(&succeeded, &Ok(()));
        let inner = lifecycle.lock_inner();
        assert_eq!(inner.completed_request_id, Some(8));
        assert_eq!(inner.failed_request_id, None);
    }

    #[test]
    fn operation_outcomes_cover_success_failure_and_supersession() {
        let lifecycle = MainUiLifecycle::new(LifecycleMode::Destroyed);
        let operation = LifecycleOperation {
            request_id: 11,
            intent: WakeIntent::Main,
            requested_at: Instant::now(),
            target: TargetVisibility::Hidden,
            source: WakeSource::Explicit,
            hide_reason: Some(HideReason::PasteFocusRestore),
        };
        {
            let mut inner = lifecycle.lock_inner();
            MainUiLifecycle::push_operation_outcome(
                &mut inner,
                operation.request_id,
                OperationOutcome::Superseded,
            );
            assert_eq!(
                inner.operation_outcomes.back(),
                Some(&(11, OperationOutcome::Superseded))
            );
            MainUiLifecycle::complete_operation_inner(&mut inner, &operation, &Ok(()));
            assert_eq!(
                inner.operation_outcomes.back(),
                Some(&(11, OperationOutcome::Succeeded))
            );
            MainUiLifecycle::complete_operation_inner(
                &mut inner,
                &LifecycleOperation {
                    request_id: 12,
                    ..operation
                },
                &Err("boom".to_string()),
            );
            assert_eq!(
                inner.operation_outcomes.back(),
                Some(&(12, OperationOutcome::Failed("boom".to_string())))
            );
        }
    }

    #[test]
    fn opposite_pending_target_supersedes_completed_operation_for_waiters() {
        assert_eq!(
            super::completion_outcome(
                TargetVisibility::Hidden,
                &Ok(()),
                Some(TargetVisibility::Visible),
            ),
            OperationOutcome::Superseded
        );
        assert_eq!(
            super::completion_outcome(TargetVisibility::Hidden, &Ok(()), None),
            OperationOutcome::Succeeded
        );
    }

    #[test]
    fn same_target_coalescing_returns_the_canonical_request_id() {
        assert_eq!(
            super::canonical_request_id(
                23,
                TargetVisibility::Hidden,
                Some((21, TargetVisibility::Hidden)),
                None,
            ),
            21
        );
        assert_eq!(
            super::canonical_request_id(
                23,
                TargetVisibility::Hidden,
                None,
                Some((22, TargetVisibility::Hidden)),
            ),
            22
        );
        assert_eq!(
            super::canonical_request_id(
                23,
                TargetVisibility::Hidden,
                Some((21, TargetVisibility::Visible)),
                Some((22, TargetVisibility::Visible)),
            ),
            23
        );
    }
}
