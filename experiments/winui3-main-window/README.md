# TieZ WinUI 3 main-window migration slice

> **First production-intended native Windows slice — Tauri/WebView2 remains the
> fallback entry point.**

This experiment tests one question:

> Can a C++/WinRT WinUI 3 main window consume a narrow Rust C interface in the
> same process while providing enough clipboard-list interaction to justify a
> real Windows-native product slice?

The executable does **not yet** replace the Tauri window or start the real
clipboard listener. The reusable history, paste, and window-lifecycle policies
now live in the standalone, Tauri-independent `tiez-core` crate. By default the
probe uses synthetic in-memory data. An opt-in adapter can open a TieZ
`clipboard.db` for read or write. **Do not run this executable at the same time
as Tauri** against the live database — the two entries are mutually exclusive
writers. The existing WebView2 application remains the production fallback.
The production Tauri list, search, and full-content commands still use the same
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
tiez-core
  clipboard_history · paste_coordinator · ui_lifecycle
    - synthetic in-memory data (default)
    - production-schema SQLite history (opt-in, writable unless TIEZ_WINUI_DB_READ_ONLY=1)
```

The C ABI interface is deliberately small:

- create/destroy one core handle;
- fetch a UTF-8 JSON snapshot for a search query;
- fetch full content metadata for one stable entry ID;
- report the active adapter and whether it is read-only;
- apply `pin`, `delete`, `paste-plain`, or `paste-rich` (memory or writable SQLite);
- return a structured mutation result with requested/effective/replacement IDs,
  removal state, generation, and a display message;
- retrieve per-thread errors and free returned strings.

The shared Rust module owns adapter selection, query filtering (including
`type:<content_type>` chips), sorting, sensitive-preview redaction, relative
timestamps, generation tracking, paste payload planning, and pin/delete writes.
`PasteCoordinator` plans hide → restore-focus → clipboard → Ctrl+V; the WinUI
shell captures the last foreground HWND and the Windows executor applies Unicode
text plus a paste keystroke. The C ABI library owns only transport concerns:
UTF-8/C string ownership, panic containment, JSON serialization, and ABI
stability.

The JSON format is only a prototype transport. A production seam should use
versioned request/response structs or another explicitly versioned wire format.

## What the UI demonstrates

- native WinUI 3 cards and controls;
- search plus type chips (`type:text` and friends) driven by the Rust snapshot;
- a native master-detail view backed by full-content lookup;
- pin/unpin and delete, including SQLite writes when the probe is the only process;
- real plain/rich paste through `PasteCoordinator` (Unicode text + Ctrl+V);
- keyboard up/down + Enter to paste, Esc to hide;
- Alt+C toggle, last-foreground HWND capture, and deactivate-to-hide (unless pinned);
- UTF-8 text, including Chinese and emoji;
- a five-second hide/show lifecycle action for memory measurements;
- an optional ready marker for startup and memory measurement.

Paste uses the shared coordinator. On Windows the probe writes CF_UNICODETEXT and
sends Ctrl+V after hiding and restoring the last foreground window. Sensitive and
encrypted payloads stay unavailable until a privacy adapter exists. Do not start
Tauri and this executable against the same `clipboard.db` at once.

## Windows prerequisites

- Windows 11 x64 recommended; Windows 10 2004 is the current project minimum;
- Visual Studio 2022 (or Build Tools) with:
  - **Desktop development with C++**;
  - **Windows application development** / Windows App SDK tools;
  - MSVC v143;
  - Windows 11 SDK `10.0.26100`;
  - `build.ps1` searches every VS product, including Build Tools;
- repository-pinned Rust `1.88.0-x86_64-pc-windows-msvc` toolchain;
- PowerShell 5.1 or newer;
- internet access for first-time NuGet restore.

The project currently pins the maintained Windows App SDK `1.8.260710003`
line instead of the newer `2.x` line so the first build tests a serviced,
established toolchain before exploring a major SDK upgrade. 1.8 ships WinUI
and WebView2 as split packages; `packages.config` restores those explicitly.

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

### Read or write TieZ history

The default remains the mutable synthetic adapter. To use a real database,
**stop Tauri first**. Point the probe at `clipboard.db` (copy WAL/SHM beside it
if they exist). The installed Windows database is normally under TieZ's
app-data directory; portable mode stores it under `data\clipboard.db` beside
the executable, and `datapath.txt` may redirect the directory.

Writable (WinUI as the only process — pin/delete persist):

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
.\artifacts\x64\Release\Tiez.WinUIProbe.exe
```

The header should show `sqlite` and **write enabled**. Pin/delete go through
`tiez_core_apply_action_json` and keep `replacement_id` null for persisted
positive IDs. Session-only negative IDs are still a Tauri-process concern.

Read-only inspection of a copied database (byte-identical check):

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
$env:TIEZ_WINUI_DB_READ_ONLY = "1"
.\artifacts\x64\Release\Tiez.WinUIProbe.exe
```

The header and status should show `sqlite-read-only`. The adapter reads at
most 200 latest entries, supports case-insensitive search over preview, source
app, and content type, plus exact `type:<name>` filters. It deliberately
does not:

- read session-only negative-ID entries held in the running Tauri process;
- display sensitive-tagged or `dpapi:` previews (they are replaced with a
  sensitive-entry label);
- expose sensitive or encrypted payloads without the production privacy and
  Windows decryption adapter;
- perform mutation/paste operations when `TIEZ_WINUI_DB_READ_ONLY=1`.

Use **Open details** to read the full persisted payload for non-sensitive
entries. The details panel keeps sensitive and encrypted entries metadata-only.

Unset the variables to return to synthetic mode:

```powershell
Remove-Item Env:TIEZ_WINUI_DB_PATH -ErrorAction SilentlyContinue
Remove-Item Env:TIEZ_WINUI_DB_READ_ONLY -ErrorAction SilentlyContinue
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
- [ ] Type chips send `type:text` / `type:image` / `type:url` / `type:code` / `type:files`.
- [ ] Pin/unpin changes the card and generation.
- [ ] Delete removes an entry.
- [ ] Mutation status shows the Rust result message and generation.
- [ ] Plain/rich paste writes the system clipboard and sends Ctrl+V after restoring the last HWND.
- [ ] Up/Down moves the selected card; Enter pastes plain text; Esc hides.
- [ ] Alt+C toggles visibility and captures the last foreground window.
- [ ] Sensitive cards show a redacted preview and a sensitive label.
- [ ] Open details displays full UTF-8 content without WebView2.
- [ ] Keyboard Tab traversal and text selection work.
- [ ] Hide/show restores a focused, usable window.

### Copied production history

- [ ] `TIEZ_WINUI_DB_PATH` with `TIEZ_WINUI_DB_READ_ONLY=1` switches the badge to `sqlite-read-only`.
- [ ] The newest persisted items match the production TieZ history ordering.
- [ ] Search matches preview, source app, and content type without writing.
- [ ] All item action buttons are disabled in read-only mode.
- [ ] Sensitive-tagged and encrypted previews show the sensitive-entry label.
- [ ] Sensitive-tagged and encrypted details remain metadata-only.
- [ ] The copied database and optional WAL/SHM files remain byte-identical.
- [ ] Without `TIEZ_WINUI_DB_READ_ONLY`, pin/delete persist and the badge shows write enabled.

### Evidence before a real product slice

- [ ] At least five independent release memory runs.
- [ ] At least 100 open/hide/show/close cycles without crashes.
- [ ] Median requested-to-ready no more than 750 ms.
- [ ] Worst five requested-to-ready samples no more than 1500 ms.
- [ ] Narrator announces search, buttons, list content, and status changes.
- [ ] Per-monitor DPI, IME, multiple monitors, and Windows 10/11 startup pass.

## Main-window parity matrix

WebView2 remains the production UI. This table is the first-slice contract for the WinUI
main window: what the React list does today, which C ABI / `tiez-core` seam the native
window should call, and which extraction phase owns it.

| WebView2 capability | Today's command / event | Native seam | Phase |
| --- | --- | --- | --- |
| Search by preview, source, type | `search_clipboard_history` / client `useFilteredHistory` | `tiez_core_get_snapshot_json` (`type:` prefix or free text) | 1 |
| Type chips (`text`, `image`, `url`, `code`, `files`) | header chips in `AppHeader.tsx` | same snapshot query (`type:<name>`) | 1 |
| Open details / full content | `get_clipboard_content` (unused by UI; paste loads by id) | `tiez_core_get_content_json` | 1 |
| Sensitive preview + metadata-only details | renderer blur + `sensitive`/`密码`/`password` | snapshot `is_sensitive` + redacted `HistoryContent` | 1 |
| Pin / unpin | `toggle_clipboard_pin` | `tiez_core_apply_action_json` `pin` (SQLite write adapter) | 1 |
| Delete | `delete_clipboard_entry` | `tiez_core_apply_action_json` `delete` | 1 |
| Paste plain (click / Enter) | `copy_to_clipboard` `paste: true`, `pasteWithFormat: false` | `PasteCoordinator` + `paste-plain` | 1 |
| Paste rich (right-click) | `copy_to_clipboard` `pasteWithFormat: true` | `PasteCoordinator` + `paste-rich` | 1 |
| Esc hide | `hide_window_cmd` | WinUI `UiLifecycle` hide | 1 |
| Keyboard up/down + Enter | `useKeyboardNavigation` / `navigation-action` | WinUI list selection | 1 |
| Toggle hotkey (default Alt+C) | `toggle_window_cmd` | WinUI `RegisterHotKey` + last HWND | 1 |
| Blur hide / window pin | `handle_window_event` / `set_window_pinned` | WinUI `Activated` + pin flag | 1 |
| Last-focus HWND for paste | `LAST_ACTIVE_HWND` / `restore_focus_before_paste` | recorded on hotkey-show, restored before paste | 1 |
| Live capture | clipboard listener + pipeline | later (phase 3); reuse existing Rust monitor | 3 |
| Item tags / tag search | `update_tags` | later C ABI | 4 |
| Pinned drag reorder | `update_pinned_order` | later | 4 |
| Compact preview window | `WebviewWindow` `compact-preview` | later native popup | 4 |
| Open URL/file | `open_content` | later | 4 |
| Settings, emoji, tag manager, file transfer, cloud, theme store, AI, OCR, updater | various commands | phase 5 independent WinUI surfaces | 5 |

Do not add Tauri `AppHandle` or `invoke` names to the C ABI. New behavior lands in
`tiez-core` first, then the WinUI transport crate, then XAML.

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
   **write** adapter now persists pin/delete for positive IDs when the probe is
   the only process (`replacement_id` stays null; session persist-on-pin is
   still Tauri-only);
3. `PasteCoordinator` — **extracted**: payload planning, hide/restore-focus/
   Ctrl+V contract, delete-after-paste intent, and a bounded paste-queue policy.
   WinUI executes Unicode paste on Windows; Tauri still wraps the existing
   Win32 clipboard/keystroke path after planning text payloads;
4. `UiLifecycle` — **WinUI minimum connected**: Alt+C toggle, Esc hide,
   deactivate hide unless pinned, last-foreground HWND for paste. Tray, close
   policy, and single-instance remain later.

Only after live clipboard capture (phase 3) should the WinUI executable become
a daily-driver alternative to WebView2.

## Primary references

- [WinUI 3](https://learn.microsoft.com/en-us/windows/apps/winui/winui3/)
- [Create a WinUI 3 app](https://learn.microsoft.com/en-us/windows/apps/get-started/start-here)
- [Distribute an unpackaged WinUI app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/unpackage-winui-app)
- [Windows App SDK runtime bootstrap](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/use-windows-app-sdk-run-time)
- [Official C++ unpackaged self-contained sample](https://github.com/microsoft/WindowsAppSDK-Samples/tree/main/Samples/SelfContainedDeployment/cpp/cpp-winui-unpackaged)
