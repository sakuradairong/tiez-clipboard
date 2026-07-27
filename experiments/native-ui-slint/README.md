# Native UI and WebView lifecycle feasibility experiment

This directory contains research artifacts for [upstream Issue #154](https://github.com/jimuzhe/tiez-clipboard/issues/154). The issue asks whether TieZ can release its WebView after an idle period while its clipboard capture, global shortcut, and tray behavior continue in a lightweight Rust process.

This is an isolated feasibility experiment. It is not wired into the TieZ application, does not replace the React UI, and must not be used as evidence that a native migration is ready.

## What is included

- `src/main.rs`: a Slint 1.16.1 synthetic clipboard-list PoC.
- `scripts/measure_linux.py`: Linux/X11 process-tree and window-state measurement.
- `scripts/measure_windows.ps1`: Windows process-tree, window-state, and wake-latency measurement.
- `results/linux-xvfb/`: eight raw five-run Linux/Xvfb reports and checksums.
- `../../docs/performance/issue-154-ui-lifecycle-decision.md`: the route matrix and staged decision.

The PoC exercises a virtualized list, 80 or 10,000 synthetic rows, text/image/file placeholders, search, mouse activation, Up/Down/Enter navigation, a two-second hide/show cycle, and hidden or destroyed-window sampling modes. It deliberately does **not** implement TieZ's clipboard listener, tray, global shortcut, paste pipeline, persistence, rich preview, settings, or synchronization. Those remain production-Rust architecture and Windows acceptance questions.

## Toolchain and dependency choices

- Rust MSRV: 1.88.
- Slint: exactly 1.16.1, resolved by the committed `Cargo.lock`.
- Default feature: Slint accessibility support is enabled.
- Linux: winit X11 plus the software renderer and system fonts.
- Windows/macOS: generic winit plus the software renderer and system fonts.
- License route for this GPLv3 project: Slint's `GPL-3.0-only` option.

The target-specific dependency declarations preserve the measured Linux configuration while allowing the same PoC source to build on Windows. `target/`, local binaries, and scratch lockfiles are intentionally ignored.

## Build and run the PoC

Run from the repository root:

```bash
cargo build --locked --release \
  --manifest-path experiments/native-ui-slint/Cargo.toml

TIEZ_NATIVE_POC_ROWS=80 \
  experiments/native-ui-slint/target/release/tiez-native-ui-poc
```

Supported environment variables:

| Variable | Meaning | Default |
| --- | --- | --- |
| `TIEZ_NATIVE_POC_ROWS` | Number of synthetic clipboard rows | `10000` |
| `TIEZ_NATIVE_POC_AUTO_HIDE_MS` | Hide the component after this delay and keep the tray-style event loop alive | unset |
| `TIEZ_NATIVE_POC_AUTO_DESTROY_MS` | Hide and drop the final component handle after this delay while the event loop remains alive | unset |
| `TIEZ_NATIVE_POC_AUTO_EXIT_MS` | Quit the event loop after this delay, useful for automation | unset |

The destroy mode proves only that a Rust/Slint event loop can remain alive with no component window. It does not recreate the component and it is not a substitute for testing TieZ's real tray and hotkey paths.

To build the accessibility A/B binaries locally:

```bash
mkdir -p experiments/native-ui-slint/artifacts

cargo build --locked --release \
  --manifest-path experiments/native-ui-slint/Cargo.toml
cp experiments/native-ui-slint/target/release/tiez-native-ui-poc \
  experiments/native-ui-slint/artifacts/tiez-native-ui-poc-accessibility

cargo build --locked --release --no-default-features \
  --manifest-path experiments/native-ui-slint/Cargo.toml
cp experiments/native-ui-slint/target/release/tiez-native-ui-poc \
  experiments/native-ui-slint/artifacts/tiez-native-ui-poc-no-accessibility
```

The copied binaries remain untracked. Build provenance is recorded below because compiler paths and build metadata can prevent byte-for-byte reproduction even with a lockfile.

## Linux measurement protocol

### Requirements

The script needs Linux `/proc`, `/proc/<pid>/smaps_rollup`, Python 3, `xwininfo`, and `xprop`. X11 key injection additionally needs `libX11` and `libXtst`. The archived run used:

- Debian 13, kernel `6.12.95+deb13-amd64`, x86-64.
- 32 vCPUs reported as Intel Xeon E5-2650L v2 at 1.70 GHz.
- Python 3.13.5 and Xvfb 21.1.16.
- Rust/Cargo 1.88.0.
- Slint software rendering under Xvfb, with no desktop compositor.

For every run, the tool:

1. launches a fresh root process in its own process group;
2. waits for an exact-title visible X11 window belonging to the root or a descendant;
3. optionally sends a key chord, then verifies `visible`, `hidden`, or `destroyed` state;
4. waits for the configured warm-up;
5. takes three process-tree samples at 250 ms intervals;
6. uses the median sample for that run and then the median of five runs;
7. terminates the launched process group.

Metrics are aggregated across the root and recursively discovered descendants:

- **RSS**: `Rss` from `smaps_rollup`.
- **PSS**: `Pss` from `smaps_rollup`.
- **Private**: `Private_Clean + Private_Dirty`, an approximate Linux USS measure.
- **Window ms**: process launch to the first exact-title visible window.
- **State ms**: process launch to the requested state. It is not transition-only latency.

### Reproduction examples

Use a fresh, private runtime directory for each series and run series sequentially so concurrent Xvfb or compilation load does not contaminate results:

```bash
BASE="${XDG_CACHE_HOME:-$HOME/.cache}/tiez-native-ui-bench"
mkdir -p "$BASE/slint-80/home" "$BASE/slint-80/runtime"
chmod 700 "$BASE/slint-80/runtime"

HOME="$BASE/slint-80/home" \
XDG_RUNTIME_DIR="$BASE/slint-80/runtime" \
TIEZ_NATIVE_POC_ROWS=80 \
xvfb-run -a python3 experiments/native-ui-slint/scripts/measure_linux.py \
  --label slint-1.16.1-accessibility-visible-80 \
  --runs 5 \
  --window-title "TieZ native UI feasibility PoC" \
  --sample-state visible \
  --warmup-seconds 3 \
  --samples-per-run 3 \
  --sample-interval-seconds 0.25 \
  --output "$BASE/slint-visible-80.json" \
  -- experiments/native-ui-slint/target/release/tiez-native-ui-poc
```

Hidden state uses the same command with these changes:

```bash
TIEZ_NATIVE_POC_ROWS=80 \
TIEZ_NATIVE_POC_AUTO_HIDE_MS=500 \
TIEZ_NATIVE_POC_AUTO_EXIT_MS=30000 \
xvfb-run -a python3 experiments/native-ui-slint/scripts/measure_linux.py \
  --label slint-1.16.1-accessibility-hidden-80 \
  --runs 5 \
  --window-title "TieZ native UI feasibility PoC" \
  --sample-state hidden \
  --warmup-seconds 3 \
  --output "$BASE/slint-hidden-80.json" \
  -- experiments/native-ui-slint/target/release/tiez-native-ui-poc
```

Destroyed state replaces `AUTO_HIDE` with `TIEZ_NATIVE_POC_AUTO_DESTROY_MS=500` and uses `--sample-state destroyed`. The 10,000-row series uses `TIEZ_NATIVE_POC_ROWS=10000`. For the no-accessibility series, point the final command at the binary built with `--no-default-features`.

For the Tauri baseline, use a release binary from the exact commit under test and fresh XDG directories. The archived baseline was commit `f1be5fb`/v0.3.8 with an empty `clipboard_history` table. For example:

```bash
BASE="${XDG_CACHE_HOME:-$HOME/.cache}/tiez-native-ui-bench/tauri-v038"
mkdir -p "$BASE/home" "$BASE/data" "$BASE/config" "$BASE/cache" "$BASE/runtime"
chmod 700 "$BASE/runtime"

HOME="$BASE/home" \
XDG_DATA_HOME="$BASE/data" \
XDG_CONFIG_HOME="$BASE/config" \
XDG_CACHE_HOME="$BASE/cache" \
XDG_RUNTIME_DIR="$BASE/runtime" \
xvfb-run -a python3 experiments/native-ui-slint/scripts/measure_linux.py \
  --label tauri-v0.3.8-webkit-hidden-hotkey \
  --runs 5 \
  --window-title TieZ \
  --sample-state hidden \
  --state-trigger-key Alt+C \
  --state-trigger-count 1 \
  --state-trigger-delay-seconds 2 \
  --warmup-seconds 5 \
  --output "$BASE/tauri-hidden.json" \
  -- /path/to/tiez-app
```

Use `--warmup-seconds 30` for the delayed-release check. The key chord must match the isolated profile's configured shortcut. The tool focuses the measured window before injection and verifies that TieZ actually reaches hidden state.

## Archived Linux/Xvfb results

All values below are medians of five fresh launches. Each launch contributes the median of three samples. MiB values divide the raw KiB values by 1024. The JSON files retain every launch, sample, PID, process name, and raw KiB value.

| Series | State warm-up | Processes | RSS MiB | PSS MiB | Private MiB | Window ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Slint, accessibility, 80 rows, visible | 3 s | 1 | 19.3 | 15.1 | 14.2 | 305.4 |
| Slint, accessibility, 80 rows, hidden | 3 s | 1 | 19.3 | 15.1 | 14.2 | 324.1 |
| Slint, accessibility, 80 rows, component destroyed | 3 s | 1 | 18.2 | 14.6 | 14.2 | 321.6 |
| Slint, accessibility, 10,000 rows, visible | 3 s | 1 | 22.2 | 18.0 | 17.1 | 275.5 |
| Slint, no accessibility, 80 rows, visible | 3 s | 1 | 17.9 | 13.7 | 12.8 | 285.5 |
| TieZ v0.3.8/WebKitGTK, empty history, visible | 5 s | 3 | 638.3 | 460.8 | 369.0 | 900.5 |
| TieZ v0.3.8/WebKitGTK, hidden through `Alt+C` | 5 s | 3 | 606.9 | 429.5 | 337.9 | 896.9 |
| TieZ v0.3.8/WebKitGTK, hidden through `Alt+C` | 30 s | 3 | 594.1 | 416.9 | 325.5 | 902.4 |

Observations limited to this host and protocol:

- Slint's visible, hidden, and destroyed 80-row medians are all about 14.2 MiB private. Destroying this small component did not return additional private heap on Linux.
- Increasing the synthetic list from 80 to 10,000 rows added about 2.9 MiB private.
- Disabling accessibility saved about 1.4 MiB private. This is not enough to justify removing accessibility.
- TieZ/WebKitGTK retained three median processes, including its Web process, after 5 and 30 seconds hidden. Private memory was 8.4% and 11.8% below the visible median respectively, not evidence of WebView release.
- The large Slint-versus-TieZ difference is only a screening signal. The Slint PoC omits almost all product functionality, while TieZ initializes its real database, services, tray, and React/WebKit UI.

Measured binary provenance:

| Binary used by archived series | SHA-256 | Size |
| --- | --- | ---: |
| Slint 1.16.1, accessibility | `9fcc111ef70c28cecb7c57634015211918b987094200247ef1e9784e825eb08f` | 18,272,592 bytes |
| Slint 1.16.1, no accessibility | `494ae2fbb3c8176649d86510fde3c0b975fd213761f0e1597d99de5601e10c39` | 16,196,888 bytes |
| TieZ v0.3.8 Linux release at `f1be5fb` | `3b3a4be4bb217e411dfbd67a22fd2feeaa450b4f78d251ebfcd86c6099fdb5d0` | 15,471,984 bytes |

The binaries themselves are deliberately not committed. Three early Slint reports record `state_trigger` as `null`; they were captured immediately before key-trigger metadata was added. Their process-tree, state-validation, and memory calculations match the documented protocol. Checksums in `results/linux-xvfb/SHA256SUMS` protect the unmodified raw reports.

## What Linux does not prove

These results are **not** a Windows/WebView2 conclusion:

- Linux TieZ uses WebKitGTK, not WebView2.
- Xvfb and software rendering do not represent Windows composition, GPU processes, DPI, focus, IME, or accessibility behavior.
- Linux private memory is not Windows private working set or commit.
- Descendant-only process discovery can miss brokered or reparented processes.
- Synthetic rows are not equivalent to TieZ's text, rich text, images, files, OCR, tags, themes, and settings.
- Startup times are not comparable because one executable is a small UI PoC and the other is the complete application.
- No Linux run validates WebView destruction/recreation, state hydration, paste-to-origin, or repeated lifecycle reliability in TieZ.

## Windows 11 measurement

Build release binaries first. Run `measure_windows.ps1` from 64-bit Windows PowerShell, elevated only if process inspection requires it. Example current-TieZ hidden and wake series:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\experiments\native-ui-slint\scripts\measure_windows.ps1 `
  -Label "tauri-webview2-hidden-5s" `
  -Executable "C:\path\to\tiez-app.exe" `
  -WindowTitle "TieZ" `
  -SampleState hidden `
  -StateTriggerChord "Alt+C" `
  -StateTriggerDelaySeconds 2 `
  -WarmupSeconds 5 `
  -Runs 5 `
  -SamplesPerRun 3 `
  -SampleIntervalSeconds 1 `
  -MeasureWake `
  -WakeTriggerChord "Alt+C" `
  -Output "$env:TEMP\tiez-webview2-hidden-5s.json"
```

Repeat with 30 seconds warm-up. For a destroy/recreate prototype, configure the prototype's actual destroy trigger or idle timeout, use `-SampleState destroyed`, and keep `-MeasureWake`. The current TieZ product has no destroy/recreate path, so such a run cannot be performed against v0.3.8 without an experimental implementation.

For the Slint lifecycle-only build:

```powershell
$env:TIEZ_NATIVE_POC_ROWS = "80"
$env:TIEZ_NATIVE_POC_AUTO_DESTROY_MS = "500"
$env:TIEZ_NATIVE_POC_AUTO_EXIT_MS = "30000"

powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\experiments\native-ui-slint\scripts\measure_windows.ps1 `
  -Label "slint-1.16.1-destroyed-80" `
  -Executable ".\experiments\native-ui-slint\target\release\tiez-native-ui-poc.exe" `
  -WindowTitle "TieZ native UI feasibility PoC" `
  -SampleState destroyed `
  -WarmupSeconds 5 `
  -Runs 5 `
  -Output "$env:TEMP\tiez-slint-destroyed.json"
```

Do not request wake measurement for the current Slint destroy mode. It intentionally has no tray/hotkey-driven reconstruction.

The Windows report records:

- root plus recursively discovered descendant processes;
- executable path, version, command line-derived WebView2 role, PID, parent PID, and start time;
- total working set, private working set, and commit;
- state transition and optional shortcut-to-visible-window latency;
- Windows build, CPU, memory, PowerShell version, executable hash, and raw samples.

The script verifies PID identity with process start times, retries inconsistent CIM/performance-counter snapshots, checks for replacement exact-title windows, and cleans up only captured PID/start-time identities. `summary.median_wake_ms` stops when the tracked or replacement exact-title window is visible. It does **not** establish focus, frontend hydration, or search readiness. The product decision therefore requires a correlated application-ready marker and focus assertion in addition to this script value. The script's PowerShell AST and embedded C# compile under PowerShell 7.4.7, including the expected 40-byte x64 `INPUT` layout. It has not yet run on Windows PowerShell 5.1 or a Windows 11 machine.

### Required Windows controls and cross-checks

Run current hidden WebView2, WebView2 destroyed/recreated, and Slint on the **same Windows 11 machine**, at least five fresh launches each. For causal hidden-versus-destroyed comparison, use the exact same prototype executable hash and configuration, selecting only the lifecycle mode through an internal runtime switch. Keep OS build, WebView2 runtime, release mode, dataset snapshot, app settings, display scaling, power plan, and background load constant. Accessibility stays enabled.

The script's strict ParentProcessId tree is reproducible but may miss brokered or reparented WebView2 processes. For every route, cross-check the complete related process set with Process Explorer and an ETW/WPR trace. If the cross-check finds a related process outside the tree, do not use the script total alone for the decision metric. Record and explain the complete set rather than adding an unverified process-name heuristic to the primary result.

The product gate and functional matrix are defined in the decision document. Linux screening alone cannot pass that gate.
