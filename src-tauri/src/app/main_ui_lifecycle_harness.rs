use crate::app::main_ui_lifecycle::{self, LifecycleMode, MainUiLifecycle};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub const HARNESS_DIR_ENV: &str = "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR";
const POLL_INTERVAL: Duration = Duration::from_millis(100);

static HARNESS_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq)]
struct HarnessConfig {
    dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestFile {
    request_path: PathBuf,
    response_path: PathBuf,
    filename_id: Option<String>,
}

#[derive(Clone, Debug)]
struct HarnessRequest {
    id: Value,
    command: String,
    payload: Value,
}

#[derive(Debug, Serialize)]
struct HarnessResponse {
    id: Value,
    success: bool,
    payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl HarnessResponse {
    fn ok(id: Value, payload: Value) -> Self {
        Self {
            id,
            success: true,
            payload,
            error: None,
        }
    }

    fn err(id: Value, error: impl Into<String>) -> Self {
        Self {
            id,
            success: false,
            payload: Value::Null,
            error: Some(error.into()),
        }
    }
}

/// Starts the internal filesystem lifecycle harness when both experimental
/// lifecycle mode and the private harness directory environment gate are set.
///
/// This transport intentionally never creates the configured directory and does
/// not listen on a socket. The cloned AppHandle keeps the Rust worker alive when
/// the main UI webview is hidden or destroyed.
pub fn start(app: &tauri::App) {
    let app_handle = app.handle().clone();
    let Some(config) = config_from_environment(&app_handle) else {
        return;
    };

    if HARNESS_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    crate::info!(
        ">>> [UI_LIFECYCLE_HARNESS] filesystem transport enabled at {}",
        config.dir.display()
    );

    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = process_once(&app_handle, &config.dir) {
                crate::error!(
                    ">>> [UI_LIFECYCLE_HARNESS] poll failed for {}: {}",
                    config.dir.display(),
                    error
                );
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

fn config_from_environment(app: &AppHandle) -> Option<HarnessConfig> {
    let Some(lifecycle) = app.try_state::<MainUiLifecycle>() else {
        return None;
    };

    match harness_dir_from_value(
        lifecycle.mode(),
        std::env::var(HARNESS_DIR_ENV).ok().as_deref(),
        |path| {
            path.is_dir()
                && fs::read_dir(path)
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(false)
        },
    ) {
        Ok(Some(dir)) => Some(HarnessConfig { dir }),
        Ok(None) => None,
        Err(error) => {
            crate::error!(
                ">>> [UI_LIFECYCLE_HARNESS] disabled: {} ({HARNESS_DIR_ENV})",
                error
            );
            None
        }
    }
}

fn harness_dir_from_value(
    mode: LifecycleMode,
    value: Option<&str>,
    is_existing_dir: impl Fn(&Path) -> bool,
) -> Result<Option<PathBuf>, String> {
    if !mode.is_experimental() {
        return Ok(None);
    }

    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let dir = PathBuf::from(value);
    if !dir.is_absolute() {
        return Err("harness directory must be an absolute path".to_string());
    }
    if !is_existing_dir(&dir) {
        return Err("harness directory must already exist and be empty".to_string());
    }

    Ok(Some(dir))
}

fn process_once(app: &AppHandle, dir: &Path) -> io::Result<()> {
    for request_file in discover_requests(dir)? {
        process_request_file(app, &request_file)?;
    }
    Ok(())
}

fn discover_requests(dir: &Path) -> io::Result<Vec<RequestFile>> {
    let mut requests = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };

        let Some(id) = file_name.strip_suffix(".request.json") else {
            continue;
        };
        if id.is_empty() || !is_safe_file_id(id) {
            continue;
        }

        requests.push(RequestFile {
            request_path: path,
            response_path: dir.join(format!("{id}.response.json")),
            filename_id: Some(id.to_string()),
        });
    }
    requests.sort_by(|left, right| left.request_path.cmp(&right.request_path));
    Ok(requests)
}

fn process_request_file(app: &AppHandle, request_file: &RequestFile) -> io::Result<()> {
    let content = fs::read_to_string(&request_file.request_path)?;
    let value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            crate::error!(
                ">>> [UI_LIFECYCLE_HARNESS] ignoring unreadable request {}: {}",
                request_file.request_path.display(),
                error
            );
            return Ok(());
        }
    };

    let response = match parse_request(value, request_file.filename_id.as_deref()) {
        Ok(request) => handle_request(app, request),
        Err((id, error)) => HarnessResponse::err(id, error),
    };

    remove_stale_response(&request_file.response_path)?;
    write_response_atomic(&request_file.response_path, &response)?;
    if let Err(error) = fs::remove_file(&request_file.request_path) {
        crate::error!(
            ">>> [UI_LIFECYCLE_HARNESS] failed to remove request {}: {}",
            request_file.request_path.display(),
            error
        );
    }
    Ok(())
}

fn parse_request(
    value: Value,
    filename_id: Option<&str>,
) -> Result<HarnessRequest, (Value, String)> {
    let Some(object) = value.as_object() else {
        return Err((
            filename_id.map_or(Value::Null, |id| json!(id)),
            "request must be a JSON object".to_string(),
        ));
    };

    let id = object.get("id").cloned().unwrap_or(Value::Null);

    if id.is_null() {
        return Err((id, "request id is required".to_string()));
    }

    if let Some(filename_id) = filename_id {
        match id.as_str() {
            Some(id) if id == filename_id => {}
            Some(_) => {
                return Err((
                    id,
                    format!("request id must match filename id '{filename_id}'"),
                ));
            }
            None => {
                return Err((
                    id,
                    "filename-scoped requests require a string id".to_string(),
                ));
            }
        }
    }

    let Some(command) = object.get("command").and_then(Value::as_str) else {
        return Err((id, "request command is required".to_string()));
    };

    if !is_allowed_command(command) {
        return Err((id, format!("command '{command}' is not allowed")));
    }

    Ok(HarnessRequest {
        id,
        command: command.to_string(),
        payload: object.get("payload").cloned().unwrap_or(Value::Null),
    })
}

fn is_allowed_command(command: &str) -> bool {
    matches!(
        command,
        "get_main_ui_lifecycle_snapshot"
            | "get_main_ui_lifecycle_traces"
            | "get_main_ui_lifecycle_clipboard_probe"
            | "lifecycle_test_toggle"
            | "lifecycle_test_hide"
            | "lifecycle_test_show"
    )
}

fn handle_request(app: &AppHandle, request: HarnessRequest) -> HarnessResponse {
    let id = request.id;
    let command = request.command;
    let payload = request.payload;
    let result = match command.as_str() {
        "get_main_ui_lifecycle_snapshot" => app
            .try_state::<MainUiLifecycle>()
            .map(|lifecycle| serde_json::to_value(lifecycle.snapshot(app)))
            .ok_or_else(|| "main UI lifecycle state is not initialized".to_string())
            .and_then(|value| value.map_err(|error| error.to_string())),
        "get_main_ui_lifecycle_traces" => app
            .try_state::<MainUiLifecycle>()
            .map(|lifecycle| serde_json::to_value(lifecycle.traces()))
            .ok_or_else(|| "main UI lifecycle state is not initialized".to_string())
            .and_then(|value| value.map_err(|error| error.to_string())),
        "get_main_ui_lifecycle_clipboard_probe" => payload
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| "clipboard probe payload.token is required".to_string())
            .and_then(|token| {
                payload
                    .get("clipboard_event_count_before")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        "clipboard probe payload.clipboard_event_count_before is required"
                            .to_string()
                    })
                    .map(|before| (token, before))
            })
            .and_then(|(token, before)| {
                main_ui_lifecycle::get_main_ui_lifecycle_clipboard_probe(app, token, before)
            })
            .and_then(|probe| serde_json::to_value(probe).map_err(|error| error.to_string())),
        "lifecycle_test_toggle" => main_ui_lifecycle::lifecycle_test_toggle(app.clone())
            .map(|_| json!({ "accepted": true })),
        "lifecycle_test_hide" => {
            main_ui_lifecycle::request_test_hide(app).map(|(request_id, generation_before)| {
                json!({
                    "accepted": true,
                    "request_id": request_id,
                    "generation_before": generation_before,
                })
            })
        }
        "lifecycle_test_show" => main_ui_lifecycle::request_test_show(app).map(
            |(request_id, generation_before, expected_generation)| {
                json!({
                    "accepted": true,
                    "request_id": request_id,
                    "generation_before": generation_before,
                    "expected_generation": expected_generation,
                })
            },
        ),
        _ => Err(format!("command '{command}' is not allowed")),
    };

    match result {
        Ok(payload) => HarnessResponse::ok(id, payload),
        Err(error) => HarnessResponse::err(id, error),
    }
}

fn remove_stale_response(response_path: &Path) -> io::Result<()> {
    match fs::remove_file(response_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_response_atomic(response_path: &Path, response: &HarnessResponse) -> io::Result<()> {
    let parent = response_path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "response path has no parent")
    })?;
    let file_name = response_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "response path has no filename")
        })?;
    let tmp_path = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(response).map_err(io::Error::other)?;

    fs::write(&tmp_path, bytes)?;
    match fs::rename(&tmp_path, response_path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(response_path);
            fs::rename(&tmp_path, response_path).map_err(|_| error)
        }
    }
}

fn is_safe_file_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn harness_dir_requires_experimental_absolute_existing_empty_directory() {
        let is_existing = |path: &Path| path == Path::new("/existing");

        assert_eq!(
            harness_dir_from_value(LifecycleMode::Default, Some("/existing"), is_existing).unwrap(),
            None
        );
        assert_eq!(
            harness_dir_from_value(LifecycleMode::Hidden, None, is_existing).unwrap(),
            None
        );
        assert!(
            harness_dir_from_value(LifecycleMode::Hidden, Some("relative"), is_existing)
                .unwrap_err()
                .contains("absolute")
        );
        assert!(
            harness_dir_from_value(LifecycleMode::Destroyed, Some("/missing"), is_existing)
                .unwrap_err()
                .contains("already exist")
        );
        assert_eq!(
            harness_dir_from_value(LifecycleMode::Destroyed, Some(" /existing "), is_existing)
                .unwrap(),
            Some(PathBuf::from("/existing"))
        );

        let is_non_empty = |_path: &Path| false;
        assert!(
            harness_dir_from_value(LifecycleMode::Destroyed, Some("/existing"), is_non_empty)
                .unwrap_err()
                .contains("empty")
        );
    }

    #[test]
    fn command_allowlist_rejects_arbitrary_commands() {
        assert!(is_allowed_command("get_main_ui_lifecycle_snapshot"));
        assert!(is_allowed_command("get_main_ui_lifecycle_traces"));
        assert!(is_allowed_command("get_main_ui_lifecycle_clipboard_probe"));
        assert!(is_allowed_command("lifecycle_test_toggle"));
        assert!(is_allowed_command("lifecycle_test_hide"));
        assert!(is_allowed_command("lifecycle_test_show"));
        assert!(!is_allowed_command("shell"));
        assert!(!is_allowed_command("invoke"));
        assert!(!is_allowed_command("get_clipboard_history"));
    }

    #[test]
    fn request_file_ids_are_bounded_and_path_neutral() {
        assert!(is_safe_file_id("req_123-safe"));
        assert!(!is_safe_file_id(""));
        assert!(!is_safe_file_id("../escape"));
        assert!(!is_safe_file_id(&"x".repeat(129)));
    }

    #[test]
    fn filename_scoped_requests_must_match_id_and_allowlist() {
        let request = parse_request(
            json!({
                "id": "abc_123",
                "command": "lifecycle_test_hide",
                "payload": { "ignored": true }
            }),
            Some("abc_123"),
        )
        .unwrap();
        assert_eq!(request.id, json!("abc_123"));
        assert_eq!(request.command, "lifecycle_test_hide");
        assert_eq!(request.payload, json!({ "ignored": true }));

        let (_, mismatch) = parse_request(
            json!({ "id": "other", "command": "lifecycle_test_hide" }),
            Some("abc_123"),
        )
        .unwrap_err();
        assert!(mismatch.contains("must match filename"));

        let (_, missing_id) =
            parse_request(json!({ "command": "lifecycle_test_hide" }), Some("abc_123"))
                .unwrap_err();
        assert!(missing_id.contains("id is required"));

        let (_, arbitrary) = parse_request(
            json!({ "id": "abc_123", "command": "delete_everything" }),
            Some("abc_123"),
        )
        .unwrap_err();
        assert!(arbitrary.contains("not allowed"));
    }

    #[test]
    fn discovers_single_and_id_scoped_requests_only() {
        let dir = unique_temp_dir("discover");
        fs::write(dir.join("request.json"), "{}").unwrap();
        fs::write(dir.join("abc-123.request.json"), "{}").unwrap();
        fs::write(dir.join("bad.id.request.json"), "{}").unwrap();
        fs::write(dir.join("abc.response.json"), "{}").unwrap();
        fs::write(dir.join("abc.request.json.tmp"), "{}").unwrap();

        let requests = discover_requests(&dir).unwrap();
        let request_names = requests
            .iter()
            .map(|request| {
                request
                    .request_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(request_names, vec!["abc-123.request.json"]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn response_write_removes_stale_file_and_uses_tmp_rename() {
        let dir = unique_temp_dir("response");
        let response_path = dir.join("abc.response.json");
        fs::write(
            &response_path,
            r#"{"id":"stale","success":true,"payload":{}}"#,
        )
        .unwrap();
        remove_stale_response(&response_path).unwrap();

        let response = HarnessResponse::ok(json!("abc"), json!({ "accepted": true }));
        write_response_atomic(&response_path, &response).unwrap();

        let written: Value =
            serde_json::from_str(&fs::read_to_string(&response_path).unwrap()).unwrap();
        assert_eq!(written["id"], json!("abc"));
        assert_eq!(written["success"], json!(true));
        assert_eq!(written["payload"], json!({ "accepted": true }));
        assert!(!fs::read_dir(&dir).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));

        fs::remove_dir_all(dir).unwrap();
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tiez-lifecycle-harness-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }
}
