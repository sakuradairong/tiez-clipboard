# Windows native migration prototype

This Windows-first branch incrementally introduces a WinUI 3 host and a Tauri-free Rust core. The existing Tauri application is unchanged. This prototype uses an explicitly supplied, separate SQLite database; it does not discover, migrate, or open the installed application's data.

## Rust core

```powershell
cargo test --manifest-path native/core/Cargo.toml
cargo build --manifest-path native/core/Cargo.toml
```

The DLL is `native/core/target/debug/tiez_native_core.dll` on Windows. Only four configuration keys are supported: `app.language` (`zh`, `en`, `tw`), `app.color_mode` (`system`, `light`, `dark`), `app.deduplicate`, and `app.show_search_box` (literal `true`/`false`). These are prototype configuration values; capture and paste behavior are not implemented yet.

## ABI v1

- `uint32_t tiez_core_abi_version(void)` returns `1`.
- `char* tiez_core_request(const uint8_t* request, size_t len)` accepts 1..65536 bytes of UTF-8 JSON and returns owned, NUL-terminated UTF-8 JSON.
- `void tiez_core_free(char* response)` frees each response exactly once; null is allowed. Never free a response with a different allocator or unload the DLL while it is in use.

The caller owns the request and keeps its bytes valid for the call. Results are either `{ "ok": true, "result": ... }` or `{ "ok": false, "error": { "code": "...", "message": "..." } }`. Calls are serialized and have no UI thread affinity. Rust panics inside dispatch become errors under the default unwind profile; process-fatal failures such as allocation failure are not recoverable. Do not set `panic=abort` if panic recovery is required.

Commands:

```json
{"op":"open","database_path":"C:\\absolute\\isolated\\probe.db"}
{"op":"snapshot","handle":1}
{"op":"update","handle":1,"expected_revision":0,"patch":{"app.language":"en"}}
{"op":"close","handle":1}
```

`open` requires an absolute path with an existing parent directory. It returns a numeric handle and a snapshot. Share that handle across windows; a second concurrent open is rejected. `update` returns `changed` and a snapshot. Accepted changes advance a durable revision in the same transaction as the values. Reads, invalid/stale requests, failed writes and no-op saves do not advance it. Consumers publish accepted snapshots to their views only when `changed` is true; no callback thread or polling loop is required. Unknown database rows are retained and never included in snapshots. `close` invalidates the handle; closing a settings window must not close the core shared by the main window.

## Managed interop smoke check

Requires .NET SDK 10 and a Windows x64 Rust toolchain. Build the debug DLL first, then:

```powershell
dotnet run --project native/managed/TieZ.Core.Smoke
```

The managed client owns one core on the creating thread, copies UTF-8 responses before freeing native memory in `finally`, and publishes accepted snapshots only after a changed save. The smoke program loads the real DLL and uses a temporary Unicode database directory that it removes on exit. For Release, build Rust with `--release` before using `-c Release`.

## WinUI 3 host

This unpackaged Windows x64 prototype requires .NET 10 Desktop Runtime and Windows App Runtime 1.8 (or their SDK/development installations). It uses WinUI controls with no HTML renderer. Build Rust first, then:

```powershell
dotnet build native/managed/TieZ.WinUI/TieZ.WinUI.csproj
dotnet run --project native/managed/TieZ.WinUI
```

The main window opens one independent settings window. Repeated opens activate it; closing it releases its subscriptions, while the application keeps the shared core alive. Accepted saves update both windows; initialization/reopening never writes settings. Closing the main window closes settings and disposes the core. Theme and search visibility affect the main view; language and deduplication are stored prototype settings only. Clipboard history/capture/paste, full localization, migration of existing settings and packaging remain future work.

Normal runs use `%LOCALAPPDATA%/TieZ.Native.Prototype/settings.db`, separate from installed TieZ. To run an actual window smoke check after building:

```powershell
$exe = Resolve-Path native/managed/TieZ.WinUI/bin/Debug/net10.0-windows10.0.19041.0/win-x64/TieZ.WinUI.exe
$report = Join-Path $PWD .tmp/winui-smoke.json
# Create the report's parent directory first if it does not exist.
Start-Process -WindowStyle Hidden -FilePath $exe -ArgumentList @('--smoke', $report) -Wait
Get-Content $report
```

The check invokes the native Save button through its automation peer and verifies shared settings, duplicate-open handling, main-view updates, reopen without writes, teardown and cleanup, and absence of WebView2 in loaded modules/assemblies. It uses a temporary Unicode data directory. This is functional automation, not manual visual or accessibility acceptance. Running on a machine with WebView2 installed does not establish acceptance on a clean machine without its runtime. The Windows App SDK transitively restores the WebView2 package, but the host explicitly excludes all of its assets. A fresh output contains no WebView2 files or runtime dependency entries; the window smoke check also passes from that output. Existing output folders from older builds may still contain stale files; clean before checking deployment contents. SDK package restore and application runtime requirements are separate.

## Complete replacement tracking and shared storage

See [MIGRATION.md](MIGRATION.md) and [compatibility.json](compatibility.json) for the full replacement scope and unresolved acceptance gates. Check source coverage with `python native/tools/inventory.py --check`.

`native/storage` compiles the existing model, database initialization, migrations, repositories and DPAPI code directly from their legacy source files. This is transitional source sharing, not a copied schema or a Tauri dependency; do not delete those files while either shell uses them. Validate with `cargo test --locked --manifest-path native/storage/Cargo.toml`. The initial 36 Windows tests cover whitespace/hash semantics, schema migrations, repository transaction rollback, attachments, sensitive tags and encrypted settings. The shared core now uses this crate for its history database, with the prototype settings connection opening the same explicit database file.

## Core history API (native UI integration pending)

ABI v1 now accepts the following operations in addition to settings. All use the same application-owned handle:

```json
{"op":"history_page","handle":1,"query":{"limit":50,"offset":0,"search":"中文"}}
{"op":"history_entry","handle":1,"id":1}
{"op":"history_add","handle":1,"content_type":"rich_text","content":" text ","html_content":"<b>text</b>","tags":[]}
{"op":"history_pin","handle":1,"id":1,"is_pinned":true}
{"op":"history_edit","handle":1,"id":1,"content":" revised "}
{"op":"history_tags","handle":1,"id":1,"tags":["work"]}
{"op":"history_delete","handle":1,"id":1}
{"op":"history_changes","handle":1,"after_revision":0}
```

History revisions are per core lifetime and separate from settings revisions. A page contains summaries (preview at most 500 characters, no original HTML/body); fetch original content with `history_entry`. Database page limits are 1..200. Like the legacy list, the first page additionally includes all matching bounded session-only entries. Use the returned `next_offset` and `revision` as the next query's `offset` and `expected_revision`. A revision mismatch requires restarting at offset zero. Each filtered call scans at most 2,000 stored rows; an empty page with `has_more=true` is not the end. Query filters also include `content_type`, exact `tag`, `tag_only`, and `pinned_only`. Search includes decrypted content, source, tags and valid non-sensitive OCR/QR records; `%` and `_` are literal characters.

Manual entries are persisted as in the old manual-entry command. Rust `record_processed` accepts pipeline-processed entries and provides bounded negative-ID session storage; it does not implement OS capture, cleanup or deduplication. Pinning/tagging a session item returns its positive ID and `replaced_id`; failure preserves the old item. Mutations return IDs/revision and queue a bounded change record. Consumers behind the 128-record feed receive `resync_required`. Successful persisted changes set `persistent_sync_pending`; a sync worker is not integrated yet, so this never claims remote delivery.

Sensitive save/tag changes validate encryption and commit tags, encrypted fields and OCR cleanup atomically. Same-source DPAPI and portable behavior is retained. Native default and portable core test suites each currently pass 13 tests; storage source tests pass 36. Full capture/paste, batch/clear/tag-management UI, global-tag APIs and network acceptance remain pending. Requests still have a 64 KiB cap; oversized edits/imports need a chunked transport before complete replacement. No actual user database has been opened.
