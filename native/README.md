# Windows native migration prototype

This Windows-first branch incrementally introduces a WinUI 3 host and a Tauri-free Rust core. The existing Tauri entry point remains available as the migration reference. This prototype uses an explicitly supplied, separate SQLite database; it does not discover, migrate, or open the installed application's data.

## Rust core

```powershell
cargo test --manifest-path native/core/Cargo.toml
cargo build --manifest-path native/core/Cargo.toml
```

The DLL is `native/core/target/debug/tiez_native_core.dll` on Windows. The settings snapshot supports language, color mode, deduplication, search visibility, persistence, file/rich-text capture, privacy matching, cleanup rules/policies, and persistent limits. Reads never save defaults. Capture is explicitly started by the host; paste is still pending.

The core owns one stoppable Windows clipboard listener and capture worker per open handle. Startup records the current sequence without importing the existing clipboard. The worker uses the shared production format probes and transformations, preserves whitespace/source metadata, deduplicates with rich-format identity, and publishes history revisions. Settings acknowledgement applies the accepted policy before returning. Session-only captures keep negative IDs; persistent promotion reports replacement IDs. Save, encryption and storage-limit eviction share a transaction; owned attachment cleanup occurs after commit. Status reports counts and error codes without clipboard bodies. Stop/close joins the worker before releasing history; a synchronous delayed-render provider can delay shutdown until its read returns.

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
{"op":"clipboard_copy","handle":1,"id":1,"plain_text":false,"allow_sensitive":false}
{"op":"capture_start","handle":1}
{"op":"capture_status","handle":1}
{"op":"capture_stop","handle":1}
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

The main window opens one independent settings window. Repeated opens activate it; closing it releases its subscriptions, while the application keeps the shared core alive. Accepted saves update both windows; initialization/reopening never writes settings. Closing the main window closes settings and disposes the core. Theme and search visibility affect the main view; language remains a stored configuration value pending full localization. Deduplication and capture policies are applied by the Rust worker. The main history workspace now supports native incremental browsing, search/type/exact-tag/pinned filters, original-text details, add/edit/pin/tags and confirmed single deletion. Ordinary runs start capture when the main view loads. Its command bar can pause/restart capture and displays counters/error codes. The independent settings window edits all 14 supported values, including capture/persistence/privacy/limit and advanced cleanup policies; rejected saves preserve drafts, and returning to accepted values disables unnecessary saving. Paste, full localization, migration of existing settings and packaging remain future work.

Normal runs use `%LOCALAPPDATA%/TieZ.Native.Prototype/settings.db`, separate from installed TieZ. To run an actual window smoke check after building:

```powershell
$exe = Resolve-Path native/managed/TieZ.WinUI/bin/Debug/net10.0-windows10.0.19041.0/win-x64/TieZ.WinUI.exe
$report = Join-Path $PWD .tmp/winui-smoke.json
# Create the report's parent directory first if it does not exist.
Start-Process -WindowStyle Hidden -FilePath $exe -ArgumentList @('--smoke', $report) -Wait
Get-Content $report
```

The 42-phase check invokes real native buttons/dialogs through automation peers and verifies shared settings, duplicate-open handling, reopening without writes, history add/edit/pin/tags/delete, failed-save drafts, sensitive reveal/redaction after external changes, search/filter results, unchanged CRLF payloads, incremental loading, virtualization, capture start/pause/restart, settings lifecycle without service restart, rejected capture-policy drafts and accepted corrections, teardown with capture active, and absence of WebView2 in loaded modules/assemblies. With 241 synthetic records, one run loaded 200 rows while realizing 33 containers; this checks virtualization, not large-history performance. Programmatic text changes do not establish physical IME acceptance. It uses a temporary Unicode data directory. This is functional automation, not manual visual or accessibility acceptance. Running on a machine with WebView2 installed does not establish acceptance on a clean machine without its runtime. The Windows App SDK transitively restores the WebView2 package, but the host explicitly excludes all of its assets. A fresh output contains no WebView2 files or runtime dependency entries; the window smoke check also passes from that output. Existing output folders from older builds may still contain stale files; clean before checking deployment contents. SDK package restore and application runtime requirements are separate.

## Complete replacement tracking and shared storage

See [MIGRATION.md](MIGRATION.md) and [compatibility.json](compatibility.json) for the full replacement scope and unresolved acceptance gates. Check source coverage with `python native/tools/inventory.py --check`.

`native/storage` compiles the existing model, database initialization, migrations, repositories and DPAPI code directly from their legacy source files. This is transitional source sharing, not a copied schema or a Tauri dependency; do not delete those files while either shell uses them. Validate with `cargo test --locked --manifest-path native/storage/Cargo.toml`. The initial 36 Windows tests cover whitespace/hash semantics, schema migrations, repository transaction rollback, attachments, sensitive tags and encrypted settings. The shared core now uses this crate for its history database, with the prototype settings connection opening the same explicit database file.

## Core history API

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

Sensitive save/tag changes validate encryption and commit tags, encrypted fields and OCR cleanup atomically. Same-source DPAPI and portable behavior is retained. Native default and portable core test suites each currently pass 30 tests; storage source tests pass 39. Full format round-trip acceptance/paste, batch/clear/global-tag-management UI, global-tag APIs and network acceptance remain pending. Requests still have a 64 KiB cap; oversized edits/imports need a chunked transport before complete replacement. No actual user database has been opened.

## Asynchronous managed history client

The C# client now exposes typed history query/entry/mutation/change-feed methods. Invoke them from the client's owner thread, then await them; serialization snapshots inputs before yielding and native work runs off the UI thread behind one request gate. Settings operations and disposal use the same gate. Cancellation stops queued calls, while an accepted mutation reports its actual native result even if cancellation arrives later. Disposal cancels waiting work and waits for the in-flight native response to be copied/freed before closing the handle. Views must discard stale or cancelled read results and cancel their own requests when closing.

The real-DLL smoke program additionally covers history DTOs, pages, Unicode/CRLF/HTML preservation, pin/tags/rich-to-plain edit/delete, stale pages, literal search, change feed, cancellation during a save, 32 queued reads at disposal and immediate reopen. The separate WinUI smoke check validates the native history workspace; these suites validate the OS listener lifecycle but do not establish real-content clipboard capture or target-application paste compatibility.

## Native history workspace

The virtualized ListView loads revisioned 50-row pages and continues through bounded empty scan pages. Search waits 250 ms and pauses during TextBox composition; new filters cancel older reads and clear the previous selection/detail. The change feed refreshes an idle first page, while active browsing keeps its position and offers an explicit refresh. Changed rows are redacted until refreshed, and stale mutations are blocked. View disposal cancels timers/reads before application-owned core disposal.

Sensitive rows bind a placeholder rather than their body preview; details are fetched only after explicit reveal. Changing the selection, filtering or refreshing clears the reveal. Accepted saves reload the returned ID (including session promotion), failed saves retain the dialog draft, and deletion requires confirmation. Native TextBox normalizes displayed/editor line endings, so accepting an unchanged edit preserves the original native payload; surrounding whitespace is retained. Details currently display original text only: native image/file/rich/table preview and format-specific paste remain separate migration items. Multi-selection, global tag management, real IME input, accessibility and mixed-DPI acceptance are still pending.

## Windows platform services

`native/platform` introduces a Tauri-free Win32 notification service and compiles the production `win_clipboard.rs` format implementation directly. `ClipboardListener::start()` acknowledges successful class/window/listener registration before returning one owner and a bounded receiver. The message procedure only stores a sequence hint and tries to enqueue one wake-up; consumers must read formats on their own worker and verify sequence stability. Bursts coalesce without blocking the window procedure. Stop/Drop signal a kernel event, remove the listener, destroy the HWND, unregister its unique class and join its thread. The timeout fallback also observes stop requests if signalling fails.

Run `cargo test --locked --manifest-path native/platform/Cargo.toml`. Six Windows tests pass: two existing DIB-layout regressions and four real message-window tests covering repeated startup/shutdown, 256 synthetic updates, receiver drop and Drop/join/disconnect. These tests post only synthetic notifications to owned message windows; they never read or modify clipboard contents. The native core now owns this listener and its capture worker; ordinary WinUI runs start it, and settings windows only borrow the core. Paste remains pending. A notification lifecycle check does not establish clipboard format round-trip or target-focus acceptance.

## Shared clipboard content processing

`native/content` compiles the existing clipboard utilities and bounded local-image reader directly. The only legacy refactor moves `current_clipboard_image_roots` into a Tauri host adapter; its function body is unchanged. The native library supplies `with_data_directory(directory, || ...)` for synchronous processing with an explicit core-owned data directory. Its thread-local scope restores previous roots after nested calls and panics, and does not propagate implicitly to another worker. Clipboard hashing/fingerprints, rich/CF_HTML and named-format preservation, animated-image handling, cleanup and sensitive matching retain their source implementations.

Validate with `cargo test --locked --manifest-path native/content/Cargo.toml` (118 passed), and with `--features portable -- --test-threads=1` (118 passed). An initial parallel portable run failed the existing localhost GIF-as-JPEG fixture; it passed alone and serially, but the concurrency failure's cause remains unresolved. The legacy Windows suite passes 277 tests. The content dependency tree contains no Tauri/Wry/WebView package. Source sharing remains transitional; do not delete or independently fork linked helpers. The library supplies reusable operations; core owns capture/dedup/history while network distribution and native rich rendering remain pending.

## Native clipboard reader and shared processing stages

`content::capture::read` now uses the production probes for text, CF_HTML, RTF/spreadsheet named formats, native raster/animated formats, DIB fallback and file/video inputs. File capture off still retains actual animation payloads as before; Word/WPS probing and conservative bitmap handling retain their source implementations. Source metadata comes from a read-only owner/foreground process query. Reads occur outside the message/UI thread, reject changed sequence numbers, and observe cancellation between probes. Delayed OS rendering/probe calls themselves remain synchronous; shutdown must join the in-flight reader rather than unload its DLL early. Last-target tracking is a separate pending host integration.

The existing discovery and cleanup/privacy/image transformation stages now call shared `processing` functions with explicit options/data directory. The old shell remains an adapter for the same algorithms. All helper fixtures are shared too: the content suite now passes 118 tests in default and serial portable configurations. Guard/fingerprint tests do not claim actual OS format round trips. The old Windows Rust suite passes all 277 tests after the extraction. The native core orchestrates capture, policy, dedup and persistence, and ordinary WinUI runs collect future clipboard changes. Network distribution, format round trips and target-focus/paste acceptance remain pending.

## Managed capture and native worker acceptance

Typed `StartCaptureAsync`, `StopCaptureAsync` and `GetCaptureStatusAsync` share the history request gate, cancellation and disposal rules. The real-DLL smoke covers explicit/idempotent start, stop/join, restart, policy revision acknowledgement, cancelled start, active-service disposal and immediate reopen/reuse. It never writes the user's clipboard. Window smoke keeps capture off during deterministic history fixtures, then exercises the real capture controls and independent settings while the service runs.

Seven synthetic-input worker tests exercise the production discovery/transform/history stages without touching the Windows clipboard: whitespace/source/privacy preservation, acknowledged policy changes with session promotion, rich-format burst identity, local self-write suppression, persistence-failure preservation, stop/restart/idempotence and disconnected-provider teardown. They supplement, but do not replace, real text/image/file/Office round trips or clean-OS deployment checks. No release or replacement-readiness claim follows from these checks.

## Native clipboard write preparation

`native/content::write::PreparedWrite` prepares text, file lists, CF_HTML with preserved RTF/Office named formats, still-image RGBA/PNG, and original animated GIF/WebP bytes before writing. CF_HTML serialization and animation/PNG decisions are shared with the legacy paste path. Plain-text mode preserves the stored text exactly. Image-only rich fallbacks append HTML to the raster formats; a preview fallback cannot displace real text/HTML.

Local image reads require explicit trusted roots; encoded payloads are limited to 32 MiB, dimensions to 16384 per axis and decoder and prepared RGBA allocations to 128 MiB. Unsupported/invalid resources return explicit preparation errors before clipboard mutation. Commit uses the production Windows format writers and reports a sequence plus any animation compatibility-file path. Multi-format OS writes are not transactional; failures distinguish whether the clipboard sequence changed. Production low-level writers and real target compatibility still require OS round-trip acceptance. Eleven preparation/fake-sink tests pass, and default/serial portable content suites each pass 118 tests. The core now exposes copy by ID and WinUI provides original-format/plain-text copy actions. Paste/focus handling and successful OS format acceptance remain pending.

## History copy and OS clipboard acceptance

`clipboard_copy` takes an existing history ID, `plain_text` and explicit `allow_sensitive` (both default false). It reads the authoritative stored/session body in Rust, rejects missing/invalid/hidden-sensitive content before mutation, and returns `{id, clipboard_sequence}` on success. Copy does not itself change history. WinUI enables copy only for a loaded, current detail; revealing a sensitive entry enables its explicit copy. Accepted requests finish even if a later cancellation arrives, using the same managed request gate.

The copy commit holds the capture-policy gate until it publishes a sequence-bound echo marker. Actual prepared raster hashes cover image-only rich fallbacks and animation compatibility paths are retained as aliases. Later intentional identical clipboard updates have different sequence numbers and do not match this marker. The capture worker rechecks sequence after acquiring this gate, rejecting a read that became stale while a copy completed. Three core regressions cover sequence identity, missing/sensitive/invalid preconditions and this stale-read boundary. Windows text/HTML/named-format writers now propagate failed empty/register/set operations and free allocations if transfer fails; multi-format writes remain nontransactional, with propagated partial failures reported explicitly.

The 42-phase WinUI check additionally invokes the actual copy button with an invalid stored image, proving preparation fails before clipboard writing, feedback appears, and selection/body remain usable. It does not validate successful OS copy. Native content tests cover the actual prepared raster fingerprint.

`dotnet run --project native/managed/TieZ.Clipboard.Smoke` is an opt-in real-format check using a process-private Windows window station and desktop. It verifies isolation before creating its clipboard owner, never switches the visible desktop or sends keyboard input, and uses only an isolated temporary database/files. It exercises real capture/copy for text, CF_HTML/RTF/Office binary formats, files and DIB/PNG, explicit sensitive copy, echo and persistence promotion. On this development host, both full and minimum window-station access requests returned Win32 5 at `CreateWindowStation`, before any clipboard activity; **this suite has not passed here**. The tool returns exit code 77 when private-station creation is unavailable, and other failures remain errors. Its current build succeeds, but successful format behavior and fixture teardown still need execution in an environment permitting this isolation. It does not silently fall back to the interactive clipboard. Target-application paste, real focus restoration and no-WebView2 clean-system acceptance remain separate outstanding gates.


## Target tracking and return focus

The core owns one foreground-event hook and one top-level destruction hook on a single joined thread. `target_start`, `target_status`, `target_stop` and `target_restore` share the existing handle. Start before activating the main window to observe the prior external foreground. Status contains running/starts/generation/has_target/application/last_error; application is an executable basename, never a window title. The WinUI main view shows the target and offers return-focus/restart actions. Closing/reopening settings does not restart tracking.

`target_restore` requires `expected_generation`. An opaque internal ticket binds HWND, PID, thread and process creation time. Destroy events retire the matching top-level target and advance its generation; child-control destruction is ignored. Identity and exact foreground are rechecked around restoration. Event delivery is asynchronous; these checks reduce stale-handle risk but cannot make Windows focus or HWND lifetime atomic. Restoration is attempted only when the target already has focus or this process owns the foreground, using ordinary SetForegroundWindow. Refused activation, stale/closed targets and other foreground owners return errors. There is no keyboard injection or clipboard access in return-focus.

Fifteen platform tests cover rules, generation/identity changes, destruction retirement, callback filtering, real hook start/stop/restart/Drop and own-process exclusion. The real-DLL smoke covers explicit/idempotent service lifecycle, stale rejection, disposal/reopen. The 42-phase WinUI check additionally checks target lifecycle with independent settings and reports no WebView2 module loaded. Legacy Windows tests pass 279 cases. Native core default and portable suites pass 30 each.

`dotnet run --project native/managed/TieZ.Focus.Smoke` builds a controlled two-process foreground fixture without clipboard or keyboard APIs. On this host Windows refused setup activation on two attempts, before native restoration could be exercised. It now reports this unavailable setup with exit 77; the revised reporting was compiled, not rerun. Real cross-process focus restoration remains unverified. Do not force activation or treat the lifecycle checks as target-app acceptance. Actual paste input and post-paste policy remain pending.
