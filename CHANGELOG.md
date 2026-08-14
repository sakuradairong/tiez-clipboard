# Changelog

All notable changes to this fork will be documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Extracted Tauri-independent `PasteCoordinator` and `UiLifecycle` policies into `tiez-core` for the WinUI main-window slice (payload planning, paste-queue cap, hotkey/Esc/deactivate hide, last-foreground capture).
- WinUI probe can pin/delete against a writable `clipboard.db`, paste Unicode text through the shared coordinator, filter by type chips, and toggle with Alt+C.

### Changed

- Documented the WebView2 → C ABI → phase matrix for the native main window. Tauri remains the production fallback; WinUI is a mutually exclusive alternate entry.

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
