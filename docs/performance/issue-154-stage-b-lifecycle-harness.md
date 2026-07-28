# Issue #154 Stage B lifecycle harness protocol

**Status:** internal prototype and evidence tooling only. No Windows 11/WebView2 result has been captured in this repository.

Stage B measures the Stage A recommendation: keep the React/Tauri application, but allow the main WebView2 surface to be hidden or destroyed while Rust services remain alive. Linux/WebKitGTK results are compile and logic screening only.

## Prototype and transport

The experiment is off by default and has no public setting. It starts only when both conditions hold:

- `TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE=hidden|destroyed`
- `TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR=<unique empty directory>`

The harness uses a filesystem transport because no WebView exists in the destroyed state. For each safe request ID it atomically publishes `<id>.request.json` from a same-directory temporary file, then waits for `<id>.response.json`. Rust applies a command allowlist and atomically writes the response. There is no shared `request.json`, localhost listener, or release-default endpoint.

Allowed commands are:

- `lifecycle_test_hide`
- `lifecycle_test_show`
- `lifecycle_test_toggle` for narrow Rust regression coverage, not the measurement loop
- `get_main_ui_lifecycle_snapshot`
- `get_main_ui_lifecycle_traces`
- `get_main_ui_lifecycle_clipboard_probe`

`measure_lifecycle_windows.ps1` deliberately uses separate hide and show requests. This provides an observable down interval for clipboard injection and 5 s/30 s process-memory samples.

## Snapshot, readiness, and clipboard proof

A snapshot reports the lifecycle phase and generation, window presence/visibility/focus, frontend hydration barriers, the current wake request ID, the usable-ready latency, and an absolute Rust clipboard-listener event counter.

Every wake is usable only after the same generation is visible, hydrated, and natively focused. Search/test wakes additionally require search-ready. The correlated sequence is:

1. requested,
2. visible,
3. native focused,
4. hydrated,
5. search-ready.

The show response includes `expected_generation`. The PowerShell loop accepts only a `ready` snapshot for that exact generation, preventing a stale prior ready state from satisfying the next cycle.

For every down cycle, the script:

1. records `clipboard_event_count` from the down snapshot,
2. writes a unique token with `Set-Clipboard`,
3. polls `get_main_ui_lifecycle_clipboard_probe` with that baseline count,
4. requires the Rust event count to have increased, and
5. requires exactly one token match across the persistent and session-only history backends.

The persistent probe decrypts text rows with the running application's `EncryptionManager`; encrypted history must not create a false negative. The probe is available only through the guarded internal transport and is not a public Tauri command.

## Acceptance gates

Run hidden and destroyed against the **same executable SHA-256** and otherwise matching app data, features, services, hardware, and host state.

### Per-mode functional report

- Windows 11 build 22000 or newer.
- At least 100 cycles.
- At most one visible main window at any time.
- Every cycle reaches its requested `hidden` or `destroyed` phase and then the exact expected `ready` generation.
- Every cycle proves clipboard-listener activity and exactly one history token match while the UI is down.
- Median requested-to-visible-focused-hydrated-search-ready latency is `<= 750 ms`.
- Each of the worst five latencies is `<= 1500 ms`.
- At least five independent down-state memory runs.
- Each memory run samples the recursively discovered app process tree plus `msedgewebview2.exe` processes sharing the baseline `--user-data-dir`, after 5 seconds and 30 seconds.
- Every accepted memory sample must match the baseline related-process identity keys (`role|executable_path|started_at_utc`); a process-set change invalidates the sample instead of silently changing the denominator.

The single-mode schema is `docs/performance/issue-154-lifecycle-report.schema.json` (schema version 2). Its `overall_pass` and paired memory result remain `null` by design.

### Paired hidden-vs-destroyed gate

A single-mode report cannot establish the memory benefit. Pair reports only when executable hash and all non-mode configuration match. At **both 5 seconds and 30 seconds**, destroyed must reduce the median full-process-tree private working set relative to hidden by:

- at least 40 percent, and
- at least 50 MiB.

Cross-check the captured process identities with Process Explorer and ETW/WPR before treating the attributed related-process set as complete. `experiments/issue-154-native-ui/compare_lifecycle_reports.ps1` computes this gate and rejects mismatched executable hashes, arguments, hosts, cycle counts, sample horizons, or process scopes. Its output validates against `docs/performance/issue-154-lifecycle-pair.schema.json`. It deliberately leaves `overall_pass` null until manual comparability, the process-set cross-check, and native behavior gates are signed off.

Accessibility, IME, focus restoration, paste behavior, tray and shortcut behavior, sync/background services, and crash/failure behavior are separate required Windows checks. Do not trade accessibility support for the approximately 1 to 1.4 MiB saving observed in earlier screening.

## Windows runbook

Use a Windows 11 physical machine or stable Windows 11 VM with WebView2 Runtime. Close other TieZ instances and document the data snapshot, feature flags, service configuration, power plan, WebView2 version, and foreground application. The script creates and removes its unique harness directory and restores both process-level environment variables.

```powershell
$exe = "C:\path\to\TieZ.exe"
$out = "C:\path\to\issue154-stage-b"
New-Item -ItemType Directory -Force -Path $out | Out-Null

powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\experiments\issue-154-native-ui\measure_lifecycle_windows.ps1 `
  -Label "issue154-stage-b-hidden" `
  -Executable $exe `
  -Mode hidden `
  -Cycles 100 `
  -MemoryRuns 5 `
  -Output "$out\hidden-100.json"

powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\experiments\issue-154-native-ui\measure_lifecycle_windows.ps1 `
  -Label "issue154-stage-b-destroyed" `
  -Executable $exe `
  -Mode destroyed `
  -Cycles 100 `
  -MemoryRuns 5 `
  -Output "$out\destroyed-100.json"

powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\experiments\issue-154-native-ui\compare_lifecycle_reports.ps1 `
  -HiddenReport "$out\hidden-100.json" `
  -DestroyedReport "$out\destroyed-100.json" `
  -Output "$out\hidden-vs-destroyed.json"
```

Validate both raw reports against the draft-07 schema. Confirm `executable_sha256` is identical. Do not infer the paired memory gate from the per-mode baseline-ready sample because the acceptance comparison is destroyed versus hidden at the same down-state horizons.

## Evidence filing skeleton

Create a dated note under `docs/performance/` only after a real Windows run:

```markdown
# Issue #154 Stage B Windows WebView2 lifecycle evidence, YYYY-MM-DD

- Host: Windows 11 version/build, CPU, RAM, power plan.
- WebView2 Runtime: version.
- TieZ executable: path, version, commit, SHA-256.
- Reports: hidden path/hash; destroyed path/hash; schema version 2.
- Same executable and non-mode configuration: yes/no, with evidence.
- Test data/profile and enabled services: describe.
- Process Explorer and ETW/WPR descendant-set cross-check: describe.
- Linux results: screening only, not Windows pass/fail evidence.

| Gate | Result | Evidence |
| --- | --- | --- |
| Hidden 100 cycles and readiness latency |  |  |
| Destroyed 100 cycles and readiness latency |  |  |
| Down-state clipboard listener plus unique history token |  |  |
| 5 s destroyed-vs-hidden memory >=40% and >=50 MiB |  |  |
| 30 s destroyed-vs-hidden memory >=40% and >=50 MiB |  |  |
| Accessibility and Narrator |  |  |
| IME composition |  |  |
| Focus/no-activate semantics |  |  |
| Paste and focus restoration |  |  |
| Tray, shortcuts, sync, listener, and other background services |  |  |
| Crash/failure recovery |  |  |

## Conclusion

Do not mark Stage B as passing until the paired memory gate and all native-behavior gates pass on Windows 11 with matched evidence.
```

## Current blockers

- This Linux host has no PowerShell, Windows Rust target, WebView2, or Windows 11 GUI machine. It can validate Rust/frontend compilation, tests, JSON structure, and static protocol agreement only.
- The script has not received a PowerShell 5.1 AST parse or an end-to-end Windows run.
- The paired comparator, ETW/WPR capture, Process Explorer cross-check, accessibility/Narrator, IME, real focus, paste, tray/shortcut, and service-survival tests remain Windows validation work.
- No Windows conclusion should be inferred from Linux/WebKitGTK screening.
