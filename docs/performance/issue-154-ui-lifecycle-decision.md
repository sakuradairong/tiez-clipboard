# Issue #154: UI lifecycle and native UI feasibility decision

**Status:** staged decision, 2026-07-27
**Decision:** **NO-GO for a product migration or idle-destroy release today. GO for a Windows 11 measurement phase and a minimal Tauri WebView2 destroy/recreate prototype.**

## Decision in one page

[Issue #154](https://github.com/jimuzhe/tiez-clipboard/issues/154) asks TieZ to enter a lightweight mode after an idle interval: release the WebView, keep capturing text/images/files, and accept a cold-wake delay when the user next invokes the app. The idle time and feature enablement would eventually be user-configurable.

The current evidence supports further investigation but not a release:

1. TieZ already owns clipboard monitoring, persistence, global shortcuts, tray behavior, paste services, hooks, sync, and file transfer in Rust. `setup::init` creates managed state, then starts services and tray/hooks at application scope. This makes UI-independent operation architecturally plausible.
2. The current window lifecycle is not UI-independent in implementation. `toggle_window` starts with `get_webview_window("main")` and does nothing when that window no longer exists. Close requests are converted to `hide()`, so there is no production destroy or reconstruction path.
3. A locked Slint 1.16.1 PoC proves that a small compiled Rust UI can render and filter a virtualized 10,000-row synthetic list and that its event loop can remain alive without a component window.
4. On one Linux/Xvfb host, the Slint PoC used about 14.2 MiB private at 80 rows and 17.1 MiB at 10,000 rows. TieZ v0.3.8 with WebKitGTK and empty history used about 369.0 MiB visible, 337.9 MiB after 5 seconds hidden, and 325.5 MiB after 30 seconds hidden. The Web process remained alive.
5. Those numbers are screening evidence only. Linux uses WebKitGTK, not Windows WebView2, and the Slint PoC omits almost all TieZ behavior. No Windows memory, wake, IME, accessibility, focus, paste, or repeated-lifecycle result exists yet.
6. Disabling Slint accessibility saved only about 1.4 MiB on this host. Accessibility remains enabled in all candidate product routes.

Therefore the lowest-risk next experiment is to retain the complete React UI and destroy/recreate only the Tauri `main` WebView after an idle period. A native main surface is a fallback candidate, not the first migration step.

## Scope and terminology

Here, “native UI” can mean two different things:

- **No WebView:** Slint, egui/eframe, and iced compile to native applications but draw custom widgets.
- **Platform-native Windows UI:** WinUI 3 uses Windows controls and the Windows App SDK.

The experiment demonstrates the first meaning only. It does not show that Slint is visually or behaviorally equivalent to native Windows controls.

Out of scope for this phase:

- shipping a new idle-mode setting;
- changing production window, hotkey, or clipboard code;
- claiming Windows savings from Linux data;
- replacing advanced settings, compact preview, or the full React feature surface;
- disabling accessibility for a small memory saving.

## Current architecture: what can outlive the UI

`src-tauri/src/app/setup.rs::init` establishes the relevant ownership order:

1. resolve the data directory and apply pending restore;
2. initialize logging and SQLite;
3. load settings and install Tauri-managed state;
4. configure the existing main window;
5. start background services;
6. create the tray;
7. initialize Win32 hooks and the `TaskbarCreated` listener.

`start_services` starts window tracking, the clipboard monitor, MQTT/cloud sync, edge docking, optional file transfer, announcement work, and hotkey registration. The global-shortcut plugin is installed on the Tauri application builder. The Windows clipboard listener uses its own native listener/worker path. These objects are not React component state.

That is evidence for architectural separation, not proof of correct operation after the last WebView is destroyed. The Windows prototype must demonstrate that all of the following continue while `main` has no HWND/WebView:

- text, rich text, image, and file clipboard capture;
- pipeline transformation, privacy handling, deduplication, persistence, session-only history, encryption/OCR queues, and sync requests;
- global main/search/sequential/rich/plain/relay shortcuts;
- tray menu and tray restoration after Explorer restarts;
- keyboard/mouse hooks, foreground-window tracking, edge docking, paste queue, and paste-to-origin behavior;
- MQTT/cloud sync, relay, file server, notifications, and database writes where enabled.

### Known production gaps

- `window_manager::toggle_window`, `activate_window_focus`, `hide_window_cmd`, and `focus_clipboard_window` all assume the `main` WebView exists.
- `handle_window_event` prevents closing the main window and hides it. There is no destroyed-state transition.
- No single-flight `ensure_main_window` builder recreates the label, URL, capabilities, size/position, decorations, focus rules, native material, drag/drop registration, or event subscriptions.
- Frontend listeners disappear with the document. Clipboard events emitted while no frontend exists must be reconciled from authoritative Rust/SQLite/session state on remount, not assumed to be queued.
- React owns substantial transient state. A reconstruction contract must decide which state is restored and which state intentionally resets.
- `compact-preview` and `advanced-settings` are separate webview roots. Leaving either open can retain WebView2 processes and invalidate “lightweight mode” measurements.

## Evidence from the PoC

The reproducible protocol, raw results, binary hashes, and limitations are in [`experiments/native-ui-slint/README.md`](../../experiments/native-ui-slint/README.md). Every archived series contains five launches and three samples per launch.

| Linux/Xvfb series | Median processes | Median private |
| --- | ---: | ---: |
| Slint 1.16.1, accessibility, 80 rows, visible | 1 | 14.2 MiB |
| Slint 1.16.1, accessibility, 80 rows, hidden | 1 | 14.2 MiB |
| Slint 1.16.1, accessibility, component destroyed | 1 | 14.2 MiB |
| Slint 1.16.1, accessibility, 10,000 rows, visible | 1 | 17.1 MiB |
| Slint 1.16.1, no accessibility, 80 rows, visible | 1 | 12.8 MiB |
| TieZ v0.3.8/WebKitGTK, empty history, visible | 3 | 369.0 MiB |
| TieZ v0.3.8/WebKitGTK, hidden 5 seconds | 3 | 337.9 MiB |
| TieZ v0.3.8/WebKitGTK, hidden 30 seconds | 3 | 325.5 MiB |

The Slint PoC validates only a narrow UI mechanism: virtualized synthetic rows, filtering, selection/navigation, placeholders, hide/show, and dropping the component while the event loop survives. It does not contain production services or a reconstruction path.

The Linux results justify asking whether the WebView dominates memory. They do not quantify the answer on Windows and do not establish fair full-product parity. In particular:

- WebKitGTK and WebView2 have different process and memory behavior.
- Xvfb software rendering omits Windows composition, GPU, DPI, IME, and accessibility paths.
- the current TieZ run initialized the complete product while the Slint run initialized a small synthetic screen;
- Linux `Private_Clean + Private_Dirty` is not Windows private working set or commit;
- startup latency between these executables is not an apples-to-apples product result.

## Route matrix

Versions below freeze this investigation, not a product dependency decision.

| Route | React parity / Rust reuse | Platforms | License reviewed | Evidence and expected benefit | Main risks | Decision |
| --- | --- | --- | --- | --- | --- | --- |
| Keep hiding the current WebView | Full parity; no service changes | Existing Tauri targets | Existing project stack | Linux hidden private fell only 8.4% at 5 s and 11.8% at 30 s; Web process remained | Does not satisfy the requested release semantics; Windows result unknown | Baseline only |
| **Destroy/recreate Tauri WebView2** | **Keeps all React UI and Rust services** | Existing Tauri targets, with platform-specific lifecycle validation | Existing project stack | Most direct way to test Issue #154; no Windows result yet | Rebuild/state hydration, shared/brokered processes, focus/paste, window capabilities, lost events, race/reentrancy | **First prototype** |
| Slint 1.16.1 main surface, webviews only on demand | Rust integration is direct; main UI must be rewritten; advanced UI could remain React initially | Windows/Linux/macOS via selected winit backends | `GPL-3.0-only` route is compatible with this GPLv3 project; Slint also offers other license routes | Small synthetic Linux PoC measured 14.2–17.1 MiB private | Not platform-native widgets; full rich preview/theme/list/settings parity, IME, accessibility, focus, drag/drop, custom window behavior; Slint MSRV 1.88 | Second route if Tauri fails |
| egui/eframe 0.33.3 | Direct Rust reuse; complete UI rewrite | Cross-platform native/web | MIT OR Apache-2.0; MSRV 1.88 | No TieZ PoC or memory data | Immediate-mode model, custom rendering/native feel, rich content and complex layout parity, accessibility/IME validation | Do not prototype before Slint/Tauri evidence |
| iced 0.13.1 | Direct Rust reuse; complete UI rewrite into Elm-style state/update model | Cross-platform | MIT; declared MSRV 1.80 | No TieZ PoC or memory data | Ecosystem/integration maturity for TieZ-specific behavior, custom rendering, multi-window/IME/accessibility, rewrite cost | Reserve candidate |
| WinUI 3 / Windows App SDK | Best Windows-native control path, but needs Rust FFI, a Rust service process, or a UI sidecar contract | Windows only | Windows App SDK repository is MIT; runtime/distribution review still required | No TieZ PoC or memory data | Highest integration/packaging cost, split ownership and IPC/FFI, Windows-only UI, duplicate CI/release paths, cross-platform divergence | No-go unless Windows-only strategy changes |

No reviewed license is itself a reason to reject a route for this GPLv3 repository. Dependency notices, binary redistribution, and any commercial Slint license choice still require release review. License compatibility does not offset engineering or accessibility risk.

### Relative implementation cost

- Tauri destroy/recreate: small prototype, medium hardening effort.
- Slint hybrid main surface: large feature-parity effort.
- Full Slint, egui, or iced replacement: extra-large rewrite and regression surface.
- WinUI 3: extra-large rewrite plus a second language/process boundary and Windows-specific packaging.

These are relative sizes, not delivery estimates. No native route has enough evidence for a person-week estimate yet.

## Recommended Tauri prototype architecture

Keep one Tauri application process and all Rust managed state/services alive. Treat the main UI as a disposable projection of authoritative Rust state.

A prototype should introduce an explicit state machine rather than scattering `get_webview_window` fallbacks:

```text
visible -> hidden -> idle_pending -> destroying -> destroyed
   ^                                             |
   |                                             v
   +--------------- ready <- creating <- wake_requested
```

Required properties:

1. **Single owner and single flight.** One Rust coordinator serializes hide, destroy, and create. Multiple tray/hotkey requests coalesce. Generation IDs reject stale completion callbacks.
2. **Authoritative reconstruction.** On mount, the frontend fetches current settings, history, session-only entries, tags, queues, theme, and service status. Events are incremental hints after that snapshot.
3. **Defined transient state.** Preserve only state with product value, such as query/filter/selected entry/scroll anchor if desired. Never serialize secrets or raw sensitive content into an unnecessary UI cache.
4. **Window contract.** Recreate label `main`, URL, capabilities, size and position, theme/material, always-on-top/focusability, shadow/decorations, taskbar behavior, multi-monitor placement, drag/drop, and Windows extensions before showing.
5. **Focus and paste contract.** Capture the original foreground HWND before wake, show and focus the new window, focus the search field, and restore/paste to the correct origin without stealing or losing focus.
6. **Other roots.** Close/destroy compact preview before entering lightweight mode. An open advanced-settings window cancels the idle transition or is handled explicitly. The measurement must count every related WebView2 process.
7. **Safe timing.** Do not destroy during IME composition, drag/drop, modal file/dialog operations, pending paste, or window creation. Idle mode is disabled by default until canary validation.
8. **Failure fallback.** Creation timeout, navigation/load failure, or hydration failure triggers one clean retry and then reverts to the existing hidden-WebView path for that session. The tray must retain a recovery/quit action.
9. **Observability.** Log each transition, reason, generation, duration, process/memory sample correlation ID, and fallback without logging clipboard content.
10. **User control, later.** Only after the prototype passes should a disabled-by-default setting expose enablement and an idle interval. Changing or disabling it must cancel pending destruction safely.

Do not mix this prototype with shortcut fixes, release work, or a native rewrite. Keep it behind a compile-time or internal experimental flag until the gate below passes.

## Windows 11 decision protocol

### Comparison cells

On the same Windows 11 x64 machine, collect at least five clean runs for each cell:

1. current Tauri/WebView2 visible;
2. current Tauri/WebView2 hidden for 5 seconds;
3. current Tauri/WebView2 hidden for 30 seconds;
4. experimental prototype build with idle destruction disabled, hidden for 5 seconds;
5. the same experimental prototype binary and configuration, destroyed for 5 seconds, then wake;
6. experimental prototype build with idle destruction disabled, hidden for 30 seconds;
7. the same experimental prototype binary and configuration, destroyed for 30 seconds, then wake;
8. Slint 80-row and 10,000-row lifecycle-only screening builds.

If a native product-slice prototype is later built, add it as a new cell. Do not compare the lifecycle-only Slint PoC as though it were a full TieZ replacement.

Cells 1–3 establish the released-product baseline. The 40% attribution uses the matched prototype pairs in cells 4–7, not a production binary versus a changed binary. Each hidden/destroyed pair must use the exact same executable hash, feature set, data snapshot, and service configuration. Select the lifecycle behavior through an internal runtime switch so only the requested state differs.

Pin and record:

- Windows edition/build and update state;
- WebView2 runtime and application binary versions/hashes;
- release-mode build and feature flags;
- cloned database/data directory, item mix/count, settings, and network-service state;
- monitor layout, DPI/scaling, GPU/driver, power plan, accessibility state, and background load;
- warm/cold launch policy and sampling intervals.

Run cells sequentially in a randomized or alternating order. Verify no previous TieZ or PoC process remains before each launch. Keep accessibility enabled.

### Process and memory accounting

Primary script scope is root PID plus recursively observed descendants. For each sample report:

- total and per-process private working set;
- total and per-process commit/private bytes;
- total working set and process count;
- WebView2 browser, renderer, GPU, network, utility, and crashpad roles;
- time to first window, time to target state, and script-recorded wake-to-visible latency;
- separately instrumented wake-to-focused, hydrated, and search-ready latency.

A PPID tree can miss brokered, reused, or reparented WebView2 processes. Cross-check every cell with Process Explorer and an ETW/WPR trace. If either reveals a related process outside the tree, include it in a separately documented complete process set and do not use the descendant-only total for go/no-go. Exact-title detection and CIM/performance counter behavior must also be checked on the target machine.

Long-hidden private working set can fall from paging without resources being released. Commit and WebView2 process exit must corroborate the result. The committed script's `wake_ms` ends when an exact-title replacement window is visible. It does not prove focus, frontend hydration, or search readiness. Add an application readiness marker and focus assertion, or a correlated supplemental trace, before evaluating the usable-wake gate.

### Quantitative gate

Let `B5` and `B30` be the medians of the **complete related-process** private memory totals for the experimental prototype binary with idle destruction disabled and hidden for 5 and 30 seconds. Let `D5` and `D30` be the same binary's matching destroyed-state medians. For metric `M`:

```text
reduction(M) = 1 - median_destroyed(M) / median_hidden(M)
```

Proceed toward a product implementation only if all are true:

- private working set reduction is at least **40%** at both matched horizons;
- commit/private bytes show the same release, conservatively also at least **40%**, rather than only a paging effect;
- the median absolute reduction is at least **50 MiB** at both matched horizons, so a large percentage of a small baseline cannot justify lifecycle complexity;
- destroyed-instance renderer/browser resources exit or have a documented, bounded shared-runtime reason to remain;
- the result is stable across at least five runs and is not produced by one outlier;
- no unrelated process is omitted from the complete related set.

The 40% threshold is a deliberately high effect-size gate chosen before Windows results are available. It is not derived from the Linux/Xvfb measurements. The lifecycle and parity risks are only justified by an unambiguously material saving. If maintainers choose a different absolute MiB floor, they must record it before collecting the comparison data rather than tune it after seeing results.

If the Tauri route misses either the 40% or absolute-saving gate, stop product work on that route and use the trace to decide whether one small native main-surface prototype is justified. A Slint or other native route must pass the same full-product gates before migration.

Wake latency is explicitly allowed to increase, but it must be measured to a usable state, not merely HWND creation. A provisional canary bound is median at most 750 ms and worst of five at most 1.5 s from shortcut/tray action to visible, focused, hydrated, search-ready UI. If product owners accept a different bound, record it before looking at results.

## Functional acceptance matrix

Every row is blocking. “Works after wake” alone is insufficient if the operation failed while no UI existed.

| Area | Required observation while UI is destroyed and after wake |
| --- | --- |
| Clipboard capture | Copy text with leading/trailing whitespace and CRLF, rich text, image, and file list. Each reaches Rust/session or SQLite exactly once and appears after reconstruction. |
| Persistence modes | Positive persisted IDs and negative session-only IDs remain valid; pin/tag conversions use replacement IDs; no missing or duplicate entries after remount. |
| Privacy/security | Sensitive tags, encryption/decryption queueing, OCR plaintext removal, ignored applications/rules, and no secret logging remain correct. |
| Sync/services | Cloud/MQTT/relay/file-server behavior continues according to settings; UI reconstructs their latest status without stale listeners. |
| Global shortcuts | Main, search, sequential, rich/plain paste, relay, and Win+V takeover variants remain registered before, during, and after 100 lifecycle cycles. |
| Tray/hooks | Tray menu opens and can wake/quit; Explorer restart restores it; keyboard/mouse hooks and window tracking stay active without a UI HWND. |
| Reconstruction | Repeated shortcut/tray activation creates exactly one `main` window; timeout/failure falls back; no duplicate listeners or duplicate command execution. |
| State hydration | History, tags, pin state/order, settings, theme, filters, queues, and service states match Rust/SQLite. The product explicitly defines query/selection/scroll restoration. |
| Focus | Shortcut wake focuses search reliably; pinned/no-activate behavior, blur-to-hide, Escape, task switching, and foreground tracking match current semantics. |
| Paste | Selected/latest rich and plain paste target the pre-wake application correctly. Delete-after-paste and sequential queue advance exactly once. |
| IME | Chinese Pinyin composition, candidate-window placement, commit/cancel, arrows, Enter, Escape, and rapid destroy/wake never paste or activate prematurely. |
| Accessibility | Narrator exposes useful names/roles/states, focus order, selection, search results, buttons, and status changes. Keyboard-only use and high-contrast behavior remain usable. |
| Content parity | Text whitespace, links, color/rich text, images/GIF, files, OCR, source icons, tags, emoji, previews, drag/drop, context actions, and large lists remain correct. |
| Window behavior | Multi-monitor and mixed-DPI placement, size persistence, follow-mouse, always-on-top, edge docking, shadows/material, taskbar/Alt-Tab presence, compact preview, and advanced settings work. |
| Lifecycle reliability | At least 100 hide/destroy/wake cycles produce no crash, deadlock, orphan related process, stale HWND, duplicate window, or steadily growing handles/commit. Final steady-state memory is within 10% of the first stable destroyed cycle. |
| Shutdown/restart | Quit/relaunch, OS shutdown, update/restart, crash recovery, and pending restore terminate or restore services and WebView2 processes cleanly. |

Use real Windows automation where safe, but IME, Narrator, focus, paste-to-origin, native material, and multi-monitor behavior require human validation on the target OS.

## Staged plan and rollback

### Stage A: establish Windows baseline

Run the committed PowerShell tool on unmodified v0.3.8/current release behavior. Cross-check process coverage with Process Explorer and ETW. If the measurement cannot reliably identify the complete related set, fix the protocol before coding a lifecycle change. These runs are the released-product reference, not the matched causal control for a later prototype binary.

### Stage B: minimal Tauri prototype

On an isolated branch, add the state coordinator, destroy/recreate path, authoritative hydration, and an internal runtime flag. Do not add public settings yet. Use one prototype binary for both hidden-control and destroyed cells, alternating the runtime mode. Add a focus assertion and frontend-ready marker so usable wake can be measured separately from `wake_ms`. Measure memory and wake first.

### Stage C: lifecycle hardening

Only after the 40% gate passes, implement fallback, diagnostics, all-window policy, focus/paste handling, and the acceptance matrix. Exercise 100 cycles and fault injection for navigation/hydration timeouts.

### Stage D: opt-in canary

Expose a disabled-by-default experimental toggle and idle duration. Gather anonymized operational metrics only if the project's privacy policy and user consent permit it. Keep instant rollback to permanent hiding.

### Stage E: native fallback decision

Only if Tauri destruction fails the memory gate or cannot meet lifecycle correctness should the team build a small Slint product slice using real Rust history and paste commands. Compare it on the same machine. Do not start a full rewrite from the synthetic PoC.

Rollback immediately to the existing hide path if any of these occurs:

- clipboard, shortcut, tray, hook, sync, or paste behavior fails while the UI is absent;
- IME, Narrator/accessibility, focus, paste target, or multi-window behavior regresses;
- creation or hydration times out, produces duplicate windows/listeners, or loses state;
- a related WebView2 process leaks across cycles or commit/handles trend upward;
- crashes, hangs, or wake latency exceed the agreed bound;
- complete-process-set private memory savings fall below 40%.

## Final recommendation

Do not replace TieZ's UI now. Do not ship automatic WebView destruction from this evidence. Keep accessibility enabled.

Proceed only with Windows 11 baseline measurement and a feature-flagged Tauri destroy/recreate prototype because it preserves the existing React surface and Rust services and has the smallest parity risk. Treat Slint as the leading no-WebView fallback because the locked PoC is runnable and low-cost on Linux, while recognizing that its full-product cost and Windows behavior remain unknown. Keep egui/eframe and iced as reserve research candidates. Reject WinUI 3 for the current cross-platform strategy unless the project deliberately accepts a Windows-specific UI and service boundary.

The next go/no-go occurs after same-machine Windows data and the blocking functional matrix, not after the Linux screening result.

## Sources reviewed

- [Upstream Issue #154](https://github.com/jimuzhe/tiez-clipboard/issues/154)
- TieZ `src-tauri/src/app/setup.rs`, `window_manager.rs`, `main.rs`, and `services/clipboard_listener.rs`
- TieZ frontend roots in `src/main.tsx` and feature components under `src/features/`
- [Slint 1.16.1 crate metadata and license expression](https://crates.io/crates/slint/1.16.1)
- [egui/eframe 0.33.3](https://crates.io/crates/eframe/0.33.3)
- [iced 0.13.1](https://crates.io/crates/iced/0.13.1)
- [Windows App SDK repository and MIT license](https://github.com/microsoft/WindowsAppSDK)
