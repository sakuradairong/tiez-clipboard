# Changelog

All notable changes to this fork will be documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Extracted Tauri-independent `PasteCoordinator` and `UiLifecycle` policies into `tiez-core` for the WinUI main-window slice (payload planning, paste-queue cap, hotkey/Esc/deactivate hide, last-foreground capture).
- WinUI probe can pin/delete against a writable `clipboard.db`, paste Unicode text through the shared coordinator, filter by type chips, and toggle with Alt+C.
- WinUI `paste-rich` writes CF_HTML alongside Unicode text when the history item has HTML; the synthetic adapter includes one rich sample.
- WinUI paste plans image/file payloads separately from text: CF_DIB/PNG for images, CF_HDROP for files. Enter does not paste while IME composition is confirming.
- WinUI copy writes the clipboard without hiding or sending Ctrl+V. Ctrl+Enter pastes rich text; Delete and a card context menu cover pin/delete/copy.
- WinUI live-captures Unicode text, CF_HTML, PNG/DIB images, and Explorer files through the native Rust ABI. Startup clipboard is primed rather than ingested; probe paste/copy is treated as an echo.
- Writable WinUI history can use existing Tauri DPAPI-protected entries for copy and paste while keeping sensitive previews and the details pane redacted; read-only database inspection remains metadata-only.
- WinUI ABI v6 and the native details pane now support searchable item tags, Chinese comma-separated editing, tag chips, and positive replacement IDs when tagging persists a session-only entry.
- WinUI pinned cards can now be reordered by drag-and-drop or the Chinese “上移”/“下移” controls; writable SQLite updates the complete pinned order atomically and rejects stale or filtered partial lists.
- WinUI now owns a native TieZ system-tray icon with Chinese show/exit commands, restores it after Explorer restarts, hides instead of exiting when the main window closes, and exits only from the explicit tray command.
- The native WinUI build no longer imports WebView2 build targets, links its loader, or ships its runtime files; only the compile-time WinMD needed to project Windows App SDK's `IWebView2` declaration remains. The build helper now recreates validated generated-output directories so removed runtime files cannot survive as stale artifacts.
- WinUI ABI v7 exposes an allowlisted, non-secret native settings surface with SQLite and memory adapters. The Chinese settings dialog can immediately apply theme, compact-list, persistence, history-limit, deduplication, file/rich-text capture, privacy, tray visibility, and real always-on-top window behavior.
- WinUI compact mode now opens a no-activate, always-on-top native hover preview for text, rich text, files, images, and protected-entry messages without using WebView2.
- WinUI ABI v8 adds a shared, privacy-preserving open-content plan. The Chinese native UI can open trusted web links, existing files, UTF-8 text, rich text, and images through `ShellExecuteW` without command shells; dangerous URL schemes are rejected and custom protocols or local HTML require confirmation.
- WinUI ABI v9 adds asynchronous Chinese backup export and restore controls backed by a shared `tiez-core` archive implementation. Both frontends now create and validate the same database/attachment archives, while WinUI applies scheduled restores before opening SQLite and retains a rollback copy.
- WinUI ABI v10 adds shared OCR/QR image analysis, an asynchronous Chinese native details panel, result copying, and native history search over cached recognition text. The Tauri commands remain compatible wrappers over the same `tiez-core` implementation.
- WinUI ABI v11 adds Tauri-compatible WebDAV settings, a write-only password field, and an asynchronous Chinese read-only connectivity test. Full background upload/download and conflict reconciliation remain on the native migration roadmap.
- WinUI ABI v12 adds a Rust-owned WebDAV background service, automatic scheduling, immediate manual synchronization, sanitized status polling, and Chinese progress/error/result UI. Its SQLite host preserves sync revisions, tombstones, tags, sensitive DPAPI storage, settings, image attachments, and cross-device emoji images without exposing credentials through C++.
- The Chinese WinUI settings dialog can check the current MSIX package's associated App Installer feed and hand available updates back to Windows for signed installation; unpackaged builds fail closed without contacting a hard-coded endpoint.
- Native Windows CI now builds a versioned `TieZ.exe` and structurally validates a self-contained x64 MSIX. Release publishing requires a matching Publisher certificate, SHA256/RFC 3161 signing, package re-verification, and emits an App Installer update feed plus checksum.
- The Chinese WinUI settings dialog now owns Windows login startup. Installed MSIX builds use the native `TieZStartup` task, unpackaged builds register only the current native executable, and startup activation remains hidden in the system tray.
- Native WinUI launches now register one AppLifecycle main instance before XAML and Rust/SQLite startup. Duplicate startup activations remain hidden, while ordinary duplicate launches exit and reveal the existing tray process; an isolated PowerShell smoke test covers both paths.
- Native Windows validation and signed-release jobs now run the single-instance activation smoke test before packaging; the dedicated WinUI workflow follows `master` instead of the retired exploration branch.
- The native lifecycle smoke test now repeats real activation and `WM_CLOSE`-to-tray cycles while asserting one persistent PID/Rust owner; CI runs ten cycles and release acceptance can run one hundred.
- Native WinUI clipboard cards now expose focusable list-item semantics with Chinese Narrator names and help text; Enter/Space opens details, Shift+F10 opens the existing card menu, child action buttons remain keyboard reachable, and status surfaces are live regions.
- Native Windows CI now runs an isolated OS-level global-hotkey smoke test before packaging, proving keyboard registration ownership, real keyboard and mouse-middle activation, and close-to-tray survival without touching the user's TieZ data or shortcut.

### Changed

- The native WinUI main window now inherits and edits existing `app.hotkey` settings instead of hard-coding Alt+C. Keyboard shortcuts use `RegisterHotKey`, while existing `MouseMiddle`/`MButton` settings use a low-level mouse hook that only posts to the native message window and is removed during switching or teardown. The Chinese settings dialog registers a new input method before persisting it, rolls system registration back if the database write fails, and can disable the shortcut with an empty value. Common modifier/key aliases are accepted, and invalid or conflicting changes preserve the previously working shortcut and saved value.

- WebDAV client construction, safe path encoding, retry policy, atomic publication, blob transfer, and remote listing parsers now live in Tauri-independent `tiez-core`, establishing the reusable transport contract for the native WinUI sync runner.
- Cloud-sync item, snapshot, op-batch, head, content-preference, identity, digest, and revision-collapse rules now share one Tauri-independent protocol model while retaining the existing snake_case remote JSON format.
- Defined the Tauri-independent cloud-sync host boundary for runtime state, SQLite items, payload materialization, settings, emoji, cancellation, and UI events, plus bounded deterministic remote-op and snapshot planning for the native runner.
- Implemented the shared single-pass WebDAV sync runner with atomic head commit points, bounded incremental pull, snapshot/settings reconciliation, verified blob offload, cancellation persistence, and runtime-neutral status events; both Tauri and the native WinUI lifecycle now run through shared host adapters.
- Tauri WebDAV sync now runs through the shared runner and an atomic settings-backed host adapter while retaining the previous implementation behind the local-only `cloud_sync_webdav_use_legacy_runner=true` recovery switch.
- Tauri and writable WinUI database startup now use the same `tiez-core` schema-v15 migration and 77-setting bootstrap; WinUI validates and applies a scheduled restore itself before opening the database instead of requiring the Tauri fallback.
- Tauri and writable WinUI history mutations now share atomic row, normalized-tag, sync-revision, and deletion-tombstone semantics; native file captures use the production `file` content type.
- Writable WinUI capture now follows the production persistence and deduplication settings: session-only entries use replaceable negative IDs with a 500-item cap, while persisted captures reuse matching rows and enforce the configured unprotected-history limit.
- Tauri and the WinUI candidate now share the same default, redirected, and portable data-directory selection policy. WinUI Release builds use production data by default, while Debug/tests remain synthetic and Release can opt back into synthetic diagnostics explicitly.
- Release version validation now covers the WinUI Rust core in addition to the four Tauri version files; native backup metadata, EXE resources, MSIX identity, and the release tag derive from the synchronized product version.
- Native clipboard search now matches cached OCR text and QR payloads without exposing those payloads in list previews, and remains compatible with older read-only databases that do not yet have the analysis table.
- WinUI 主窗口现以中文作为主要界面语言，同时保留内部内容类型和搜索筛选协议不变。
- The native main-window title no longer labels the WinUI runtime as an experiment, and installed startup ownership removes legacy Tauri Run entries to prevent both frontends launching together.
- WinUI Debug builds now use the correct C++/WinRT namespaces in the generated-XAML unhandled-exception hook.

- Documented the WebView2 → C ABI → phase matrix for the native main window. Tauri remains the production fallback; WinUI is a mutually exclusive alternate entry.

### Security

- Cloud settings snapshots now share one fail-closed eligibility policy across Tauri and WinUI, excluding MQTT credentials, AI profiles, relay keys, WebDAV credentials, token/secret/password-style keys, local synchronization state, and device-local autostart/silent-start preferences from upload and remote application.
- Shared WebDAV transport refuses credential-bearing endpoints and cross-origin redirects, encodes path segments defensively, validates blob identities, and bounds remote error bodies before surfacing them.
- Writable WinUI capture now applies the same privacy kinds and custom regular expressions as Tauri, tags matching text as sensitive, and DPAPI-protects content, preview, and rich HTML before committing it.
- WinUI tag changes atomically synchronize normalized tags and revision metadata; `sensitive`, `密码`, and `password` transitions encrypt or decrypt stored payloads, and sensitive transitions remove plaintext OCR indexes.
- Shared backup validation rejects undeclared or duplicate ZIP entries, unsafe paths, oversized payloads, invalid hashes, and checksum mismatches; failed pending restores are quarantined before either frontend opens SQLite.
- Shared image analysis never persists OCR or QR results for sensitive/encrypted entries and rechecks the current row before writing, so a privacy-tag change during background recognition cannot leave a plaintext index behind.
- Native update checks accept only the HTTPS App Installer URI associated with the installed package; Windows remains responsible for publisher-signature verification and applying the upgrade.
- Native WebDAV setup never returns saved passwords through the ABI, commits compatible settings transactionally, requires HTTPS except for loopback tests, rejects credential-bearing URLs and redirects, and uses a read-only `PROPFIND` without writing remote data.
- Native MSIX packaging disables AppData write virtualization and verifies the required capability so installed WinUI builds continue using the Tauri-compatible data directory instead of package-private clipboard history.
- Sensitive WinUI cards suppress their preview value from all newly assigned UI Automation names and announce only that the preview is hidden.

### Fixed

- The native SQLite cloud-sync host now inlines local rich-HTML images before upload and safely removes tombstoned managed attachments only when no clipboard or emoji reference remains.
- `emoji_sync` now uses the same stable text payload hash as the legacy Tauri runner, so native favorites are uploaded once and exact remote payloads can suppress echo uploads.
- The WinUI build and MSIX packaging helpers now run release-version validation from the repository root, so their documented invocations also work from inside `experiments/winui3-main-window`.
- Tauri and the native WinUI candidate now share a per-database Windows ownership mutex, preventing concurrent restore or write access to the same `clipboard.db`.
- Duplicate native WinUI launches no longer reach the Rust database-ownership error path when the tray process is already running.
- Writable WinUI history now copies captured images from its temporary capture area into the TieZ data directory before committing the database row, so system temp cleanup cannot break saved image entries.
- WinUI history deletion and capacity eviction now remove attachment files only after the last live database reference is gone, and fail closed when a surviving encrypted path cannot be inspected.
- WinUI probe now restores WASDK 1.8 split packages, imports the Runtime and MSIX/MRT build assets required by self-contained XAML windows, generates the current app PRI, compiles native sources as UTF-8, finds MSBuild on Visual Studio Build Tools, compiles the Windows paste helper against `windows-sys` 0.59, and assembles the current native EXE with the matching Rust DLL.

## [0.3.8] - 2026-07-27

### Added

- Added explicit desktop clipboard relay shortcuts for sending or fetching one encrypted text item through a separate WebDAV `relay/v1` namespace.
- Added native OS credential-store management for the relay shared key and an authenticated local delivery receipt ledger.

### Security

- Relay requires HTTPS, authenticates message metadata and acknowledgements, and exposes no plaintext content fingerprint or shared key through general settings.

### Fixed

- Restored the Windows-only “Use Win+V Shortcut” setting with persisted state, immediate hotkey switching, and rollback when applying the takeover fails.

## [0.3.7] - 2026-07-26

### Added

- Linux X11 clipboard round-trip integration coverage for text, HTML, images, and file lists
- Native macOS pasteboard revision monitoring and Linux XFixes event monitoring with a resilient polling fallback
- Wayland data-control clipboard support and explicit Linux automatic-paste capability detection
- Cross-platform release preflight builds for NSIS, MSI, DEB, AppImage, macOS app bundles, and DMGs
- Community-maintainer takeover notice in `README.md` and `README.zh-CN.md`
- `CONTRIBUTING.md` for contributor onboarding
- `SECURITY.md` for responsible disclosure guidance
- `.github/pull_request_template.md` for consistent pull requests
- `.github/CODEOWNERS` placeholder for future ownership assignment

### Changed

- Split Windows and non-Windows clipboard capture paths so Linux and macOS no longer depend on Windows API stubs
- Preserve text, rich HTML, image, and file-list clipboard snapshots during transient paste where supported
- Propagate macOS/Linux paste-injection failures before use counts, ordering, or delete-after-paste actions are applied
- Enabled four-platform release documentation and platform-specific local build commands
- Expanded settings, hotkey, appearance, clipboard, database migration, and cloud-sync behavior
- Simplified release workflow to remove a broken portable build/upload path
- Replaced legacy maintainer-facing README support links with fork-maintenance guidance
- Removed outdated issue-template contact links that pointed to legacy upstream infrastructure
- Switched inherited app links and hosted-service defaults to fork-safe configuration and environment variables
- Replaced generic fork placeholders with the maintainer fork URL `sakuradairong/tiez-clipboard`

### Fixed

- Linux automatic paste no longer invokes macOS `osascript`
- Linux and macOS image, file, and rich-text copy operations no longer report success without writing clipboard data
- Linux and macOS direct-text and emoji paste now stages the requested text and safely restores the previous clipboard after a successful paste
- Non-Windows clipboard monitoring no longer exits permanently after a single initialization failure
- Transient paste no longer overwrites a clipboard value changed by the user or target application during restoration
