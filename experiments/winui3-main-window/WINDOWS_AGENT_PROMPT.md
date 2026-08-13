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
Do not wire it to the real database, clipboard listener, global hotkeys, sync,
or updater in this task. Do not refactor production Rust modules unless a build
fix absolutely requires it, and report before making such a change.

Goals:

1. prove that the unpackaged C++/WinRT WinUI 3 app builds and launches;
2. prove that it loads `tiez_winui_core.dll` in-process through C ABI v1;
3. verify search, pin/unpin, delete, plain/rich paste requests, UTF-8, and the
   hide/show button;
4. collect release startup and memory evidence;
5. make only focused fixes inside the experiment directory;
6. produce a concise verdict: proceed to a real product slice, revise, or stop.

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

Required negative tests:

1. rename `tiez_winui_core.dll`, launch the EXE, and verify a visible startup
   error without a crash; restore the DLL afterward;
2. set `TIEZ_WINUI_CORE_DLL` to a nonexistent path and verify the same behavior;
3. type `中文`, `edge`, `image`, and a query with no results into search;
4. pin/unpin the same item repeatedly and verify generation increments;
5. delete one item and verify it remains deleted until the process exits;
6. activate **Hide for 5 seconds** and verify the exact same process and Rust
   state return.

Evidence to save under:

```text
experiments\winui3-main-window\artifacts\evidence\
```

Save:

- `environment.txt` with Windows build, Visual Studio, MSBuild, Rust, and CPU;
- full `build-release.log`;
- `measurement-run-01.json` through `measurement-run-05.json`;
- screenshots of loaded UI, UTF-8 search, missing-DLL error, and hidden/restored
  state;
- a 100-cycle lifecycle result or a clearly documented blocker;
- `verdict.md` with medians, worst-five startup values, private/working-set
  ranges, failures, and recommendation.

Do not claim this proves a full TieZ replacement. The probe uses synthetic Rust
data and does not include the production services. The decision question is
only whether C++/WinRT + WinUI 3 + an in-process Rust C interface is sufficiently
stable, fast, accessible, and maintainable to justify extracting the first real
Rust module.

Before committing, inspect the diff and ensure generated NuGet packages,
Visual Studio caches, build artifacts, logs, and screenshots are not committed.

---
