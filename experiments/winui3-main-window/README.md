# TieZ WinUI 3 main-window experiment

> **Throwaway experiment — not a production entry point.**

This experiment tests one question:

> Can a C++/WinRT WinUI 3 main window consume a narrow Rust C interface in the
> same process while providing enough clipboard-list interaction to justify a
> real Windows-native product slice?

It does **not** replace the Tauri window, start the real clipboard listener, or
open the production database. The existing application remains untouched.

## Shape

```text
Tiez.WinUIProbe.exe
  WinUI 3 / XAML / C++/WinRT
        │ dynamic loading + C ABI v1
        ▼
tiez_winui_core.dll
  Rust cdylib with an in-memory clipboard adapter
```

The Rust interface is deliberately small:

- create/destroy one core handle;
- fetch a UTF-8 JSON snapshot for a search query;
- apply `pin`, `delete`, `paste-plain`, or `paste-rich`;
- retrieve per-thread errors and free returned strings.

The JSON format is only a prototype transport. A production seam should use
versioned request/response structs or another explicitly versioned wire format.

## What the UI demonstrates

- native WinUI 3 cards and controls;
- search driven by the Rust snapshot;
- pin/unpin and delete state mutations in Rust;
- plain/rich paste requests crossing the C interface;
- UTF-8 text, including Chinese and emoji;
- a five-second hide/show lifecycle action;
- an optional ready marker for startup and memory measurement.

Paste buttons intentionally record requests instead of changing the system
clipboard. Connecting them to TieZ's real paste implementation requires first
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

## Build and run

From this directory in PowerShell:

```powershell
rustup toolchain install 1.88.0-x86_64-pc-windows-msvc
Set-ExecutionPolicy -Scope Process Bypass
.\build.ps1 -Configuration Release
.\artifacts\x64\Release\Tiez.WinUIProbe.exe
```

`build.ps1` performs these steps:

1. runs the Rust core tests;
2. builds the `cdylib` in release mode;
3. restores the native NuGet packages into the experiment directory;
4. resolves Visual Studio's MSBuild with `vswhere`;
5. builds the unpackaged, self-contained WinUI executable;
6. places the executable and Rust DLL in the same artifact directory.

Override the Rust DLL path for debugging with:

```powershell
$env:TIEZ_WINUI_CORE_DLL = "C:\path\to\tiez_winui_core.dll"
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

This result is not comparable to TieZ until both executables use the same
dataset and backend capability. The probe currently measures only WinUI plus
the in-memory Rust adapter.

## Acceptance checklist

### Build and ABI

- [ ] Release build succeeds from a fresh NuGet cache.
- [ ] `tiez_winui_core.dll` loads without changing `PATH`.
- [ ] Status shows `Rust ABI 1`.
- [ ] Chinese and emoji render without replacement characters.
- [ ] Missing/wrong DLL produces a visible startup error instead of a crash.

### Interaction

- [ ] Search filters by preview, source app, and content type.
- [ ] Pin/unpin changes the card and generation.
- [ ] Delete removes an entry.
- [ ] Plain/rich paste buttons update the Rust action status.
- [ ] Keyboard Tab traversal and text selection work.
- [ ] Hide/show restores a focused, usable window.

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

1. `ClipboardHistory` — search/list/content retrieval with stable IDs;
2. `ClipboardMutation` — pin, tag, delete, move-to-top, sync/event effects;
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
