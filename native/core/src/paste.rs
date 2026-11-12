//! Clipboard and input ordering for one explicit history paste request.
use crate::{CoreError, CoreResult, CoreRuntime};
use serde::Serialize;
use tiez_native_platform::keyboard;

#[derive(Debug, Serialize)]
pub(crate) struct PasteResult {
    id: i64,
    clipboard_sequence: u32,
    target_generation: u64,
    input_events: u32,
    post_paste_error: Option<String>,
}
fn before_write(code: &'static str) -> CoreError {
    CoreError::new(
        code,
        "Paste target or keys could not be verified; clipboard unchanged and no input sent",
    )
}
pub(crate) fn paste_entry(
    service: &CoreRuntime,
    id: i64,
    plain_text: bool,
    allow_sensitive: bool,
    expected_generation: u64,
    expected_revision: Option<u64>,
) -> CoreResult<PasteResult> {
    let ticket = service
        .target
        .ticket(expected_generation)
        .map_err(before_write)?;
    // Prepare every format before changing focus or the OS clipboard.
    let prepared = match expected_revision {
        Some(revision) => service.capture.prepare_copy_at_revision(id, plain_text, allow_sensitive, Some(revision))?,
        None => service.capture.prepare_copy(id, plain_text, allow_sensitive)?,
    };
    keyboard::ensure_keys_idle().map_err(before_write)?;
    service.target.restore(&ticket).map_err(before_write)?;
    let copied = service.capture.commit_copy(prepared, || {
        keyboard::ensure_keys_idle().map_err(before_write)?;
        service
            .target
            .validate_foreground(&ticket)
            .map_err(before_write)
    })?;
    keyboard::send_paste(copied.clipboard_sequence, || service.target.validate_foreground(&ticket))
        .map_err(|error| CoreError::new(error.code, format!(
            "Clipboard was written; paste input accepted {}/4 events, cleanup accepted {} events, cleanup complete: {}. History retained",
            error.sent, error.cleanup_sent, error.cleanup_complete)))?;
    // Sending a complete batch does not prove the target consumed its clipboard.
    let post_paste_error = apply_after_successful_paste(service, copied.id);
    Ok(PasteResult {
        id: copied.id,
        clipboard_sequence: copied.clipboard_sequence,
        target_generation: expected_generation,
        input_events: 4,
        post_paste_error,
    })
}
fn apply_after_successful_paste(service: &CoreRuntime, id: i64) -> Option<String> {
    let snapshot = match service.settings.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return Some(error.message),
    };
    let delete_after = snapshot.values["app.delete_after_paste"] == "true";
    let move_to_top = snapshot.values["app.move_to_top_after_paste"] != "false";
    let mut history = match service.history.lock() {
        Ok(history) => history,
        Err(_) => return Some("History unavailable".to_string()),
    };
    // The paste already reached the OS. A bookkeeping failure must not be reported as an unsent paste.
    history
        .apply_after_paste(id, delete_after, move_to_top)
        .err()
        .map(|error| error.message)
}
pub(crate) fn quick_paste(service: &CoreRuntime, index: u8) -> CoreResult<serde_json::Value> {
    let snapshot = service.settings.snapshot()?;
    if snapshot.values["app.quick_paste_modifier"] == "disabled" {
        return Ok(serde_json::json!({"pasted": false, "reason": "disabled"}));
    }
    let edge_hidden = service
        .edge_hidden
        .load(std::sync::atomic::Ordering::Relaxed);
    if edge_hidden && snapshot.values["app.quick_paste_when_edge_hidden"] != "true" {
        return Ok(serde_json::json!({"pasted": false, "reason": "edge_hidden"}));
    }
    let (id, plain_text, revision) = {
        let history = service
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?;
        let Some((id, kind)) = history.quick_paste_target(usize::from(index))? else {
            return Ok(serde_json::json!({"pasted": false, "reason": "not_pinned"}));
        };
        (
            id,
            matches!(kind.as_str(), "text" | "code" | "url" | "rich_text"),
            history.revision(),
        )
    };
    let status = service.target.status();
    if !status.running || !status.has_target {
        return Err(before_write("no_target"));
    }
    // The hotkey itself is the explicit confirmation, matching the legacy global shortcut.
    let pasted = paste_entry(service, id, plain_text, true, status.generation, Some(revision))?;
    Ok(serde_json::json!({
        "pasted": true,
        "reason": null,
        "id": pasted.id,
        "clipboard_sequence": pasted.clipboard_sequence,
        "target_generation": pasted.target_generation,
        "input_events": pasted.input_events,
        "post_paste_error": pasted.post_paste_error
    }))
}
pub(crate) fn paste_latest(service: &CoreRuntime, text_only: bool) -> CoreResult<serde_json::Value> {
    let (id, revision) = {
        let history = service
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?;
        let Some((id, _)) = history.latest_paste_target(text_only)? else {
            return Ok(serde_json::json!({"pasted": false, "reason": "empty"}));
        };
        (id, history.revision())
    };
    let status = service.target.status();
    if !status.running || !status.has_target {
        return Err(before_write("no_target"));
    }
    let pasted = paste_entry(service, id, text_only, true, status.generation, Some(revision))?;
    Ok(serde_json::json!({
        "pasted": true,
        "reason": null,
        "id": pasted.id,
        "post_paste_error": pasted.post_paste_error
    }))
}
