# TieZ WinUI 3 main-window migration slice

> **First production-intended native Windows slice — Tauri/WebView2 remains the
> fallback entry point.**

This experiment tests one question:

> Can a C++/WinRT WinUI 3 main window consume a narrow Rust C interface in the
> same process while providing enough clipboard-list interaction to justify a
> real Windows-native product slice?

The executable does **not yet** replace the Tauri window. It now starts a native
Windows clipboard listener for Unicode, HTML, image, and file payloads. The
reusable history, paste, privacy, and window-lifecycle policies
now live in the standalone, Tauri-independent `tiez-core` crate. Release builds
open the production TieZ data directory by default; Debug/test builds keep the
synthetic in-memory adapter. **Do not run this executable at the same time
as Tauri** against the live database. Both runtimes now acquire the same
per-database Windows ownership mutex before restore or SQLite open, so the
second process fails with a visible startup error instead of becoming a
concurrent writer. The existing WebView2 application remains the production fallback.
The production Tauri list, search, and full-content commands still use the same
storage-neutral merge/search policies through `TauriHistoryAdapter`; their
command names and serialized `ClipboardEntry` contract remain unchanged.

## Shape

```text
TieZ.exe
  WinUI 3 / XAML / C++/WinRT
        │ dynamic loading + C ABI v12
        ▼
tiez_winui_core.dll
  Rust C ABI transport adapter
        │
        ▼
tiez-core
  backup · clipboard_history · cloud_sync runner/SQLite host · image_analysis · content_opening · paste_coordinator · ui_lifecycle
    - production-schema SQLite history (Release default, writable unless TIEZ_WINUI_DB_READ_ONLY=1)
    - synthetic in-memory data (Debug/test default or explicit diagnostic override)
```

The C ABI interface is deliberately small:

- create/destroy one core handle;
- fetch a UTF-8 JSON snapshot for a search query;
- fetch full content metadata for one stable entry ID;
- prepare a validated URL or local-file launch plan without invoking a command shell;
- report the active adapter and whether it is read-only;
- apply `pin`, `delete`, `paste-plain`, or `paste-rich` (memory or writable SQLite);
- replace item tags from a UTF-8 JSON string array, including session-to-persisted ID replacement;
- replace the complete pinned order from a UTF-8 JSON integer array;
- read and update a strict allowlist of non-secret daily-use settings;
- read and transactionally update the existing WebDAV settings without returning
  its password, then run a read-only connectivity probe;
- start, request, stop, and poll the Rust-owned background WebDAV runner without
  returning its credentials or binding worker lifetime to a XAML object;
- create, fully validate, and schedule restoration of the same `.tiez-backup`
  archives as the Tauri fallback;
- return a structured mutation result with requested/effective/replacement IDs,
  removal state, generation, and a display message;
- retrieve per-thread errors and free returned strings.

The shared Rust module owns adapter selection, query filtering (including
`type:<content_type>` chips), sorting, sensitive-preview redaction, relative
timestamps, generation tracking, paste payload planning, tag privacy transitions,
pin/delete/tag/pinned-order writes, and the backup/rollback transaction.
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
- searchable tag chips and Chinese comma-separated tag editing in the details pane;
- pinned-card drag-and-drop plus Chinese “上移”/“下移” controls in an unfiltered writable view;
- real plain/rich paste through `PasteCoordinator` (Unicode, HTML, image, files);
- copy without paste (clipboard write only);
- keyboard up/down, Enter, Ctrl+Enter, Ctrl+C, Delete, Esc; IME Enter does not paste;
- card context menu and double-click paste;
- focusable clipboard list items with Chinese Narrator summaries, keyboard help, live status announcements, and protected sensitive-preview names;
- configured keyboard or `MouseMiddle`/`MButton` global toggle (inherited from and editable through `app.hotkey`, default Alt+C), last-foreground HWND capture, and deactivate-to-hide (unless pinned); the Chinese editor registers before persisting, database-write failure rolls registration back, and invalid/conflicting registrations keep the previous working shortcut and saved value;
- native TieZ system tray: left-click show, Chinese show/exit menu, close-to-hide, and Explorer restart recovery;
- process-wide AppLifecycle single-instancing before XAML or Rust/SQLite initialization: hidden startup redirects stay tray-only, while an ordinary second launch exits and reveals the existing native window;
- a Chinese native settings dialog for theme, compact list, global keyboard shortcut, persistence, limits, capture, privacy, tray, window pinning, and Windows login startup;
- packaged login startup through the MSIX `StartupTask` contract, plus a current-EXE HKCU Run fallback for unpackaged development; startup activation stays hidden in the tray and packaged ownership removes legacy Tauri Run values;
- immediate compact-card rendering, theme switching, tray visibility, and real always-on-top behavior without restarting;
- a no-activate native compact hover preview for text, rich text, files, local images, and protected-entry messages;
- Chinese “打开” actions for validated links/files and controlled temporary text, rich-text, or image files;
- asynchronous Chinese backup export and restore controls with native file dialogs,
  archive/checksum validation, next-startup restore, and seven-day rollback retention;
- asynchronous OCR/QR recognition with cached native search and sensitive-result protection;
- Chinese WebDAV setup with write-only password handling, compatible settings,
  a read-only `PROPFIND` connectivity test, automatic background scheduling,
  immediate synchronization, and live sanitized status;
- a Chinese App Installer update check/install entry that uses the feed associated with the installed MSIX;
- UTF-8 text, including Chinese and emoji;
- a five-second hide/show lifecycle action for memory measurements;
- an optional ready marker for startup and memory measurement.

Paste uses the shared coordinator. On Windows the probe writes CF_UNICODETEXT and,
for paste-rich when HTML is available, CF_HTML (`HTML Format`). Image entries with a
local path or `data:image/` payload write CF_DIB (and PNG when the source is PNG).
File lists write CF_HDROP. Ctrl+V is sent after hiding and restoring the last
foreground window. Synthetic image/file placeholders are not pasteable; use a real
`clipboard.db` to try those types. Writable SQLite mode decrypts protected payloads
only inside the Rust core for approved copy/paste operations; native cards and the
details pane remain redacted. Read-only inspection never exposes protected payloads.
Do not start Tauri and this executable against the same `clipboard.db` at once.

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
established toolchain before exploring a major SDK upgrade. 1.8 ships WinUI,
Runtime, and MSIX/MRT build assets as split packages; `packages.config`
restores those explicitly so XBF compilation and app PRI generation both run
in command-line builds. Because WinUI's metadata declares `IWebView2`, cppwinrt
also needs the WebView2 WinMD while generating projections. That reference is
compile-time-only (`Private=false`): the project does not import WebView2 build
targets, link its loader, use a WebView control, or ship WebView2 runtime files.

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
complete `TieZ.exe` still requires `build.ps1` on Windows; the
`winui3-probe` GitHub Actions workflow provides the reproducible remote build.

## Build and run

From this directory in PowerShell:

```powershell
rustup toolchain install 1.88.0-x86_64-pc-windows-msvc
Set-ExecutionPolicy -Scope Process Bypass
.\build.ps1 -Configuration Release
.\artifacts\x64\Release\TieZ.exe
```

`build.ps1` performs these steps:

1. runs the shared `tiez-core` and WinUI C ABI tests;
2. builds the `cdylib` in release mode;
3. restores the native NuGet packages into the experiment directory;
4. resolves Visual Studio's MSBuild with `vswhere`;
5. builds the unpackaged, self-contained WinUI executable and current app PRI;
6. stamps `TieZ.exe` with the synchronized product version and places it beside
   the Rust DLL. Release builds enable the production-data adapter by default.

### Package, sign, install, and upgrade

Create and re-open an unsigned MSIX for local/CI structural validation:

```powershell
.\package-msix.ps1 -SkipBuild
```

Unsigned packages are never accepted by the release job. A distributable build
must use a code-signing certificate whose subject exactly matches `Publisher`,
plus an RFC 3161 timestamp service:

```powershell
.\package-msix.ps1 `
  -SkipBuild `
  -Publisher "CN=Your Publisher" `
  -CertificatePath "C:\secure\tiez-release.pfx" `
  -CertificatePassword $env:TIEZ_CERT_PASSWORD `
  -TimestampUrl "https://your-controlled-timestamp-service.example" `
  -AppInstallerBaseUri "https://github.com/OWNER/REPO/releases/latest/download" `
  -RequireSigning
```

The package script reads the product version from the repository-wide version
gate, generates `TieZ_<version>.0_x64.msix`, re-opens it with `MakeAppx`, checks
the package identity, native `TieZStartup` login task, and required runtime files,
rejects WebView2 payloads, and writes SHA256. The manifest deliberately disables
AppData write virtualization,
and the package validator requires that declaration plus the corresponding
`unvirtualizedResources` capability. This keeps the installed WinUI app and the
unpackaged Tauri fallback on the same `%APPDATA%\com.tiez` database instead of
silently creating package-private history. When a base URI is supplied it also creates
`TieZ-x64.appinstaller`; Windows checks that stable file on launch and in the
background, and only accepts a package with the same identity/publisher and a
higher four-part version.

`unvirtualizedResources` is a restricted capability. A signed package may be
sideloaded without Microsoft approval, but a future Microsoft Store submission
must justify the capability and pass Store review. Keep the GitHub release in
draft until `TieZ-x64.appinstaller` has been installed and upgraded on both
Windows 10 and Windows 11 with the release certificate.

The release workflow requires `WINUI_MSIX_CERTIFICATE_BASE64`,
`WINUI_MSIX_CERTIFICATE_PASSWORD`, and `WINUI_MSIX_PUBLISHER` secrets plus a
`WINUI_MSIX_TIMESTAMP_URL` repository variable. The decoded PFX exists only in
the runner temporary directory and is removed in an `always()` cleanup step.

Override the Rust DLL path for debugging with:

```powershell
$env:TIEZ_WINUI_CORE_DLL = "C:\path\to\tiez_winui_core.dll"
```

### Read or write TieZ history

Release uses the real writable database by default, so **stop Tauri first**.
Debug and Rust test builds remain synthetic. The installed Windows database is
normally under TieZ's
app-data directory; portable mode stores it under `data\clipboard.db` beside
the executable, and `datapath.txt` may redirect the directory.

Writable (WinUI as the only process — pin/delete/tag/order changes persist):

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
.\artifacts\x64\Release\TieZ.exe
```

To exercise the same production data-directory selection as Tauri without
copying a path, explicitly enable production data mode:

```powershell
$env:TIEZ_WINUI_USE_PRODUCTION_DATA = "1"
.\artifacts\x64\Release\TieZ.exe
```

This resolves `%APPDATA%\com.tiez`, honors a valid `datapath.txt`, and lets an
existing `data` directory beside the WinUI executable take precedence. The
shared ownership mutex still requires Tauri to be fully stopped. Release already
enables this policy; the flag remains useful for Debug builds. For an isolated
Release demonstration, set `TIEZ_WINUI_USE_SYNTHETIC_DATA=1` before launch.
In writable mode the shared Rust bootstrap creates or upgrades schema version
15 and seeds the same defaults as Tauri. If a scheduled restore is pending,
WinUI now validates and applies it itself before opening SQLite. Invalid pending
archives are quarantined, and a successful restore keeps the replaced data in a
seven-day rollback directory; starting the Tauri fallback is no longer required.

The header should show `sqlite` and **write enabled**. Pin/delete go through
`tiez_core_apply_action_json`; tag edits go through `tiez_core_update_tags_json`.
Pinned-order edits go through `tiez_core_update_pinned_order_json` and submit
the complete visible pinned ID list. Reordering is disabled while search or a
type filter is active; the Rust core atomically rejects stale or partial lists.
Persisted positive IDs stay stable. Tagging or pinning a WinUI session-only
negative ID persists it and returns the new positive `replacement_id`.

Read-only inspection of a copied database (byte-identical check):

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
$env:TIEZ_WINUI_DB_READ_ONLY = "1"
.\artifacts\x64\Release\TieZ.exe
```

The header and status should show `sqlite-read-only`. The adapter reads at
most 200 latest entries, supports case-insensitive search over preview, source
app, content type, and tags, plus exact `type:<name>` filters. It deliberately
does not:

- read session-only negative-ID entries held in the running Tauri process;
- display sensitive-tagged or `dpapi:` previews (they are replaced with a
  sensitive-entry label);
- expose sensitive or encrypted payloads in read-only mode;
- perform mutation/paste operations when `TIEZ_WINUI_DB_READ_ONLY=1`.

Use **Open details** to read the full persisted payload for non-sensitive
entries. The details panel keeps sensitive and encrypted entries metadata-only.

Unset the variables to return to synthetic mode:

```powershell
Remove-Item Env:TIEZ_WINUI_DB_PATH -ErrorAction SilentlyContinue
Remove-Item Env:TIEZ_WINUI_DB_READ_ONLY -ErrorAction SilentlyContinue
Remove-Item Env:TIEZ_WINUI_USE_PRODUCTION_DATA -ErrorAction SilentlyContinue
Remove-Item Env:TIEZ_WINUI_USE_SYNTHETIC_DATA -ErrorAction SilentlyContinue
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

Live capture is always on for Unicode text. CF_HTML and Explorer files follow
the native capture settings; images remain supported by the format pipeline
(consecutive-copy dedup, paste-echo skip, configured privacy detection).
Do not point
`TIEZ_WINUI_DB_PATH` at the live production `clipboard.db` while Tauri TieZ is
running. OCR/QR analysis and native search are connected through the shared
core. WebDAV configuration, safe connectivity testing, redirect-free transport
(retry, atomic publication, blobs, and remote listings), conflict rules, and the
single-pass runner are shared. ABI v12 owns the native scheduler and writable
SQLite host; C++ only starts/stops it and polls credential-free status.
Sensitive or encrypted entries never retain
plaintext analysis, including when a privacy tag is added during background
recognition. Record the active adapter with every result.

## Acceptance checklist

### Build and ABI

- [ ] Release build succeeds from a fresh NuGet cache.
- [ ] `tiez_winui_core.dll` loads without changing `PATH`.
- [ ] Status shows `Rust ABI 12`.
- [ ] The Release directory and EXE import table contain no WebView2 files or loader dependency.
- [ ] `TieZ.exe` file/product version matches all eight release-version sources.
- [ ] Unsigned MSIX validation packs and re-opens successfully but is never uploaded as a release.
- [ ] The packed manifest disables AppData write virtualization and declares `unvirtualizedResources`.
- [ ] The packed manifest contains the enabled `TieZStartup` task targeting `TieZ.exe`, and an installed login activation starts tray-only without flashing the main window.
- [ ] `test-single-instance.ps1 -Configuration Release -LifecycleCycles 100` passes against an isolated temporary database.
- [ ] `test-hotkey.ps1 -Configuration Release` proves an isolated keyboard shortcut owns its registration, then independently proves real keyboard and mouse-middle input activate the hidden window and survive close-to-tray.
- [ ] The signed MSIX Publisher matches the certificate subject and `signtool verify /pa` succeeds.
- [ ] Installing `TieZ-x64.appinstaller` creates the Start-menu entry; a higher four-part version upgrades in place without losing `%APPDATA%\com.tiez` data.
- [ ] In an App Installer-based installation, “检查更新” reports the current availability and “安装更新” opens only the associated HTTPS feed; unpackaged builds show a local explanatory error instead of using a fallback endpoint.
- [ ] Chinese and emoji render without replacement characters.
- [ ] Missing/wrong DLL produces a visible startup error instead of a crash.

### Interaction

- [ ] Search filters by preview, source app, and content type.
- [ ] Type chips send `type:text` / `type:image` / `type:url` / `type:code` / `type:file`.
- [ ] Pin/unpin changes the card and generation.
- [ ] Delete removes an entry.
- [ ] Mutation status shows the Rust result message and generation.
- [ ] Plain/rich paste writes the system clipboard and sends Ctrl+V after restoring the last HWND.
- [ ] Image paste writes CF_DIB (and PNG when applicable); file paste writes CF_HDROP.
- [ ] Enter in the search box does not paste while an IME composition is being confirmed.
- [ ] Up/Down moves the selected card; Enter pastes plain text; Ctrl+Enter pastes rich; Esc hides.
- [ ] Ctrl+C copies without injecting Ctrl+V; Delete removes the selected card when search is not focused.
- [ ] Right-click a card for open/paste/copy/pin/delete; double-click pastes plain text.
- [ ] The configured `app.hotkey` (Alt+C by default, with `MouseMiddle`/`MButton` compatibility) toggles visibility and captures the last foreground window; the Chinese shell displays the active input method.
- [ ] Editing the shortcut in Chinese native settings registers it before saving it; reopening settings shows the saved value and the main shell immediately shows the active value.
- [ ] Switching keyboard → mouse middle → keyboard removes the previous registration or hook each time; mouse middle is consumed only while configured and teardown leaves no hook behind.
- [ ] An invalid or already-registered shortcut leaves both the previous working registration and saved value unchanged; an empty setting disables and persists the shortcut without affecting tray access.
- [ ] A simulated database-write failure restores the previous system registration and reports a Chinese error; read-only adapters and `TIEZ_WINUI_HOTKEY` diagnostic overrides disable the editor.
- [ ] The TieZ tray icon is registered; left-click shows the main window without replacing the saved paste target.
- [ ] Closing the main window hides it while the process stays alive; the tray “退出 TieZ” command exits.
- [ ] An ordinary second launch exits with code 0 and reveals the existing native window without constructing another Rust/SQLite owner.
- [ ] A `--autostart` or `--minimized` second launch exits with code 0 without revealing a hidden primary window.
- [ ] Right-clicking the tray icon shows the Chinese “显示主界面” and “退出 TieZ” commands.
- [ ] Restarting Explorer restores the tray icon.
- [ ] Sensitive cards show a redacted preview and a sensitive label.
- [ ] Open details displays full UTF-8 content without WebView2.
- [ ] Tab reaches each clipboard record as a focusable list item, then reaches its visible action buttons; the focused record has a visible focus indicator.
- [ ] Narrator announces each record's pinned/sensitive state, type, source, time, and non-sensitive preview; sensitive records announce only “预览已隐藏”.
- [ ] Enter or Space on a focused record opens its details without pasting, and Shift+F10 opens the same Chinese card menu as right-click.
- [ ] Text selection still works inside card previews and the details pane.
- [ ] Copying text in another app prepends a new card without restarting the probe.
- [ ] Copying formatted HTML, an image, or Explorer files prepends the matching card type.
- [ ] The clipboard already present at startup is not ingested.
- [ ] Pasting from the probe does not create a duplicate card.
- [ ] Leading/trailing whitespace and internal newlines are preserved.
- [ ] Tag chips render, tag search finds matching items, and Chinese comma-separated edits persist.
- [ ] Adding `sensitive`, `密码`, or `password` redacts the item; removing the last sensitive tag restores access.
- [ ] Tagging a negative session ID follows the positive replacement ID without losing selection.
- [ ] Pinned cards reorder by drag-and-drop and by “上移”/“下移”, then retain that order after restart.
- [ ] Pinned reordering is unavailable in searched, type-filtered, or read-only views.
- [ ] The Chinese settings dialog reads existing TieZ values and never exposes secret keys.
- [ ] Theme, compact list, tray visibility, and window pinning apply immediately and persist after restart.
- [ ] “开机启动 TieZ” reflects the actual Windows state; disabling it persists locally, user/policy-disabled states explain where to re-enable it, and the setting never crosses WebDAV.
- [ ] File/rich-text capture, persistence, deduplication, limits, and privacy changes affect subsequent captures.
- [ ] The Chinese WebDAV section reads the existing Tauri-compatible URL, username, path, intervals, and content preferences without ever displaying the saved password.
- [ ] Saving WebDAV settings is transactional; leaving the password blank preserves it, while the explicit confirmed clear action removes it.
- [ ] The WebDAV connectivity action runs off the UI thread, sends only a read-only `PROPFIND`, accepts HTTPS (plus loopback HTTP for local testing), rejects embedded credentials and redirects, and reports Chinese success/authentication/error state.
- [ ] Enabling automatic WebDAV sync starts the Rust-owned scheduler; “立即同步” works with automatic sync disabled and shows Chinese running/result/error state.
- [ ] A second device receives text, image, file metadata, tags, settings, and emoji favorites; newer revisions win deterministically and deletions do not echo back.
- [ ] Remote setting changes apply to the native window without restart, while MQTT/AI/WebDAV credentials, relay keys, and local runner state never appear in settings snapshots.
- [ ] Hovering a compact card shows the native always-on-top preview without stealing focus; leaving the card or hiding TieZ closes it.
- [ ] An ordinary image can run OCR/QR recognition without blocking the UI, shows Chinese progress/error state, and copies the combined result.
- [ ] Reopening an analyzed image uses the cache, “重新识别” forces a refresh, and searching recognized OCR text or a QR payload finds the image card without exposing that payload in its preview.
- [ ] Sensitive/encrypted images expose no recognition action or cached plaintext; adding a privacy tag during recognition leaves no persisted analysis row.
- [ ] HTTP/HTTPS and existing files open through the Windows default handler without `cmd` or PowerShell.
- [ ] Custom URL protocols and local rich-text HTML require Chinese confirmation; dangerous URL protocols and sensitive entries are rejected.
- [ ] Text and embedded images open from uniquely named files under the TieZ temporary directory without changing the stored clipboard entry.
- [ ] The Chinese settings dialog exports a backup without blocking the UI and reports version, record count, file count, and byte size.
- [ ] Restore rejects a malformed or checksum-mismatched archive before changing current data.
- [ ] A scheduled restore applies on the next WinUI startup before SQLite opens, removes the pending archive, and keeps a rollback directory.
- [ ] Read-only production mode permits export but disables restore; synthetic mode disables both actions.
- [ ] The UI warns that backup archives are not additionally encrypted and DPAPI-protected fields remain bound to the current Windows account.

### Copied production history

- [ ] `TIEZ_WINUI_DB_PATH` with `TIEZ_WINUI_DB_READ_ONLY=1` switches the badge to `sqlite-read-only`.
- [ ] The newest persisted items match the production TieZ history ordering.
- [ ] Search matches preview, source app, content type, cached OCR text, and QR payloads without writing.
- [ ] Mutation/paste/copy buttons are disabled in read-only mode; details and safe opening remain available for non-sensitive content.
- [ ] Sensitive-tagged and encrypted previews show the sensitive-entry label.
- [ ] Sensitive-tagged and encrypted details remain metadata-only.
- [ ] The copied database and optional WAL/SHM files remain byte-identical.
- [ ] Without `TIEZ_WINUI_DB_READ_ONLY`, pin/delete persist and the badge shows write enabled.

### Evidence before a real product slice

- [ ] At least five independent release memory runs.
- [ ] `test-single-instance.ps1 -Configuration Release -LifecycleCycles 100` completes 100 show/WM_CLOSE-to-tray cycles without changing the primary PID or exiting the Rust owner.
- [ ] `test-hotkey.ps1 -Configuration Release` completes without touching the production database or the user's configured shortcut.
- [ ] Median requested-to-ready no more than 750 ms.
- [ ] Worst five requested-to-ready samples no more than 1500 ms.
- [ ] Narrator announces the Chinese search, buttons, focusable clipboard list items, help text, empty state, image-analysis status, and global status changes.
- [ ] Per-monitor DPI, IME, multiple monitors, and Windows 10/11 startup pass.

## Main-window parity matrix

WebView2 remains the production UI. This table is the first-slice contract for the WinUI
main window: what the React list does today, which C ABI / `tiez-core` seam the native
window should call, and which extraction phase owns it.

| WebView2 capability | Today's command / event | Native seam | Phase |
| --- | --- | --- | --- |
| Search by preview, source, type | `search_clipboard_history` / client `useFilteredHistory` | `tiez_core_get_snapshot_json` (`type:` prefix or free text) | 1 |
| Type chips (`text`, `image`, `url`, `code`, `file`) | header chips in `AppHeader.tsx` | same snapshot query (`type:<name>`) | 1 |
| Open details / full content | `get_clipboard_content` (unused by UI; paste loads by id) | `tiez_core_get_content_json` | 1 |
| Sensitive preview + metadata-only details | renderer blur + `sensitive`/`密码`/`password` | snapshot `is_sensitive` + redacted `HistoryContent` | 1 |
| Pin / unpin | `toggle_clipboard_pin` | `tiez_core_apply_action_json` `pin` (SQLite write adapter) | 1 |
| Delete | `delete_clipboard_entry` | `tiez_core_apply_action_json` `delete` | 1 |
| Paste plain (click / Enter) | `copy_to_clipboard` `paste: true`, `pasteWithFormat: false` | `PasteCoordinator` + `paste-plain` | 1 |
| Paste rich (right-click) | `copy_to_clipboard` `pasteWithFormat: true` | `PasteCoordinator` + `paste-rich` | 1 |
| Esc hide | `hide_window_cmd` | WinUI `UiLifecycle` hide | 1 |
| Keyboard up/down + Enter | `useKeyboardNavigation` / `navigation-action` | WinUI list selection | 1 |
| Configured keyboard/mouse-middle toggle (default Alt+C) | `toggle_window_cmd` + `app.hotkey` | allowlisted native settings read/write + parsed WinUI `RegisterHotKey` or scoped `WH_MOUSE_LL`, registration-first persistence, rollback, teardown, and last HWND | 5 (connected) |
| Blur hide / window pin | `handle_window_event` / `set_window_pinned` | WinUI `Activated` + pin flag | 1 |
| System tray / close-to-hide / explicit exit | `setup_tray` / `CloseRequested` | `Shell_NotifyIconW` + native `WM_CLOSE` policy | 4 |
| Second launch / tray wake | single-instance plugin + window commands | AppLifecycle key registration and activation redirection before XAML/Rust startup | 5 (connected) |
| Last-focus HWND for paste | `LAST_ACTIVE_HWND` / `restore_focus_before_paste` | recorded on hotkey-show, restored before paste | 1 |
| Live capture (Unicode, HTML, image, files) | clipboard listener + pipeline | `CaptureFilter` + `tiez_core_start_capture`; privacy and OCR/QR connected, cloud later | 3 |
| Item tags / tag search | `update_tags` | `tiez_core_update_tags_json` + secure SQLite transition | 4 |
| Pinned drag reorder | `update_pinned_order` | `tiez_core_update_pinned_order_json` + atomic complete-set validation | 4 |
| Compact preview window | `WebviewWindow` `compact-preview` | no-activate native WinUI/Win32 popup | 4 |
| Open URL/file/text/rich/image | `open_content` | `tiez_core_prepare_open_content_json` + native `ShellExecuteW`, without command shells | 4 |
| Daily native settings | `get_all_settings` / `save_setting` | `tiez_core_get_settings_json` / `tiez_core_update_setting_json` allowlist + Chinese dialog | 4 |
| Windows login startup / silent activation | Tauri Run value + minimized argument | MSIX `StartupTask` + AppLifecycle activation, with current-EXE Run fallback for unpackaged development | 5 (connected) |
| Backup / inspect / restore | `create_backup` / `inspect_backup` / `schedule_backup_restore` | shared `tiez-core::backup` + current ABI + async native file dialogs and startup restore | 4 |
| Image OCR / QR analysis and search | `get_image_analysis` / `analyze_image_entry` | shared `tiez-core::image_analysis` + current ABI + async Chinese details panel and cached-index search | 5 (connected) |
| WebDAV settings / connectivity | `get_all_settings` / `save_setting` / provider test | shared `tiez-core::cloud_sync_settings` + ABI 12 + write-only secret and read-only `PROPFIND` | 5 (connected) |
| WebDAV transport | request retry, path/layout, atomic PUT/MOVE, blobs, remote listing | shared `tiez-core::cloud_sync_webdav`; Tauri uses compatibility wrappers and WinUI can reuse it directly | 5 (extracted) |
| Cloud-sync wire model / conflict identity | local item, snapshot, op/head structs and digest/revision rules | shared `tiez-core::cloud_sync_protocol`; existing snake_case JSON and whitespace-preserving hash versions remain stable | 5 (extracted) |
| Cloud-sync runner host boundary | Tauri `AppHandle`, repositories, settings, emoji and events | shared `tiez-core::cloud_sync_runner::CloudSyncHost`; typed runtime state/events and bounded remote planning, with no window handle in the port | 5 (defined) |
| Background cloud upload/download and conflict reconciliation | cloud-sync service and mutation distribution | shared `run_webdav_once` plus `SqliteCloudSyncHost`; WinUI ABI v12 owns scheduling/cancellation/status and Tauri retains a local-only legacy escape hatch | 5 (connected) |
| Check/install application update | Tauri updater plugin | packaged `PackageManager` availability check + associated HTTPS App Installer feed | 5 (connected) |
| Emoji, tag manager, file transfer, advanced theme store, AI | various commands | phase 5 independent WinUI surfaces | 5 |

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
   **write** adapter now persists pin/delete/tag/pinned-order changes when the probe is the
   only process. Pinning or tagging session entries returns the positive
   replacement ID, while sensitive tag changes synchronously protect storage;
3. `PasteCoordinator` — **extracted**: payload planning, hide/restore-focus/
   Ctrl+V contract, delete-after-paste intent, and a bounded paste-queue policy.
   WinUI executes Unicode, CF_HTML, CF_DIB/PNG, and CF_HDROP paste on Windows;
   Tauri still wraps the existing Win32 clipboard/keystroke path after planning
   text payloads;
4. `UiLifecycle` — **WinUI daily shell connected**: inherited and natively editable keyboard/mouse-middle toggle, Esc hide,
   deactivate hide unless pinned, last-foreground HWND for paste, native tray,
   close-to-hide, explicit tray exit, Explorer restart recovery, AppLifecycle
   launch redirection, and shared per-database single-instance ownership;
5. `ClipboardCapture` — **WinUI Unicode/HTML/image/file connected**: format
   priority (files, rich text, image, text), CRLF normalization without trim,
   consecutive-copy dedup, self-paste echo skip, and a `WM_CLIPBOARDUPDATE`
   worker that never reads the clipboard in WndProc. Configured privacy tagging
   and protected persistence are connected; cloud distribution remains later;
6. `ContentOpening` — **WinUI native shell connected**: the shared core rejects
   sensitive/unavailable payloads and dangerous URL protocols, normalizes web
   links, resolves existing files, and creates unique UTF-8/HTML/image temporary
   files. WinUI confirms custom protocols or local HTML, then calls
   `ShellExecuteW` directly without `cmd` or PowerShell. Editing temporary files
   back into history remains a later parity item;
7. `BackupRestore` — **shared core and WinUI surface connected**: Tauri command
   wrappers and the current WinUI ABI use one manifest/checksum implementation.
   Native export and restore run off the UI thread; restore is staged, revalidated,
   applied before SQLite opens, and retains a rollback directory;
8. `ImageAnalysis` — **shared core and WinUI surface connected**: Windows OCR
   and QR decoding run off the UI thread, Tauri keeps its command contract,
   cached text is searchable from native snapshots, and sensitive/encrypted
   results are memory-only with a pre-write privacy recheck.
9. `CloudSync` — **shared core, Tauri adapter, and WinUI runtime connected**: ABI v12
   reads and transactionally writes the existing WebDAV keys, keeps passwords
   write-only, validates HTTPS and safe remote paths, and runs a redirect-free
   read-only `PROPFIND` off the UI thread. `CloudSyncWebDav` now also owns the
   reusable retry, safe path/layout, atomic PUT/MOVE, blob, JSON, and listing
   transport contract, while `CloudSyncProtocol` owns the compatible item,
   snapshot, op/head, digest, and deterministic revision-collapse model.
   `SqliteCloudSyncHost` applies remote revisions, tombstones, tags, sensitive
   DPAPI storage, settings, attachments, and emoji materialization. It inlines
   local rich-HTML image resources before upload and removes tombstoned managed
   attachments only after verifying that clipboard history and emoji favorites
   no longer reference them. The Rust ABI owns the periodic/manual scheduler,
   cancellation, worker teardown, and credential-free status; WinUI renders it
   in Chinese. Real-account endurance and multi-device installer testing remain
   required before default cutover.

Before the WinUI executable becomes the default daily-driver entry, it still
needs signed installer upgrade, real-account multi-device sync endurance, and
manual Windows 10/11, DPI, IME, accessibility, and long-run lifecycle acceptance.

## Primary references

- [WinUI 3](https://learn.microsoft.com/en-us/windows/apps/winui/winui3/)
- [Create a WinUI 3 app](https://learn.microsoft.com/en-us/windows/apps/get-started/start-here)
- [Distribute an unpackaged WinUI app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/unpackage-winui-app)
- [Windows App SDK runtime bootstrap](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/use-windows-app-sdk-run-time)
- [AppLifecycle rich activation](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/applifecycle/applifecycle-rich-activation)
- [App instancing with AppLifecycle](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/applifecycle/applifecycle-instancing)
- [Desktop startup-task manifest extension](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions)
- [Official C++ unpackaged self-contained sample](https://github.com/microsoft/WindowsAppSDK-Samples/tree/main/Samples/SelfContainedDeployment/cpp/cpp-winui-unpackaged)
