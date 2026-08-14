# TieZ WinUI 3 main-window migration slice

> **First production-intended native Windows slice — Tauri/WebView2 remains the
> fallback entry point.**

This experiment tests one question:

> Can a C++/WinRT WinUI 3 main window consume a narrow Rust C interface in the
> same process while providing enough clipboard-list interaction to justify a
> real Windows-native product slice?

The executable does **not yet** replace the Tauri window or start the real
clipboard listener. The reusable history behavior now lives in the standalone,
Tauri-independent `tiez-core` crate. By default it uses synthetic in-memory
data. An opt-in adapter can open a **copied** TieZ `clipboard.db` in SQLite
read-only mode. The existing application remains available as a rollback path.
The production Tauri list, search, and full-content commands now use the same
storage-neutral merge/search policies through `TauriHistoryAdapter`; their
command names and serialized `ClipboardEntry` contract remain unchanged.

## Shape

```text
Tiez.WinUIProbe.exe
  WinUI 3 / XAML / C++/WinRT
        │ dynamic loading + C ABI v3
        ▼
tiez_winui_core.dll
  Rust C ABI transport adapter
        │
        ▼
tiez-core::clipboard_history
  Tauri-independent deep module with switchable adapters:
    - synthetic in-memory data (default)
    - production-schema SQLite history (opt-in, read-only)
```

The C ABI interface is deliberately small:

- create/destroy one core handle;
- fetch a UTF-8 JSON snapshot for a search query;
- fetch full content metadata for one stable entry ID;
- report the active adapter and whether it is read-only;
- apply `pin`, `delete`, `paste-plain`, or `paste-rich` in memory mode;
- return a structured mutation result with requested/effective/replacement IDs,
  removal state, generation, and a display message;
- retrieve per-thread errors and free returned strings.

The shared Rust module owns adapter selection, query filtering, sorting,
sensitive-preview redaction, relative timestamps, generation tracking, and
memory-only actions. Its `production_history` policy also owns session/persisted
merge rules, stable pinned ordering, and session search matching for the Tauri
adapter. The C ABI library owns only transport concerns: UTF-8/C string
ownership, panic containment, JSON serialization, and ABI stability.

The JSON format is only a prototype transport. A production seam should use
versioned request/response structs or another explicitly versioned wire format.

## What the UI demonstrates

- native WinUI 3 cards and controls;
- search driven by the Rust snapshot;
- a native master-detail view backed by full-content lookup;
- pin/unpin and delete state mutations in Rust;
- plain/rich paste requests crossing the C interface;
- UTF-8 text, including Chinese and emoji;
- a five-second hide/show lifecycle action;
- an optional ready marker for startup and memory measurement.

Paste buttons intentionally record requests instead of changing the system
clipboard. All action buttons are disabled when the real-history adapter is
active. Connecting them to TieZ's real paste implementation requires first
extracting Tauri-independent Rust modules.

## Windows prerequisites

- Windows 11 x64 recommended; Windows 10 2004 is the current project minimum;
- Visual Studio 2022 with:
  - **Desktop development with C++**;
  - **Windows application development** / Windows App SDK tools;
  - MSVC v143;
  - Windows 11 SDK `10.0.26100`;
- repository-pinned Rust `1.88.0-x86_64-pc-windows-msvc` toolchain;
- PowerShell 5.1 or newer;
- internet access for first-time NuGet restore.

The project currently pins the maintained Windows App SDK `1.8.260710003`
line instead of the newer `2.x` line so the first build tests a serviced,
established toolchain before exploring a major SDK upgrade.

## Linux development

Linux can run the Rust core tests and cross-compile the Windows MSVC DLL, but
it cannot run the WinUI XAML/XBF compiler. Install the repository-pinned Rust
toolchain plus the Linux cross-compilation prerequisites:

```bash
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
# Debian/Ubuntu
sudo apt-get install clang lld
```

Then run from this directory:

```bash
./build-linux.sh
```

The helper runs the shared `tiez-core` tests and the C ABI tests, builds
`tiez_winui_core.dll` with `cargo-xwin`, and copies it to:

```text
artifacts/x64/Release/tiez_winui_core.dll
```

Use `./build-linux.sh --skip-windows-target` for a native-test-only pass. The
complete `Tiez.WinUIProbe.exe` still requires `build.ps1` on Windows; the
`winui3-probe` GitHub Actions workflow provides the reproducible remote build.

## Build and run

From this directory in PowerShell:

```powershell
rustup toolchain install 1.88.0-x86_64-pc-windows-msvc
Set-ExecutionPolicy -Scope Process Bypass
.\build.ps1 -Configuration Release
.\artifacts\x64\Release\Tiez.WinUIProbe.exe
```

`build.ps1` performs these steps:

1. runs the shared `tiez-core` and WinUI C ABI tests;
2. builds the `cdylib` in release mode;
3. restores the native NuGet packages into the experiment directory;
4. resolves Visual Studio's MSBuild with `vswhere`;
5. builds the unpackaged, self-contained WinUI executable;
6. places the executable and Rust DLL in the same artifact directory.

Override the Rust DLL path for debugging with:

```powershell
$env:TIEZ_WINUI_CORE_DLL = "C:\path\to\tiez_winui_core.dll"
```

### Read copied TieZ history

The default remains the mutable synthetic adapter. To inspect real history,
first stop TieZ and copy `clipboard.db` to a scratch directory. If
`clipboard.db-wal` and `clipboard.db-shm` exist, copy them beside it too. Do
not point this experiment at the live production files.

The installed Windows database is normally under TieZ's app-data directory;
portable mode stores it under `data\clipboard.db` beside the executable, and
`datapath.txt` may redirect the directory. Then launch the probe with:

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
.\artifacts\x64\Release\Tiez.WinUIProbe.exe
```

The header and status should show `sqlite-read-only`. The adapter reads at
most 200 latest entries, supports case-insensitive search over preview, source
app, and content type, and never opens SQLite with write flags. It deliberately
does not:

- read session-only negative-ID entries held in the running Tauri process;
- display sensitive-tagged or `dpapi:` previews (they are replaced with a
  sensitive-entry label);
- expose sensitive or encrypted payloads without the production privacy and
  Windows decryption adapter;
- perform mutation/paste operations.

Use **Open details** to read the full persisted payload for non-sensitive
entries. The details panel keeps sensitive and encrypted entries metadata-only.

Unset the variable to return to synthetic mode:

```powershell
Remove-Item Env:TIEZ_WINUI_DB_PATH -ErrorAction SilentlyContinue
```

## Measurement

Build Release first, then run:

```powershell
.\measure.ps1 -Configuration Release -SampleSeconds 30
```

The script writes:

```text
artifacts/x64/Release/measurement.json
```

It records requested-to-ready time plus one-second samples of private bytes,
working set, handles, and thread count. Use the **Hide for 5 seconds** button for
manual visible/hidden comparison.

The SQLite mode allows UI measurements against a copied production dataset,
but it still omits the clipboard listener, sync, paste, hotkeys, tray, and
other production services. Record the active adapter with every result.

## Acceptance checklist

### Build and ABI

- [ ] Release build succeeds from a fresh NuGet cache.
- [ ] `tiez_winui_core.dll` loads without changing `PATH`.
- [ ] Status shows `Rust ABI 3`.
- [ ] Chinese and emoji render without replacement characters.
- [ ] Missing/wrong DLL produces a visible startup error instead of a crash.

### Interaction

- [ ] Search filters by preview, source app, and content type.
- [ ] Pin/unpin changes the card and generation.
- [ ] Delete removes an entry.
- [ ] Mutation status shows the Rust result message and generation.
- [ ] Plain/rich paste buttons update the Rust action status.
- [ ] Open details displays full UTF-8 content without WebView2.
- [ ] Keyboard Tab traversal and text selection work.
- [ ] Hide/show restores a focused, usable window.

### Copied production history

- [ ] `TIEZ_WINUI_DB_PATH` switches the badge to `sqlite-read-only`.
- [ ] The newest persisted items match the production TieZ history ordering.
- [ ] Search matches preview, source app, and content type without writing.
- [ ] All item action buttons are disabled.
- [ ] Sensitive-tagged and encrypted previews show the sensitive-entry label.
- [ ] Sensitive-tagged and encrypted details remain metadata-only.
- [ ] The copied database and optional WAL/SHM files remain byte-identical.

### Evidence before a real product slice

- [ ] At least five independent release memory runs.
- [ ] At least 100 open/hide/show/close cycles without crashes.
- [ ] Median requested-to-ready no more than 750 ms.
- [ ] Worst five requested-to-ready samples no more than 1500 ms.
- [ ] Narrator announces search, buttons, list content, and status changes.
- [ ] Per-monitor DPI, IME, multiple monitors, and Windows 10/11 startup pass.

## Production extraction path

Do not expose Tauri types through the C interface. Extract deep Rust modules in
this order, each with an in-memory adapter and the existing Tauri adapter:

1. `ClipboardHistory` — **production read seam connected**: WinUI preview/full
   content and Tauri list/search/content now share Tauri-independent policies;
   stable negative session IDs, rich-text normalization, and production
   decryption behavior remain preserved by `TauriHistoryAdapter`;
2. `ClipboardMutation` — **production Tauri seam connected** for pin, tag,
   delete, clear, and pinned-order changes through `TauriMutationAdapter` and
   `tiez-core` session policies, while preserving replacement IDs, privacy
   encryption/decryption jobs, cleanup events, and cloud sync. The WinUI SQLite
   adapter remains read-only until a production write adapter and C ABI are
   validated on Windows;
3. `PasteCoordinator` — focus restoration, rich/plain payload, paste queue;
4. `UiLifecycle` — hotkey/tray wake, hide, close, and single-instance policy.

Only after the first three modules work through both adapters should the WinUI
executable become an alternative application entry point.

## Primary references

- [WinUI 3](https://learn.microsoft.com/en-us/windows/apps/winui/winui3/)
- [Create a WinUI 3 app](https://learn.microsoft.com/en-us/windows/apps/get-started/start-here)
- [Distribute an unpackaged WinUI app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/unpackage-winui-app)
- [Windows App SDK runtime bootstrap](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/use-windows-app-sdk-run-time)
- [Official C++ unpackaged self-contained sample](https://github.com/microsoft/WindowsAppSDK-Samples/tree/main/Samples/SelfContainedDeployment/cpp/cpp-winui-unpackaged)
