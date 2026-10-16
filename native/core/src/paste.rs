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
) -> CoreResult<PasteResult> {
    let ticket = service
        .target
        .ticket(expected_generation)
        .map_err(before_write)?;
    // Prepare every format before changing focus or the OS clipboard.
    let prepared = service
        .capture
        .prepare_copy(id, plain_text, allow_sensitive)?;
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
    Ok(PasteResult {
        id: copied.id,
        clipboard_sequence: copied.clipboard_sequence,
        target_generation: expected_generation,
        input_events: 4,
    })
}
