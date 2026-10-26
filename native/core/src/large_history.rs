//! Bounded, runtime-owned UTF-8 staging for authoritative history mutations only.
use crate::history::{HistoryMutation, HistoryService};
use crate::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use zeroize::Zeroize;

pub(crate) const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_RESERVED: usize = 2 * MAX_PAYLOAD;
const MAX_UPLOADS: usize = 2;
pub(crate) const MAX_CHUNK: usize = 4096;
const IDLE: Duration = Duration::from_secs(120);

fn error(code: &'static str, message: &'static str) -> CoreError { CoreError::new(code, message) }
fn missing() -> CoreError { error("transfer_missing", "Staged resource is missing or expired") }

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Target {
    Add {},
    Edit { id: i64, #[serde(default)] allow_sensitive: bool },
}
#[derive(Serialize)]
pub(crate) struct Limits {
    version: u32,
    history_revision: u64,
    max_payload_bytes: usize,
    max_reserved_bytes: usize,
    max_chunk_bytes: usize,
    max_uploads: usize,
    idle_seconds: u64,
}
pub(crate) fn limits(history_revision: u64) -> Limits {
    Limits { version: 1, history_revision, max_payload_bytes: MAX_PAYLOAD, max_reserved_bytes: MAX_RESERVED,
        max_chunk_bytes: MAX_CHUNK, max_uploads: MAX_UPLOADS, idle_seconds: IDLE.as_secs() }
}
#[derive(Debug, Serialize)]
pub(crate) struct Status {
    pub token: String,
    pub total_bytes: usize,
    pub received_bytes: usize,
    pub expected_revision: u64,
}
struct Upload {
    target: Target,
    expected_revision: u64,
    total: usize,
    digest: [u8; 32],
    bytes: Vec<u8>,
    touched: Instant,
}
impl Drop for Upload {
    fn drop(&mut self) { self.bytes.zeroize(); }
}
impl Upload {
    fn status(&self, token: &str) -> Status {
        Status { token: token.into(), total_bytes: self.total, received_bytes: self.bytes.len(), expected_revision: self.expected_revision }
    }
}
#[derive(Default)]
pub(crate) struct Uploads {
    items: BTreeMap<String, Upload>,
    reserved: usize,
}
fn token_valid(token: &str) -> CoreResult<()> {
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error("transfer_token", "Invalid staged resource token"));
    }
    Ok(())
}
fn checksum(value: &str) -> CoreResult<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error("transfer_digest", "SHA-256 must contain 64 hexadecimal characters"));
    }
    Ok(std::array::from_fn(|index| u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap()))
}
fn current(history: &HistoryService, target: &Target, expected: u64) -> CoreResult<()> {
    if history.revision() != expected {
        return Err(error("history_conflict", "History changed while content was staged"));
    }
    if let Target::Edit { id, allow_sensitive } = target {
        let metadata = history.tag_selection(*id)?;
        if tiez_storage::database::has_sensitive_tag(&metadata.tags) && !allow_sensitive {
            return Err(error("sensitive_confirmation", "Explicit sensitive-content authorization is required"));
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddPayload {
    content_type: String,
    content: String,
    html_content: Option<String>,
    #[serde(default)] tags: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditPayload { content: String }
impl Uploads {
    pub(crate) fn reap(&mut self) { self.reap_at(Instant::now()); }
    fn reap_at(&mut self, now: Instant) {
        let expired: Vec<_> = self.items.iter().filter(|(_, item)| now.saturating_duration_since(item.touched) >= IDLE)
            .map(|(token, _)| token.clone()).collect();
        for token in expired { self.remove(&token); }
    }
    fn remove(&mut self, token: &str) -> Option<Upload> {
        let item = self.items.remove(token)?;
        self.reserved -= item.total;
        Some(item)
    }
    pub(crate) fn clear(&mut self) { self.items.clear(); self.reserved = 0; }
    pub(crate) fn begin(&mut self, version: u32, target: Target, expected_revision: u64,
        total_bytes: usize, sha256: &str, history: &HistoryService) -> CoreResult<Status>
    {
        self.reap();
        if version != 1 { return Err(error("transfer_version", "Unsupported history transfer version")); }
        if total_bytes == 0 || total_bytes > MAX_PAYLOAD {
            return Err(error("transfer_bounds", "Staged history payload exceeds its declared bound"));
        }
        let digest = checksum(sha256)?;
        current(history, &target, expected_revision)?;
        if self.items.len() >= MAX_UPLOADS || total_bytes > MAX_RESERVED - self.reserved {
            return Err(error("transfer_capacity", "Too many staged history resources are retained"));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(total_bytes).map_err(|_| error("transfer_capacity", "Staging allocation is unavailable"))?;
        let token = loop {
            let token = uuid::Uuid::new_v4().simple().to_string();
            if !self.items.contains_key(&token) { break token; }
        };
        let item = Upload { target, expected_revision, total: total_bytes, digest, bytes, touched: Instant::now() };
        let status = item.status(&token);
        self.reserved += total_bytes;
        self.items.insert(token, item);
        Ok(status)
    }
    pub(crate) fn status(&mut self, token: &str) -> CoreResult<Status> {
        self.reap(); token_valid(token)?;
        self.items.get(token).map(|item| item.status(token)).ok_or_else(missing)
    }
    pub(crate) fn append(&mut self, token: &str, offset: usize, data: &str) -> CoreResult<Status> {
        self.reap(); token_valid(token)?;
        let item = self.items.get_mut(token).ok_or_else(missing)?;
        if data.is_empty() || data.len() > MAX_CHUNK {
            return Err(error("transfer_chunk", "Transfer chunks must contain 1..4096 UTF-8 bytes"));
        }
        let end = offset.checked_add(data.len()).filter(|end| *end <= item.total)
            .ok_or_else(|| error("transfer_offset", "Transfer offset exceeds the declared resource"))?;
        if offset == item.bytes.len() { item.bytes.extend_from_slice(data.as_bytes()); }
        else if end > item.bytes.len() || item.bytes[offset..end] != *data.as_bytes() {
            return Err(error("transfer_offset", "Transfer chunks must be contiguous or exact replays"));
        }
        item.touched = Instant::now();
        Ok(item.status(token))
    }
    pub(crate) fn abort(&mut self, token: &str) -> CoreResult<bool> {
        self.reap(); token_valid(token)?;
        Ok(self.remove(token).is_some())
    }
    pub(crate) fn commit(&mut self, token: &str, history: &mut HistoryService) -> CoreResult<HistoryMutation> {
        self.reap(); token_valid(token)?;
        let item = self.items.get(token).ok_or_else(missing)?;
        if item.bytes.len() != item.total {
            return Err(error("transfer_incomplete", "Staged history content is incomplete"));
        }
        // A complete commit is one-shot, including malformed or rejected data. Drafts stay with
        // the caller; never retry an accepted mutation or retain plaintext after a failed commit.
        let item = self.remove(token).unwrap();
        if Sha256::digest(&item.bytes).as_slice() != item.digest {
            return Err(error("transfer_digest", "Staged history checksum does not match"));
        }
        current(history, &item.target, item.expected_revision)?;
        let payload_error = |_| error("transfer_payload", "Staged payload does not match its history operation");
        match item.target {
            Target::Add {} => {
                let payload: AddPayload = serde_json::from_slice(&item.bytes).map_err(payload_error)?;
                history.add_manual(payload.content_type, payload.content, payload.html_content, payload.tags)
            }
            Target::Edit { id, .. } => {
                let payload: EditPayload = serde_json::from_slice(&item.bytes).map_err(payload_error)?;
                history.edit(id, payload.content)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!("TieZ-history-upload-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&path).unwrap(); path
    }
    fn cleanup(path: &std::path::Path) {
        use std::os::windows::fs::MetadataExt;
        let name = path.file_name().unwrap().to_string_lossy();
        assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        assert!(name.strip_prefix("TieZ-history-upload-").is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()));
        assert_eq!(std::fs::symlink_metadata(path).unwrap().file_attributes() & 0x400, 0);
        std::fs::remove_dir_all(path).unwrap();
    }
    struct Fixture { path: PathBuf, history: Option<HistoryService> }
    impl Fixture {
        fn new() -> Self {
            let path = directory(); let history = Some(HistoryService::open(&path.join("history.db")).unwrap());
            Self { path, history }
        }
        fn h(&mut self) -> &mut HistoryService { self.history.as_mut().unwrap() }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { self.history.take(); cleanup(&self.path); }
    }
    fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
    fn begin(uploads: &mut Uploads, fixture: &mut Fixture, target: Target, body: &[u8]) -> Status {
        let revision = fixture.h().revision();
        uploads.begin(1, target, revision, body.len(), &hash(body), fixture.h()).unwrap()
    }
    fn feed(uploads: &mut Uploads, token: &str, bytes: &[u8]) {
        let text = std::str::from_utf8(bytes).unwrap(); let mut offset = 0;
        while offset < text.len() {
            let mut end = (offset + MAX_CHUNK).min(text.len());
            while !text.is_char_boundary(end) { end -= 1; }
            let status = uploads.append(token, offset, &text[offset..end]).unwrap();
            assert_eq!(status.received_bytes, end); offset = end;
        }
    }
    #[test]
    fn large_rich_add_is_atomic_preserves_original_bytes_and_is_one_shot() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default();
        let content = format!("  {}{}中文😀{}{} ", char::from(13), char::from(10), "正文😀".repeat(30000), char::from(9));
        let html = format!("<table><tr><td>{}</td></tr></table>{}{}", "格😀".repeat(20000), char::from(13), char::from(10));
        let body = serde_json::to_vec(&json!({"content_type":"rich_text","content":content,"html_content":html,"tags":[]})).unwrap();
        assert!(body.len() > crate::MAX_REQUEST_BYTES);
        let upload = begin(&mut uploads, &mut fixture, Target::Add {}, &body);
        feed(&mut uploads, &upload.token, &body);
        assert_eq!(fixture.h().revision(), 0);
        assert_eq!(fixture.h().page(Default::default()).unwrap().stored_count, 0);
        let result = uploads.commit(&upload.token, fixture.h()).unwrap();
        assert_eq!(result.revision, 1); assert!(result.id > 0 && result.changed);
        let entry = fixture.h().entry(result.id).unwrap();
        assert_eq!(entry.content, content); assert_eq!(entry.html_content.as_deref(), Some(html.as_str()));
        assert_eq!(uploads.commit(&upload.token, fixture.h()).unwrap_err().code, "transfer_missing");
        assert_eq!(uploads.reserved, 0); assert!(uploads.items.is_empty());
    }
    #[test]
    fn append_offsets_bounds_and_exact_replays_do_not_corrupt_resources() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default();
        let upload = begin(&mut uploads, &mut fixture, Target::Add {}, b"abcdefghijklmnop");
        assert_eq!(uploads.append(&upload.token, 1, "bad").unwrap_err().code, "transfer_offset");
        assert_eq!(uploads.append(&upload.token, usize::MAX, "x").unwrap_err().code, "transfer_offset");
        assert_eq!(uploads.append(&upload.token, 0, "").unwrap_err().code, "transfer_chunk");
        assert_eq!(uploads.append(&upload.token, 0, &"x".repeat(MAX_CHUNK + 1)).unwrap_err().code, "transfer_chunk");
        assert_eq!(uploads.append(&upload.token, 0, "abcd").unwrap().received_bytes, 4);
        assert_eq!(uploads.append(&upload.token, 0, "abcd").unwrap().received_bytes, 4);
        assert_eq!(uploads.append(&upload.token, 1, "bc").unwrap().received_bytes, 4);
        assert_eq!(uploads.append(&upload.token, 1, "XX").unwrap_err().code, "transfer_offset");
        assert_eq!(uploads.status(&upload.token).unwrap().received_bytes, 4);
        assert_eq!(uploads.commit(&upload.token, fixture.h()).unwrap_err().code, "transfer_incomplete");
        assert!(uploads.abort(&upload.token).unwrap()); assert!(!uploads.abort(&upload.token).unwrap());
        assert_eq!(uploads.reserved, 0); assert_eq!(fixture.h().revision(), 0);
    }
    #[test]
    fn reservation_version_digest_and_expiry_have_explicit_bounds() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default(); let digest = hash(b"payload");
        for size in [0, MAX_PAYLOAD + 1] {
            assert_eq!(uploads.begin(1, Target::Add {}, 0, size, &digest, fixture.h()).unwrap_err().code, "transfer_bounds");
        }
        assert_eq!(uploads.begin(2, Target::Add {}, 0, 1, &digest, fixture.h()).unwrap_err().code, "transfer_version");
        for invalid in ["x".repeat(64), "😀".repeat(16), "0".repeat(63)] {
            assert_eq!(uploads.begin(1, Target::Add {}, 0, 1, &invalid, fixture.h()).unwrap_err().code, "transfer_digest");
        }
        let first = uploads.begin(1, Target::Add {}, 0, MAX_PAYLOAD, &digest, fixture.h()).unwrap();
        let second = uploads.begin(1, Target::Add {}, 0, MAX_PAYLOAD, &digest, fixture.h()).unwrap();
        assert_ne!(first.token, second.token); assert_eq!(uploads.reserved, MAX_RESERVED);
        assert_eq!(uploads.begin(1, Target::Add {}, 0, 1, &digest, fixture.h()).unwrap_err().code, "transfer_capacity");
        uploads.abort(&first.token).unwrap(); assert_eq!(uploads.reserved, MAX_PAYLOAD);
        let third = uploads.begin(1, Target::Add {}, 0, 1, &digest, fixture.h()).unwrap();
        assert_ne!(first.token, third.token);
        uploads.reap_at(Instant::now() + IDLE + Duration::from_secs(1));
        assert_eq!(uploads.reserved, 0); assert!(uploads.items.is_empty());
        assert_eq!(uploads.status(&third.token).unwrap_err().code, "transfer_missing");
        assert_eq!(uploads.status("not a token").unwrap_err().code, "transfer_token");
        assert!(MAX_CHUNK * 6 + 512 < crate::MAX_REQUEST_BYTES);
        assert!(serde_json::to_vec(&limits(0)).unwrap().len() < 512);
    }
    #[test]
    fn complete_invalid_payloads_release_memory_without_history_changes() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default();
        for body in [b"not JSON".as_slice(), br#"{"op":"close","handle":1}"#,
            br#"{"content_type":"text","content":"private fixture","path":"not-a-staging-path"}"#] {
            let upload = begin(&mut uploads, &mut fixture, Target::Add {}, body);
            feed(&mut uploads, &upload.token, body);
            let failure = uploads.commit(&upload.token, fixture.h()).unwrap_err();
            assert_eq!(failure.code, "transfer_payload"); assert!(!failure.message.contains("private fixture"));
            assert_eq!(uploads.reserved, 0); assert_eq!(fixture.h().revision(), 0);
        }
        let upload = uploads.begin(1, Target::Add {}, 0, 2, &"0".repeat(64), fixture.h()).unwrap();
        uploads.append(&upload.token, 0, "{}").unwrap();
        assert_eq!(uploads.commit(&upload.token, fixture.h()).unwrap_err().code, "transfer_digest");
        assert_eq!(uploads.reserved, 0); assert_eq!(fixture.h().revision(), 0);
    }
    #[test]
    fn stale_and_sensitive_edits_are_rechecked_at_acceptance() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default();
        let id = fixture.h().add_manual("rich_text".into(), "secret fixture".into(), Some("<b>original</b>".into()), vec!["密码".into()]).unwrap().id;
        let body = serde_json::to_vec(&json!({"content":"changed private fixture"})).unwrap();
        let revision = fixture.h().revision();
        assert_eq!(uploads.begin(1, Target::Edit { id, allow_sensitive: false }, revision, body.len(), &hash(&body), fixture.h()).unwrap_err().code, "sensitive_confirmation");
        assert_eq!(uploads.reserved, 0);
        let upload = begin(&mut uploads, &mut fixture, Target::Edit { id, allow_sensitive: true }, &body);
        feed(&mut uploads, &upload.token, &body);
        fixture.h().set_tags(id, vec!["sensitive".into()]).unwrap();
        let accepted_revision = fixture.h().revision();
        assert_eq!(uploads.commit(&upload.token, fixture.h()).unwrap_err().code, "history_conflict");
        assert_eq!(uploads.reserved, 0); assert_eq!(fixture.h().revision(), accepted_revision);
        assert_eq!(fixture.h().entry(id).unwrap().content, "secret fixture");
        assert_eq!(fixture.h().entry(id).unwrap().html_content.as_deref(), Some("<b>original</b>"));
        let upload = begin(&mut uploads, &mut fixture, Target::Edit { id, allow_sensitive: true }, &body);
        feed(&mut uploads, &upload.token, &body);
        let result = uploads.commit(&upload.token, fixture.h()).unwrap();
        assert_eq!(result.id, id); assert!(result.changed);
        assert_eq!(fixture.h().entry(id).unwrap().content, "changed private fixture");
        assert_eq!(fixture.h().entry(id).unwrap().content_type, "text");
    }
    #[test]
    fn negative_session_edit_preserves_identity_without_promotion() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default();
        let saved = fixture.h().add_manual("rich_text".into(), "old".into(), Some("<b>old</b>".into()), vec![]).unwrap();
        let mut entry = fixture.h().entry(saved.id).unwrap(); entry.id = 0;
        let session = fixture.h().record_processed(entry, false).unwrap(); assert!(session.id < 0);
        let content = format!("  {}{}{}😀  ", char::from(13), char::from(10), "新正文".repeat(40000));
        let body = serde_json::to_vec(&json!({"content":content})).unwrap();
        let upload = begin(&mut uploads, &mut fixture, Target::Edit { id: session.id, allow_sensitive: false }, &body);
        feed(&mut uploads, &upload.token, &body);
        let result = uploads.commit(&upload.token, fixture.h()).unwrap();
        assert_eq!(result.id, session.id); assert_eq!(result.replaced_id, None);
        let edited = fixture.h().entry(session.id).unwrap();
        assert_eq!(edited.content, content); assert_eq!(edited.content_type, "text"); assert_eq!(edited.html_content, None);
        assert_eq!(fixture.h().entry(saved.id).unwrap().content, "old");
    }
    #[test]
    fn failed_database_commit_keeps_history_and_releases_the_complete_resource() {
        let mut fixture = Fixture::new(); let mut uploads = Uploads::default();
        let saved = fixture.h().add_manual("rich_text".into(), "preserved".into(), Some("<b>preserved</b>".into()), vec![]).unwrap();
        let revision = fixture.h().revision();
        let body = serde_json::to_vec(&json!({"content":"changed".repeat(20000)})).unwrap();
        let upload = begin(&mut uploads, &mut fixture, Target::Edit { id: saved.id, allow_sensitive: false }, &body);
        feed(&mut uploads, &upload.token, &body);
        let connection = rusqlite::Connection::open(fixture.path.join("history.db")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_staged_edit BEFORE UPDATE ON clipboard_history
            BEGIN SELECT RAISE(ABORT, 'injected staged write failure'); END;").unwrap();
        assert_eq!(uploads.commit(&upload.token, fixture.h()).unwrap_err().code, "history");
        assert_eq!(uploads.reserved, 0); assert!(uploads.items.is_empty());
        assert_eq!(fixture.h().revision(), revision);
        let entry = fixture.h().entry(saved.id).unwrap();
        assert_eq!(entry.content, "preserved"); assert_eq!(entry.html_content.as_deref(), Some("<b>preserved</b>"));
        assert!(fixture.h().changes(revision).unwrap().changes.is_empty());
    }

    fn ffi(value: Value) -> Value {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() <= crate::MAX_REQUEST_BYTES);
        unsafe {
            let response = crate::tiez_core_request(bytes.as_ptr(), bytes.len());
            assert!(!response.is_null());
            let value = serde_json::from_slice(std::ffi::CStr::from_ptr(response).to_bytes()).unwrap();
            crate::tiez_core_free(response); value
        }
    }
    struct AbiFixture { handle: u64, path: PathBuf, _guard: std::sync::MutexGuard<'static, ()> }
    impl AbiFixture {
        fn new() -> Self {
            let guard = crate::ABI_TEST_LOCK.lock().unwrap(); let path = directory();
            let result = ffi(json!({"op":"open","database_path":path.join("history.db")}));
            assert_eq!(result["ok"], true, "{result}");
            Self { handle: result["result"]["handle"].as_u64().unwrap(), path, _guard: guard }
        }
    }
    impl Drop for AbiFixture {
        fn drop(&mut self) { if self.handle != 0 { ffi(json!({"op":"close","handle":self.handle})); } cleanup(&self.path); }
    }
    #[test]
    fn abi_chunked_utf8_roundtrip_keeps_small_requests_and_single_mutation_receipt() {
        let fixture = AbiFixture::new(); let handle = fixture.handle;
        let limits = ffi(json!({"op":"history_upload_limits","handle":handle}));
        assert_eq!(limits["result"]["history_revision"], 0); assert_eq!(limits["result"]["version"], 1); assert_eq!(limits["result"]["max_payload_bytes"], MAX_PAYLOAD);
        let content = format!(" {}{}{}😀{} ", char::from(13), char::from(10), "原始正文".repeat(40000), char::from(9));
        let body = serde_json::to_string(&json!({"content_type":"text","content":content})).unwrap();
        let result = ffi(json!({"op":"history_upload_begin","handle":handle,"version":1,"target":{"kind":"add"},
            "expected_revision":0,"total_bytes":body.len(),"sha256":hash(body.as_bytes())}));
        assert_eq!(result["ok"], true, "{result}");
        let token = result["result"]["token"].as_str().unwrap();
        let mut offset = 0;
        while offset < body.len() {
            let mut end = (offset + MAX_CHUNK).min(body.len()); while !body.is_char_boundary(end) { end -= 1; }
            let result = ffi(json!({"op":"history_upload_append","handle":handle,"token":token,"offset":offset,"data":&body[offset..end]}));
            assert_eq!(result["result"]["received_bytes"], end, "{result}"); offset = end;
        }
        let committed = ffi(json!({"op":"history_upload_commit","handle":handle,"token":token}));
        assert_eq!(committed["ok"], true, "{committed}"); assert_eq!(committed["result"]["revision"], 1);
        assert!(serde_json::to_vec(&committed).unwrap().len() < 2048);
        let id = committed["result"]["id"].as_i64().unwrap();
        let entry = ffi(json!({"op":"history_entry","handle":handle,"id":id}));
        assert_eq!(entry["result"]["content"], content);
        assert_eq!(ffi(json!({"op":"history_upload_commit","handle":handle,"token":token}))["error"]["code"], "transfer_missing");
        assert_eq!(ffi(json!({"op":"history_upload_limits","handle":handle}))["result"]["history_revision"], 1);
        assert_eq!(ffi(json!({"op":"snapshot","handle":handle}))["result"]["snapshot"]["revision"], 0);
    }
    #[test]
    fn abi_strict_schema_invalid_handles_abort_and_close_do_not_retain_uploads() {
        let mut fixture = AbiFixture::new(); let handle = fixture.handle;
        assert_eq!(ffi(json!({"op":"history_upload_limits","handle":0}))["error"]["code"], "invalid_handle");
        let request = json!({"op":"history_upload_begin","handle":handle,"version":1,"target":{"kind":"add"},
            "expected_revision":0,"total_bytes":MAX_PAYLOAD,"sha256":"0".repeat(64)});
        let mut invalid = request.clone(); invalid["target"]["path"] = json!("not a core resource");
        assert_eq!(ffi(invalid)["error"]["code"], "invalid_request");
        let mut invalid = request.clone(); invalid["target"]["kind"] = json!("close");
        assert_eq!(ffi(invalid)["error"]["code"], "invalid_request");
        let resource = ffi(request); assert_eq!(resource["ok"], true, "{resource}");
        let token = resource["result"]["token"].as_str().unwrap();
        assert_eq!(ffi(json!({"op":"history_upload_append","handle":0,"token":token,"offset":0,"data":"x"}))["error"]["code"], "invalid_handle");
        assert_eq!(ffi(json!({"op":"history_upload_status","handle":handle,"token":token}))["result"]["received_bytes"], 0);
        assert_eq!(ffi(json!({"op":"close","handle":handle}))["ok"], true);
        fixture.handle = 0;
        assert_eq!(ffi(json!({"op":"history_upload_status","handle":handle,"token":token}))["error"]["code"], "invalid_handle");
        let reopened = ffi(json!({"op":"open","database_path":fixture.path.join("history.db")}));
        assert_eq!(reopened["ok"], true, "{reopened}");
        fixture.handle = reopened["result"]["handle"].as_u64().unwrap();
        assert_ne!(fixture.handle, handle);
        assert_eq!(ffi(json!({"op":"history_upload_status","handle":fixture.handle,"token":token}))["error"]["code"], "transfer_missing");
        assert_eq!(ffi(json!({"op":"history_upload_abort","handle":fixture.handle,"token":token}))["result"]["aborted"], false);
        let resource = ffi(json!({"op":"history_upload_begin","handle":fixture.handle,"version":1,"target":{"kind":"add"},
            "expected_revision":0,"total_bytes":MAX_PAYLOAD,"sha256":"0".repeat(64)}));
        assert_eq!(resource["ok"], true, "{resource}");
        assert_ne!(resource["result"]["token"], token);
    }
}
