# Prompt for the Windows agent

Copy the prompt below into an agent running on a Windows 11 x64 workstation.

---

You are validating the TieZ WinUI 3 main-window experiment on Windows.

Repository branch: `explore/winui3-main-window`

Experiment directory:

```text
experiments/winui3-main-window
```

The prototype is deliberately isolated from the production Tauri application.
It now has an opt-in SQLite adapter that reads a **copied** TieZ database with
read-only open flags. Do not point it at live production files or wire it to
the clipboard listener, global hotkeys, sync, or updater. Do not refactor
production Rust modules unless a build fix absolutely requires it, and report
before making such a change.

Goals:

1. prove that the unpackaged C++/WinRT WinUI 3 app builds and launches;
2. prove that it loads `tiez_winui_core.dll` in-process through C ABI v3;
3. verify search, full-content details, pin/unpin, delete, plain/rich paste
   requests, UTF-8, and the hide/show button;
4. verify the copied production-history adapter is genuinely read-only;
5. collect release startup and memory evidence for both adapters;
6. make only focused fixes inside the experiment directory;
7. produce a concise verdict: proceed to a real product slice, revise, or stop.

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
.\measure.ps1 -Configuration Release -SampleSeconds 30 -KeepOpen
```

Then manually verify the checklist in `README.md`.

For the real-history pass, stop TieZ and copy `clipboard.db` plus any sibling
`clipboard.db-wal` and `clipboard.db-shm` files to a scratch directory. Hash
the copied files before launch, then run:

```powershell
$env:TIEZ_WINUI_DB_PATH = "C:\scratch\tiez-history\clipboard.db"
.\artifacts\x64\Release\TieZ.exe
```

Verify the badge says `sqlite-read-only`, newest persisted entries match TieZ,
search works, all action buttons are disabled, and sensitive-tagged/encrypted
previews show a sensitive-entry label. Verify **Open details** displays full
content for an ordinary entry while sensitive/encrypted entries remain
metadata-only. Close the probe and confirm the copied files' hashes are
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
6. activate **Hide for 5 seconds** and verify the exact same process and Rust
   state return.
7. set `TIEZ_WINUI_DB_PATH` to a nonexistent path and verify a visible startup
   error without a crash; clear it afterward.
8. open details for a synthetic entry, delete it, and verify the list refreshes
   without crashing or corrupting the native details panel.

Evidence to save under:

```text
experiments\winui3-main-window\artifacts\evidence\
```

Save:

- `environment.txt` with Windows build, Visual Studio, MSBuild, Rust, and CPU;
- full `build-release.log`;
- five synthetic-adapter and five copied-history measurement JSON files;
- screenshots of loaded UI, UTF-8 search, native full-content details,
  protected sensitive details, missing-DLL error, and hidden/restored state,
  plus the `sqlite-read-only` badge;
- copied-database hashes before and after the read-only run;
- a 100-cycle lifecycle result or a clearly documented blocker;
- `verdict.md` with medians, worst-five startup values, private/working-set
  ranges, failures, and recommendation.

Do not claim this proves a full TieZ replacement. The copied-history adapter
only reads persisted previews/content and does not include production services,
decryption, or session-only entries. The decision question is only whether C++/WinRT + WinUI
3 + an in-process Rust C interface is sufficiently stable, fast, accessible,
and maintainable to justify extracting the first real Rust module.

Before committing, inspect the diff and ensure generated NuGet packages,
Visual Studio caches, build artifacts, logs, and screenshots are not committed.

---
