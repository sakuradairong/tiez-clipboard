# Prompt for the Windows agent

Copy the prompt below into an agent running on a Windows 11 x64 workstation.

---

You are validating the TieZ WinUI 3 main-window experiment on Windows.

Repository branch: the current WinUI migration branch

Experiment directory:

```text
experiments/winui3-main-window
```

This is a production-intended, mutually exclusive WinUI candidate. Release
builds use the Tauri-compatible data directory by default; synthetic and copied
read-only adapters remain available for diagnostics. Never run WinUI and Tauri
against the same live database. WebDAV setup, probing, and the background
cloud-sync runner are connected, but file transfer and some secondary surfaces
are not yet parity-complete. Do not make WinUI the default installed entry
during this validation until the signed upgrade and startup acceptance passes.

Goals:

1. prove that the unpackaged C++/WinRT WinUI 3 app builds and launches;
2. prove that it loads `tiez_winui_core.dll` in-process through C ABI v15;
3. verify Chinese-first search/details, protected history clearing, tags, pin ordering, capture, copy/paste,
   safe opening, settings, compact preview, Unicode and image Emoji favorites,
   the native tag catalog and exact tagged-entry workflow,
   backup/restore, OCR/QR analysis,
   WebDAV background synchronization, and Windows login startup;
4. verify the copied production-history adapter is genuinely read-only and the
   writable adapter remains schema/data compatible with Tauri;
5. verify sensitive/encrypted payloads and recognition results stay protected;
6. collect release startup and memory evidence for synthetic, copied read-only,
   and isolated writable adapters;
7. produce a concise verdict and list every remaining blocker before the native
   executable can become the installed default.

Prerequisites to confirm:

- Visual Studio 2022 with Desktop development with C++, Windows application
  development/Windows App SDK tools, MSVC v143, and Windows 11 SDK 10.0.26100;
- repository-pinned Rust `1.88.0-x86_64-pc-windows-msvc` toolchain;
- PowerShell execution policy bypassed for the current process.

Run:

```powershell
cd experiments\winui3-main-window
rustup toolchain install 1.88.0-x86_64-pc-windows-msvc
Set-ExecutionPolicy -Scope Process Bypass
.\build.ps1 -Configuration Release
.\test-hotkey.ps1 -Configuration Release
.\test-single-instance.ps1 -Configuration Release -LifecycleCycles 100
.\measure.ps1 -Configuration Release -SampleSeconds 30 -KeepOpen
```

Then manually verify the checklist in `README.md`.

For the real-history pass, stop TieZ and copy `clipboard.db` plus any sibling
`clipboard.db-wal` and `clipboard.db-shm` files to a scratch directory. Hash
the copied files before launch, then run:

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
$env:TIEZ_WINUI_DB_READ_ONLY = "1"
.\artifacts\x64\Release\TieZ.exe
```

Verify the badge says `sqlite-read-only`, newest persisted entries match TieZ,
search (including existing OCR/QR indexes) works, mutation/paste buttons are
disabled, and sensitive-tagged/encrypted previews show a sensitive-entry label.
Verify **Open details** displays full content for an ordinary entry while
sensitive/encrypted entries remain metadata-only. Close the probe and confirm
the copied files' hashes are
unchanged. Never use the live production database for this validation.

Required negative tests:

1. rename `tiez_winui_core.dll`, launch the EXE, and verify a visible startup
   error without a crash; restore the DLL afterward;
2. set `TIEZ_WINUI_CORE_DLL` to a nonexistent path and verify the same behavior;
3. type `中文`, `edge`, `image`, and a query with no results into search;
4. pin/unpin the same item repeatedly and verify generation increments and the
   structured Rust mutation message is shown without a JSON parsing error;
5. delete one item and verify it remains deleted until the process exits and
   the mutation result reports removal;
6. choose **清空历史**, cancel once, then confirm once and verify unpinned
   untagged entries are removed while pinned, tagged, and sensitive-protected
   entries remain; the action must be unavailable in copied read-only mode;
7. open **表情**, use Tab and Narrator to inspect multiple categories, then
   select a multi-code-point Emoji and verify it is pasted into the previous
   window without adding a synthetic or echo clipboard-history row;
8. in **表情**, import valid PNG/JPEG/GIF/WebP files (including a Chinese file
   name), verify they persist after restart, paste one into the previous window,
   remove one, and confirm only the managed copy is deleted. Verify spoofed and
   oversized images are rejected and copied read-only mode disables add/remove;
9. open **标签**, create a Chinese tag, change its color, add multiline text with
   leading/trailing whitespace, restart, and verify the catalog count, color,
   content, and exact tagged-entry list persist. Rename it onto an existing tag
   and confirm entry tags deduplicate; then use the explicit destructive action
   to permanently delete the tag's records and confirm tombstones/sync are
   produced. Verify `sensitive`, `密码`, and `password` cannot be renamed,
   deleted, or used by the manual-text action, sensitive previews remain
   redacted, copied read-only mode disables all mutations, and a result above
   1,000 entries reports both its total and cap;
10. activate **Hide for 5 seconds** and verify the exact same process and Rust
   state return.
11. set `TIEZ_WINUI_DB_PATH` to a nonexistent path and verify a visible startup
   error without a crash; clear it afterward.
12. open details for a synthetic entry, delete it, and verify the list refreshes
   without crashing or corrupting the native details panel.
13. verify the WebDAV password is never displayed, blank preserves it, explicit
   clear requires confirmation, non-loopback HTTP is rejected, and the test
   action reports a Chinese result without blocking the UI or writing remotely.
14. install the signed MSIX, verify “开机启动 TieZ” matches Task Manager, then
    sign out/in and confirm TieZ remains tray-only until the configured global
    shortcut (Alt+C by default) or the tray icon is used. Disable it in Windows
    settings and verify the native UI explains
    that only Windows can re-enable a user-disabled startup task.
15. with no TieZ process running, execute `test-single-instance.ps1` for 100
    lifecycle cycles; verify every real WM_CLOSE returns the same primary PID to
    the tray, the hidden duplicate exits without revealing the primary, the
    ordinary duplicate reveals the same primary PID, and both secondary exit
    codes are zero.
16. enable Narrator, Tab from the type filters into a clipboard record, and
    verify it is announced as a Chinese list item with pinned/sensitive state,
    type, source, time, and only a non-sensitive preview. Enter or Space must
    open details without pasting, the next Tab must reach the card's action
    buttons, Shift+F10 must open the Chinese card menu, and a sensitive item must
    announce “预览已隐藏” without exposing its preview value.
17. execute `test-hotkey.ps1` with no TieZ process running; verify the isolated
    Ctrl+Alt+F24 registration blocks a second owner, a real key injection reveals
    the hidden native window, and WM_CLOSE returns the same process to the tray.
    Verify the script then starts a separate `MouseMiddle` instance, real mouse
    input reveals it, and WM_CLOSE again returns the same process to the tray.
    Then edit the shortcut in Chinese native settings and verify registration
    succeeds before the new value is persisted, the main shell reports the active
    shortcut, and reopening settings shows the saved value. Switch keyboard →
    mouse middle → keyboard and verify each previous registration or hook is
    removed. Verify an invalid or conflicting value preserves both the previous
    working registration and saved
    value, an empty value disables and persists the shortcut, and a simulated
    database-write failure rolls the system registration back. Read-only adapters
    and `TIEZ_WINUI_HOTKEY` diagnostic overrides must disable the editor without
    exposing any secret setting.

Evidence to save under:

```text
experiments\winui3-main-window\artifacts\evidence\
```

Save:

- `environment.txt` with Windows build, Visual Studio, MSBuild, Rust, and CPU;
- full `build-release.log`;
- five synthetic-adapter and five copied-history measurement JSON files;
- screenshots of loaded UI, UTF-8 search, native full-content details,
  protected sensitive details, missing-DLL error, hidden/restored state,
  packaged startup state, and the `sqlite-read-only` badge;
- copied-database hashes before and after the read-only run;
- a 100-cycle lifecycle result or a clearly documented blocker;
- `verdict.md` with medians, worst-five startup values, private/working-set
  ranges, failures, and recommendation.

Do not claim this proves a full TieZ replacement. The current candidate already
owns substantial daily-use history, capture, privacy, lifecycle, backup,
image-analysis, WebDAV, and startup behavior, but signed upgrade/endurance
acceptance and the default entry switch remain unfinished. The decision question
is whether the verified native slice can proceed toward those remaining blockers
without regressing data compatibility, privacy, accessibility, or rollback safety.

Before committing, inspect the diff and ensure generated NuGet packages,
Visual Studio caches, build artifacts, logs, and screenshots are not committed.

---
