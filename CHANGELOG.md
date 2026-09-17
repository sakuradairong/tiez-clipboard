# Changelog

All notable changes to this fork will be documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- 修复 Windows 上 GIF 复制进历史后粘贴仍变成 PNG 的问题：粘贴 GIF 时不再附带 PNG 剪贴板格式（避免目标应用优先取静态 PNG）；从 CF_HTML 恢复的动画 GIF 改为按图片条目保存；关闭“捕获文件”时仍会从 CF_HDROP 保留单个 GIF。

## [0.3.11] - 2026-09-16

### Fixed

- 修复 Windows 从浏览器复制动态图片时 GIF 不显示、伪装为 `.jpg`/`image/jpeg` 的 GIF 被降级为静态图的问题；现在会从 CF_HTML 恢复原始动画，并按实际字节签名持久化附件格式。
- 修复 Windows 上使用自定义/便携数据目录时，已外置保存的图片在重启、切换分类或重新加载历史后因 Tauri 资源协议权限不足而消失的问题；运行时权限仅覆盖 `attachments` 与 `emoji_favorites` 图片目录。
- 修复富文本复制后粘贴会在相邻块元素之间重复插入空行的问题；优先保留来源提供的有效纯文本及其首尾空白，并为 HTML-only 内容按块边界生成纯文本。
- 修复位于用户目录外的自定义背景在重启后消失、必须调整透明度才看似恢复的问题；启动时重新授权已保存图片，保存时限制为受支持的实际图片文件，缺失/不可读时安全回退并反馈，主窗口与高级设置窗口同步更新。
- Startup race where the history list could appear empty after a reboot: the main window can load before Rust setup finishes managing the database state, causing the initial history fetch to fail with no retry. The frontend now retries failed history fetches with backoff, refreshes on the `app-ready` event emitted when backend setup completes, and polls the new `is_app_ready` command as a fallback once retries are exhausted.
- 修复升级后读取错误数据目录、旧历史看似丢失的问题。历史提交 972cb86 将应用标识从 `com.tiez.app` 改为 `com.tiez`，默认数据目录随之从 `%APPDATA%\com.tiez.app` 变为 `%APPDATA%\com.tiez`，旧目录中的历史记录不再被使用。现在启动时会按以下非破坏性规则兼容旧标识目录（`src-tauri/src/migration.rs`、`src-tauri/src/app/setup.rs`）：
  - 显式配置优先：启动先读取当前 `datapath.txt` 重定向（含无效内容）与可执行文件旁的 `data/` 便携目录再执行迁移，优先级保持便携 > 重定向 > 默认；存在任一显式配置时，所有数据迁移整体跳过，**绝不覆盖已有重定向**（重复启动同样安全）。
  - 旧标识目录自身的 `datapath.txt` 按旧版运行时语义解析：重定向有效时实际数据目录是重定向目标（即使 `com.tiez.app` 目录里已没有数据库），无效/为空时回落到旧目录本身；解析出的数据目录没有数据库则视为无数据。
  - 仅旧数据目录有数据库（新目录未初始化）：自动写入 `datapath.txt` 指向旧目录，**原地采用**——不复制、不移动、不删除任何文件，WAL/SHM、附件及数据库内的绝对路径全部保持一致。
  - 两边都有数据库：不按体积或修改时间静默覆盖、合并或删除；弹出“是/否”对话框让用户明确选择（“是”写入重定向采用旧数据，“否”保持当前目录），不选择则双方数据原样保留。
  - v0.2.8 的“贴汁”目录迁移移除了按文件大小替换数据库并递归删除旧目录的逻辑：默认目录不存在时仍整目录重命名；否则同样以原地重定向采用或保留双方数据。旧安装目录清理增加护栏：目录本身或其 `data/` 子目录（便携模式）含 `clipboard.db`、`datapath.txt` 或 `attachments` 时一律跳过；当前数据目录、显式重定向目标、便携目录及其任何祖先目录也不会被当作安装残留删除。**候选与受保护目录在比较前先解析为真实路径**（消除 8.3 短路径、`..`、`\\?\` 扩展前缀、目录联接等 Windows 路径别名），任何一方无法解析、候选为符号链接/联接时即跳过删除（fail closed），不回退到字符串比较；受保护目录**存在性无法判定**（权限不足、元数据错误等，与“明确不存在”严格区分）或运行中程序目录不可得/无法解析时整体放弃目录清理；运行中程序目录的祖先目录同样受保护。
  - **手动恢复指引**：若升级后历史“消失”，旧数据通常仍在 `%APPDATA%\com.tiez.app`。在本机 `%APPDATA%\com.tiez\datapath.txt` 写入旧数据目录绝对路径（单行、无引号）即可切回旧数据；也可在 设置 → 数据目录 中切换（该流程会完整迁移数据库、WAL/SHM、附件并重写库内绝对路径）。升级前请自行备份两个数据目录。

## [0.3.10] - 2026-09-12

### Added

- Collapsible pinned section in the clipboard history: a header with item count toggles the pinned block, the collapsed state persists across restarts (`app.pinned_collapsed`), and keyboard navigation skips hidden pinned items while collapsed.

### Fixed

- Keyboard navigation no longer selects or pastes hidden pinned entries when the pinned section is collapsed and no visible history entries remain.
- Turning off “Show Source App Icon” now hides the source app name in clipboard items and the compact preview header, not just the icon.

## [0.3.9] - 2026-08-18

### Added

- Sound effect preview button in settings so volume can be verified before copying.
- Setting to choose whether number quick paste stays active while the window is edge-docked/hidden.
- Per-item plain text paste action on text and rich-text clipboard entries.

### Changed

- Documented that active development and release validation currently target Windows only.

### Fixed

- Restored audible copy/paste sound effects by fixing the volume scale (`0~1` was incorrectly divided by 100).
- Rich-text paste no longer falls back to the preview image snapshot when HTML content is available.
- Duplicate-content merge no longer clears tags when updating an existing persisted entry.
- Silent startup and `--minimized` autostart now hide the main window instead of leaving it visible.
- Settings hotkey labels no longer force macOS symbols on Windows.
- Global hotkey re-sync no longer fails with `Win+V already registered` when the same shortcut is registered twice during sync.
- Pressing Backspace while recording a shortcut now clears it instead of trying to register `Backspace` as a global hotkey.

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
