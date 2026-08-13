# WebView2 alternatives and lightweight UI strategy

**Status:** research recommendation, 2026-08-13
**Related:** [upstream Issue #154](https://github.com/jimuzhe/tiez-clipboard/issues/154), branch `explore/native-ui-webview2-alternative`

## Executive recommendation

Do **not** replace Microsoft Edge WebView2 as the first response to TieZ's idle-memory problem.

The preferred order is:

1. **Set WebView2's memory usage target to `Low` while TieZ is hidden.** This keeps the React UI and scripts alive and is already supported by the project's locked WRY version (`0.54.1`).
2. **Prototype WebView2 `TrySuspend`/automatic resume.** This is a stronger hidden-state mode: it pauses page timers and animations, minimizes renderer CPU, and allows Windows to reuse renderer memory. It preserves the loaded React document but requires a small Windows COM integration because Tauri/WRY do not currently expose it directly.
3. **Complete the existing destroy/recreate Stage B measurement.** This is the strongest same-stack option and remains the correct fallback if suspend does not meet the memory target.
4. **If the WebView2 process must not remain resident, split TieZ into a lightweight Rust core process and an on-demand Tauri UI process.** This is cleaner than switching to another HTML engine because the UI process can exit completely while clipboard monitoring continues.
5. **Only if all WebView lifecycle/process-isolation routes fail, prototype a real Slint product slice.** Keep advanced settings and rich previews in on-demand React windows initially.

CEF, Ultralight, Sciter, full Slint/egui/iced rewrites, and WinUI 3 are not better first moves for this repository.

## Problem and constraints

Issue #154 asks TieZ to release the web UI after an idle interval while the application continues capturing text, images, and files. Users may accept a cold-wake delay, and the feature and timeout should eventually be configurable.

TieZ is already well positioned for this because its clipboard listener, persistence, hotkeys, tray, hooks, paste operations, backup, synchronization, encryption, and file transfer are owned by Rust. The main obstacle is lifecycle implementation: current code assumes the `main` WebView window exists, and close operations normally hide rather than destroy it.

Any solution must preserve:

- global shortcut, tray, clipboard listener, hook, sync, and paste-to-origin behavior while the UI is unavailable;
- positive and negative clipboard IDs, authoritative history reload, and event resubscription;
- main, compact-preview, and advanced-settings window contracts;
- Windows focus restoration, IME, Narrator/accessibility, DPI, multi-monitor placement, material effects, drag/drop, and native file dialogs;
- current Windows, Linux, and macOS packaging unless the project deliberately becomes Windows-only;
- GPLv3-compatible distribution.

## Existing evidence

The exploration branch already contains a Slint 1.16.1 lifecycle-only PoC and a Tauri destroy/recreate harness.

The archived Linux/Xvfb screening measured approximately 14.2–17.1 MiB private memory for a synthetic Slint list. A full TieZ/WebKitGTK run measured 460.8 MiB private while visible, 429.5 MiB after 5 seconds hidden, and 416.9 MiB after 30 seconds hidden. Hiding therefore reduced private memory by only 8.4% and 11.8% in that environment, and the web process remained.

These numbers do **not** compare full products and do not predict WebView2 behavior on Windows. The Slint PoC omits the real clipboard services, persistence, rich preview, settings, synchronization, accessibility acceptance, and reconstruction path. The useful conclusion is only that a compiled UI may be materially smaller and that hiding a WebView is not equivalent to releasing it.

The existing decision gate remains sound: use matched Windows 11 runs, require at least 100 lifecycle cycles, median usable wake no more than 750 ms, each of the worst five wakes no more than 1500 ms, and at least 40% complete-process-set private-memory savings before hardening or exposing an idle-destroy setting.

## Best near-term option: WebView2 low-memory mode

Microsoft exposes `MemoryUsageTargetLevel`. Setting it to `Low` is intended for inactive applications that still need scripts or network connections. Scripts continue running. The operation is best-effort and may swap browser-process memory to disk; the application must explicitly restore `Normal` when active.

WRY exposes this on Windows as `WebViewExtWindows::set_memory_usage_level`. The API has existed since WebView2 Runtime `114.0.1823.32`; WRY documents that it becomes a no-op on older runtimes. TieZ currently locks WRY `0.54.1`, whose source includes `MemoryUsageLevel::{Normal, Low}` and this extension method.

Tauri 2.10.2 exposes a `with_webview` callback on the UI thread, but its Windows `PlatformWebview` wrapper carries only the WebView2 controller and environment, not the full WRY `WebView`. The least invasive production implementation choices are therefore:

1. add a narrow method upstream or through a patched `tauri-runtime-wry` that forwards WRY's existing API; or
2. use Tauri's `with_webview` callback plus a direct `webview2-com` query for the required WebView interface if the platform handle is extended to expose `ICoreWebView2`.

The first PoC should be tiny: switch to `Low` after hiding, wait 5 and 30 seconds, switch to `Normal` before showing, and compare complete process groups against the current hidden baseline. It does not need frontend lifecycle changes.

### Expected advantages

- smallest implementation and regression surface;
- no React reload, hydration, state serialization, listener replay, or cold startup;
- no new UI toolkit, renderer, runtime, packaging, or license obligation;
- maintains scripts and event listeners, so current frontend assumptions remain valid.

### Limits

- best-effort rather than a guarantee;
- may reduce working set more than commit/private bytes;
- scripts can page memory back in;
- browser/GPU processes remain, so it may not meet Issue #154's desired “release WebView” semantics.

## Second option: WebView2 `TrySuspend`

`ICoreWebView2_3::TrySuspend` is specifically intended for an invisible WebView. The controller must first be invisible. Microsoft states that suspension pauses script timers and animations, minimizes renderer CPU, and allows the operating system to reuse renderer memory. Suspension is best-effort; a running script finishes first, and some conditions may prevent suspension. The WebView automatically resumes when visible. Navigation and some other APIs can also resume it.

This is a better fit than destroying the WebView when the goal is low idle resource use with a fast wake:

- the React DOM, JavaScript heap, scroll state, query state, and subscriptions remain conceptually loaded;
- wake does not require page navigation and authoritative hydration;
- the background clipboard listener remains in Rust, so pausing frontend timers is acceptable if UI events are treated as hints and the next visible state refreshes authoritatively.

WRY `0.54.1` does not expose `TrySuspend` or `Resume`. TieZ would need a narrow Windows-specific implementation using `webview2-com`, or an upstream WRY/Tauri extension. Do not combine `TrySuspend/Resume` with `MemoryUsageTargetLevel` switching in the same experiment; Microsoft advises choosing one mechanism.

The PoC must verify:

- the controller is invisible before suspension;
- success/failure from the asynchronous completion handler is recorded;
- no frontend invocation accidentally auto-resumes the WebView while hidden;
- on show, the frontend performs one lightweight authoritative refresh before becoming search-ready;
- shortcut, tray, clipboard, hooks, sync, and paste behavior remain UI-independent.

## Third option: destroy and recreate the Tauri WebView

The existing Stage B branch is still the strongest solution that preserves the current product UI. Tauri's Rust core remains alive while the `main` WebView is disposable.

It can release more memory than suspension, but requires an explicit lifecycle coordinator:

```text
visible -> hidden -> idle_pending -> destroying -> destroyed
   ^                                             |
   |                                             v
   +--------------- ready <- creating <- wake_requested
```

The coordinator must provide single-flight creation, generation IDs, complete window reconstruction, authoritative state hydration, event-gap tolerance, focus/paste guarantees, and explicit handling of compact preview and advanced settings. This has a medium hardening cost but avoids a full UI rewrite.

## Better architectural fallback: lightweight core plus on-demand UI process

If the requirement is that no WebView2 process remains after the idle timeout, a process boundary is more direct than replacing Edge with another embedded browser.

Suggested shape:

```text
tiez-core.exe
  clipboard listener, database, tray, shortcuts, hooks, sync, paste services
  owns single-instance and lifecycle policy
          |
          | authenticated local IPC / named pipe
          v
tiez-ui.exe
  Tauri + React + WebView2
  starts on hotkey/tray request, hydrates, exits after idle
```

This fully releases the UI process group while retaining React feature parity. It also isolates WebView crashes. The trade-off is a new IPC contract, startup coordination, updater/bundle changes, process supervision, and stricter secret/data-boundary design. It should be evaluated only after same-process suspend and destroy/recreate measurements, because those preserve Tauri's simpler deployment model.

## Native and alternative-engine candidates

| Candidate | What it preserves | Resource expectation | Main costs and risks | Recommendation |
| --- | --- | --- | --- | --- |
| WebView2 memory target `Low` | Full React/Tauri state | Unknown until Windows measurement; best-effort memory reduction | Runtime-version check, Tauri forwarding gap | **Prototype first** |
| WebView2 `TrySuspend` | Loaded React document; timers paused | Better idle CPU and reclaim potential than hiding | Windows COM integration, accidental auto-resume, best-effort result | **Prototype second** |
| Destroy/recreate WebView2 | Full React source and Rust services | Highest reclaim potential within one process | hydration, focus, race, event-gap, multi-window complexity | **Continue Stage B** |
| Core/UI process split | Full React feature surface; UI exits entirely | Strongest deterministic release of WebView process group | IPC, packaging, startup, supervision | **Architectural fallback** |
| Slint hybrid | Direct Rust integration; native-rendered main list | Synthetic PoC was small, but no full-product Windows result | rewrite main UI, custom widgets, rich HTML/image preview, accessibility, IME, drag/drop, theme parity | **First native fallback** |
| egui/eframe | Rust and cross-platform | Likely small at idle; no TieZ evidence | immediate-mode layout, very large virtualized history, custom visual language, interfaces still in flux | Reject for primary product UI |
| iced | Rust, Elm-style state model, cross-platform | No TieZ evidence | project describes itself as experimental; full rewrite and custom-rendered parity work | Reserve candidate |
| WinUI 3 | Best Windows-native controls and Fluent behavior | Must be measured; not automatically tiny | C#/C++ and XAML-centric integration, Windows-only UI, second process/FFI boundary, packaging divergence | No-go unless Windows-only strategy changes |
| CEF | Existing web UI could theoretically be ported | Chromium multi-process architecture remains | bundles Chromium, larger distribution/update/security burden, Tauri does not support swapping to CEF today | Worse than WebView2 for this goal |
| Ultralight | HTML/CSS-like UI and potentially lighter renderer | Claims require a TieZ-specific measurement | commercial/limited free tiers, non-Chromium compatibility, React/web API gaps, new bindings and packaging | No-go for community fork |
| Sciter | Compact desktop-oriented HTML/CSS engine | Must be measured | own HTML/CSS/JS runtime with documented web-standard differences; React/Tauri frontend is not a drop-in | No-go without a separate rewrite experiment |

### Why CEF is not a replacement win

CEF embeds Chromium and therefore keeps the browser/renderer/GPU multi-process model that causes much of the resource footprint. Unlike WebView2 Evergreen, TieZ would also own runtime bundling, updates, security patch cadence, and binary size. WRY's `os-webview` feature notes that it was designed in preparation for possible CEF and Servo ports, but current WRY still requires the operating-system WebView implementation. Tauri cannot switch its Windows backend to CEF today.

### Why Ultralight and Sciter are not drop-in engines

Ultralight has commercial and restricted free tiers. Its free tier advertises limited performance and features and is restricted to qualifying indie use; the Pro tier is priced per application per year. That is a poor dependency for a community-maintained GPLv3 application without a clear sustainability owner.

Sciter explicitly documents that it uses its own HTML/CSS renderer and JavaScript runtime and differs from W3C browser behavior to remain compact and desktop-focused. The existing React/Vite/Tauri frontend uses normal browser and Tauri IPC/event assumptions, so adoption would be a frontend port rather than an engine swap.

### Why Slint is the preferred native fallback

Slint is declarative, Rust-friendly, cross-platform, and available under GPLv3 for an open-source GPLv3 project. The existing PoC already validates list virtualization, filtering, navigation, placeholders, hide/show, and component destruction. It is therefore the only native candidate with TieZ-specific evidence.

The correct next Slint experiment is not a full rewrite. Build one real product slice that consumes the current Rust repository and paste commands:

- real latest history query with positive and negative IDs;
- text, code, URL, image, file, and rich-text placeholders;
- search, keyboard navigation, pin/tag/delete, and plain/rich paste;
- real tray/hotkey wake and paste-to-origin;
- Windows IME, Narrator, DPI, focus, drag/drop, and 100-cycle lifecycle validation;
- advanced settings and rich HTML preview remain on-demand React windows.

Only compare this real slice with the matched Tauri prototype. Do not compare the synthetic 14 MiB PoC with the complete application.

## Proposed experiment sequence

### Experiment 1: hidden WebView2 memory target

Add an internal-only runtime mode:

```text
TIEZ_EXPERIMENT_WEBVIEW_IDLE_MODE=hidden|memory-low
```

For the same binary, data, WebView2 user-data directory, services, and host state:

1. show and fully hydrate TieZ;
2. hide it;
3. switch to `Low` only in the experimental cell;
4. sample the complete root process tree and attributed WebView2 process group at 5 and 30 seconds;
5. restore `Normal`, show, focus, and measure usable wake;
6. repeat at least five independent memory runs and 100 wake cycles.

Promote to an opt-in idle mode if it materially improves private bytes or commit without regressions, even if it does not reach the 40% destroy gate. It can be a safe intermediate feature.

### Experiment 2: hidden WebView2 suspend

Add:

```text
TIEZ_EXPERIMENT_WEBVIEW_IDLE_MODE=suspended
```

Use the same measurement protocol. Record suspend completion and verify that no hidden frontend traffic resumes the renderer. Require one authoritative refresh before the ready marker.

### Experiment 3: destroy/recreate

Run the already-committed Stage B protocol. Compare hidden, memory-low, suspended, and destroyed states on the same Windows 11 machine. The current pair schemas may need a generalized mode field or separate pairwise reports.

### Experiment 4: process isolation, only if needed

Build a minimal `tiez-core`/`tiez-ui` launch-and-hydrate tracer before changing product packaging. Measure UI cold start, named-pipe handshake, complete process release, and update/restart behavior.

### Experiment 5: real Slint slice, only if WebView routes fail

Reuse the existing PoC crate but replace synthetic rows with real repository/command adapters. Keep the scope small enough to produce comparable Windows evidence before committing to a UI architecture.

## Decision rules

- **Ship memory-low** if it has meaningful, repeatable savings and near-zero behavioral regressions, even below 40%.
- **Prefer suspend** if it substantially beats memory-low and wake remains effectively instant.
- **Prefer destroy/recreate** if it reaches at least 40% complete-process-set private-memory savings, meets wake limits, and passes all lifecycle/focus/accessibility gates.
- **Prefer core/UI process isolation** if deterministic process release is required and same-process destruction leaves shared WebView2 processes resident or proves unreliable.
- **Prototype Slint** only if WebView lifecycle and process isolation cannot satisfy resource goals or if the project separately decides that a native UI is strategically valuable.
- **Do not adopt CEF, Ultralight, or Sciter** merely to reduce idle memory; each exchanges a known, evergreen system runtime for a new engine, compatibility surface, and maintenance obligation without TieZ-specific evidence.
- **Do not adopt WinUI 3** unless the project intentionally becomes Windows-first and accepts a second-language/build pipeline.

## Primary sources

- Microsoft, [WebView2 process model](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model)
- Microsoft, [`ICoreWebView2_19::MemoryUsageTargetLevel`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19)
- Microsoft, [`ICoreWebView2_3::TrySuspend` and `Resume`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_3)
- Microsoft, [Distribute the WebView2 Runtime](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)
- Tauri, [Process model](https://v2.tauri.app/concept/process-model/)
- Tauri, [`Webview::with_webview` source, v2.10.2](https://github.com/tauri-apps/tauri/blob/tauri-v2.10.2/crates/tauri/src/webview/mod.rs)
- WRY, [`MemoryUsageLevel` and `WebViewExtWindows`, v0.54.1](https://github.com/tauri-apps/wry/blob/wry-v0.54.1/src/lib.rs)
- Slint, [official repository and licensing](https://github.com/slint-ui/slint)
- iced, [official repository](https://github.com/iced-rs/iced)
- egui, [official repository](https://github.com/emilk/egui)
- Microsoft, [WinUI 3](https://learn.microsoft.com/en-us/windows/apps/winui/winui3/)
- Chromium Embedded Framework, [official repository](https://github.com/chromiumembedded/cef)
- Ultralight, [official pricing and license tiers](https://ultralig.ht/pricing/)
- Sciter, [engine introduction](https://docs.sciter.com/docs/intro/)
