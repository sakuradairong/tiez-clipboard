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
