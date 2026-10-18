//! Real core -> export -> native desktop launch, using only copies of this fixture.
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiez_native_core::{external_open::OpenService, HistoryQuery, HistoryService};
use tiez_native_platform::open::Route;
const FIXTURE: &str = "TieZ-core-编辑器 %20 ' &.exe";
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn receive(path: &Path) -> PathBuf {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "core launch did not reach fixture");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len() % 2, 0);
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    let result = PathBuf::from(OsString::from_wide(&units));
    std::fs::remove_file(path).unwrap();
    result
}
fn main() {
    let exe = std::env::current_exe().unwrap();
    if exe.file_name() == Some(OsStr::new(FIXTURE)) {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        assert_eq!(args.len(), 1);
        let bytes: Vec<_> = args[0].encode_wide().flat_map(u16::to_le_bytes).collect();
        let parent = exe.parent().unwrap();
        std::fs::write(parent.join("observed.pending"), bytes).unwrap();
        std::fs::rename(parent.join("observed.pending"), parent.join("observed.bin")).unwrap();
        return;
    }
    let root =
        std::env::temp_dir().join(format!("TieZ-open-core-process-{}", uuid::Uuid::new_v4()));
    assert!(root.is_absolute());
    std::fs::create_dir(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let fixture = root.join(FIXTURE);
    std::fs::copy(&exe, &fixture).unwrap();
    let history = Arc::new(Mutex::new(
        HistoryService::open(&root.join("fixture.db")).unwrap(),
    ));
    let mut service = OpenService::new(history.clone(), root.clone()).unwrap();
    let values =
        std::collections::BTreeMap::from([("app.text".into(), fixture.to_str().unwrap().into())]);
    let mut exports = Vec::new();
    for (kind, private, session) in [
        ("text", false, false),
        ("code", true, false),
        ("text", false, true),
    ] {
        let body = format!("  原始内容 {kind} {private} {session}\r\n\t  ");
        let id = {
            let mut h = history.lock().unwrap();
            let stored = h
                .add_manual(
                    kind.into(),
                    body.clone(),
                    None,
                    if private {
                        vec!["sensitive".into()]
                    } else {
                        vec![]
                    },
                )
                .unwrap()
                .id;
            if session {
                let mut entry = h.entry(stored).unwrap();
                entry.id = 0;
                h.record_processed(entry, false).unwrap().id
            } else {
                stored
            }
        };
        assert_eq!(id < 0, session);
        let revision = history
            .lock()
            .unwrap()
            .page(HistoryQuery::default())
            .unwrap()
            .revision;
        let result = service
            .open_at_revision(id, revision, private, &values)
            .unwrap();
        assert_eq!(result.route, Route::Desktop);
        assert_eq!(result.id, id);
        assert_eq!(result.history_revision, revision);
        assert!(result.exported && result.edit_watch_required);
        let exported = receive(&root.join("observed.bin"));
        assert_eq!(exported.file_name(), Some(OsStr::new("content.txt")));
        assert!(exported
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("TieZ-open-"));
        assert_eq!(std::fs::read(&exported).unwrap(), body.as_bytes());
        let h = history.lock().unwrap();
        assert_eq!(h.page(HistoryQuery::default()).unwrap().revision, revision);
        let entry = h.entry(id).unwrap();
        assert_eq!(entry.content, body);
        assert_eq!(entry.use_count, 0);
        drop(h);
        exports.push(exported);
    }
    assert_eq!(service.status().active_exports, 3);
    assert_eq!(service.close().unwrap(), 3);
    assert!(exports.iter().all(|p| !p.exists()));
    drop(service);
    drop(history);
    // The child reports immediately before exit. Wait only for its executable mapping to release.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::fs::remove_file(&fixture) {
            Ok(()) => break,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("fixture image did not release: {e}"),
        }
    }
    std::fs::remove_dir_all(&root).unwrap();
    println!("core open process smoke: 3 actual native desktop launches, stored/sensitive/session IDs, code handler fallback, exact exported bytes, unchanged history/use counts, owned-file cleanup; no system-association, packaged activation or edit-writeback acceptance");
}
