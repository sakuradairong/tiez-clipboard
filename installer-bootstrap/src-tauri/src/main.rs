#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod embed;
mod host;

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tiez_installer_core::{
    build_silent_install, classify_exit, disk_space_check, install_dirs_equivalent,
    required_free_bytes, validate_embedded_setup, ArgError, DiskCheck, ExitKind,
};

struct ShellState {
    setup_path: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct InstallerContext {
    product_version: String,
    default_install_dir: String,
    default_available: bool,
    setup_embedded: bool,
    windows_host: bool,
    existing_version: Option<String>,
    existing_location: Option<String>,
    previous_location_differs: bool,
}

#[derive(Serialize)]
struct DirCheck {
    ok: bool,
    normalized: String,
    passes_custom_dir: bool,
    raw_tail: String,
    disk: String,
    free_bytes: Option<u64>,
    need_bytes: u64,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
struct InstallProgress {
    stage: String,
}

#[derive(Serialize)]
struct InstallOutcome {
    exit_code: i32,
    kind: String,
    setup_path: Option<String>,
    raw_tail: String,
    detail: Option<String>,
}

fn main() {
    tauri::Builder::default()
        .manage(ShellState {
            setup_path: Mutex::new(None),
        })
        .setup(|app| {
            if !host::webview2_present() {
                host::show_webview2_required_dialog();
                app.handle().exit(1);
                return Ok(());
            }
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("TieZ")
                .inner_size(480.0, 680.0)
                .resizable(false)
                .maximizable(false)
                .center()
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            installer_context,
            check_install_dir,
            browse_install_dir,
            install,
            launch_installed,
            reveal_setup,
            close_wizard
        ])
        .run(tauri::generate_context!())
        .expect("error while running the TieZ install wizard");
}

#[tauri::command]
fn installer_context() -> InstallerContext {
    let default_install_dir =
        host::default_install_dir().unwrap_or_else(|| r"%LOCALAPPDATA%\TieZ".to_string());
    let existing = host::read_existing_install();
    let previous_location_differs = existing
        .install_location
        .as_deref()
        .is_some_and(|location| !install_dirs_equivalent(location, &default_install_dir));
    InstallerContext {
        product_version: env!("CARGO_PKG_VERSION").to_string(),
        default_install_dir,
        default_available: host::default_install_dir().is_some(),
        setup_embedded: embedded_setup().is_some(),
        windows_host: cfg!(windows),
        existing_version: existing.display_version,
        existing_location: existing.install_location,
        previous_location_differs,
    }
}

#[tauri::command]
fn check_install_dir(install_dir: String) -> DirCheck {
    let default_dir = host::default_install_dir().unwrap_or_default();
    let setup_len = embedded_setup()
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    let need_bytes = required_free_bytes(setup_len);
    match build_silent_install(&install_dir, &default_dir) {
        Ok(plan) => {
            let free_bytes = host::free_bytes_for(&plan.normalized_dir);
            let disk = match disk_space_check(free_bytes, setup_len) {
                DiskCheck::Ok => "ok",
                DiskCheck::Low { .. } => "low",
                DiskCheck::Unknown => "unknown",
            };
            DirCheck {
                ok: true,
                normalized: plan.normalized_dir,
                passes_custom_dir: plan.passes_custom_dir,
                raw_tail: plan.raw_tail,
                disk: disk.to_string(),
                free_bytes,
                need_bytes,
                error: None,
            }
        }
        Err(error) => DirCheck {
            ok: false,
            normalized: install_dir,
            passes_custom_dir: false,
            raw_tail: String::new(),
            disk: "unknown".into(),
            free_bytes: None,
            need_bytes,
            error: Some(arg_error_code(error).to_string()),
        },
    }
}

#[tauri::command]
async fn browse_install_dir(title: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || host::pick_folder(&title))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
async fn install(app: AppHandle, install_dir: String) -> Result<InstallOutcome, String> {
    let default_dir = host::default_install_dir().unwrap_or_default();
    let plan = build_silent_install(&install_dir, &default_dir)
        .map_err(|error| arg_error_code(error).to_string())?;
    if !cfg!(windows) {
        return Ok(failed_outcome("windows_only", plan.raw_tail, None, None));
    }
    let Some(bytes) = embedded_setup() else {
        return Ok(failed_outcome(
            "setup_not_embedded",
            plan.raw_tail,
            None,
            None,
        ));
    };

    let raw_tail = plan.raw_tail.clone();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let app_for_task = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        emit_stage(&app_for_task, "extract");
        let path = match host::extract_setup(bytes, &version) {
            Ok(path) => path,
            Err(error) => {
                return failed_outcome(
                    "failed",
                    raw_tail,
                    None,
                    Some(format!("extract_failed:{error}")),
                );
            }
        };
        emit_stage(&app_for_task, "install");
        let code = match host::run_setup(&path, &raw_tail) {
            Ok(code) => code,
            Err(error) => {
                return failed_outcome(
                    "failed",
                    raw_tail,
                    Some(path.display().to_string()),
                    Some(error),
                );
            }
        };
        let kind = match classify_exit(code) {
            ExitKind::Success => "success",
            ExitKind::UserCancelled => "user_cancelled",
            ExitKind::ScriptAbort => "script_abort",
            ExitKind::Other(_) => "failed",
        };
        if kind == "success" {
            let _ = std::fs::remove_file(&path);
            InstallOutcome {
                exit_code: code,
                kind: kind.into(),
                setup_path: None,
                raw_tail,
                detail: None,
            }
        } else {
            InstallOutcome {
                exit_code: code,
                kind: kind.into(),
                setup_path: Some(path.display().to_string()),
                raw_tail,
                detail: None,
            }
        }
    })
    .await
    .map_err(|error| format!("join_failed:{error}"))?;

    if let Some(shell) = app.try_state::<ShellState>() {
        if let Ok(mut slot) = shell.setup_path.lock() {
            *slot = outcome.setup_path.clone();
        }
    }
    Ok(outcome)
}

#[tauri::command]
fn launch_installed() -> Result<(), String> {
    host::launch_installed()
}

#[tauri::command]
fn reveal_setup(state: State<'_, ShellState>) -> Result<(), String> {
    let path = state
        .setup_path
        .lock()
        .map_err(|_| "lock_failed".to_string())?
        .clone()
        .ok_or_else(|| "missing_setup_path".to_string())?;
    host::reveal_path(std::path::Path::new(&path))
}

#[tauri::command]
fn close_wizard(app: AppHandle) {
    app.exit(0);
}

fn failed_outcome(
    kind: &str,
    raw_tail: String,
    setup_path: Option<String>,
    detail: Option<String>,
) -> InstallOutcome {
    InstallOutcome {
        exit_code: -1,
        kind: kind.to_string(),
        setup_path,
        raw_tail,
        detail,
    }
}

fn embedded_setup() -> Option<&'static [u8]> {
    let bytes = embed::embedded_setup_bytes()?;
    validate_embedded_setup(bytes).ok()?;
    Some(bytes)
}

fn emit_stage(app: &AppHandle, stage: &str) {
    let _ = app.emit(
        "install-progress",
        InstallProgress {
            stage: stage.to_string(),
        },
    );
}

fn arg_error_code(error: ArgError) -> &'static str {
    match error {
        ArgError::EmptyPath => "empty_path",
        ArgError::NotAbsolute => "not_absolute",
        ArgError::QuotesOrControl => "quotes_or_control",
        ArgError::TooLong => "too_long",
        ArgError::ForbiddenFlag => "forbidden_flag",
    }
}
