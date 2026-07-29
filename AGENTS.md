# Repository Guide for Agents

## Project priorities

TieZ is a community-maintained Tauri 2 clipboard manager. The maintenance policy favors reproducible builds, release reliability, focused bug fixes, and small reviewable diffs before broad feature work. Read `CONTRIBUTING.md` before changing contributor or release behavior; update `CHANGELOG.md` for changes visible to users or contributors.

## Essential commands

Run commands from the repository root unless noted otherwise.

| Task | Command | Notes |
| --- | --- | --- |
| Install exactly from the lockfile | `npm ci` | This is what CI uses. `npm install` is the documented first-time setup command. |
| Run the complete desktop app | `npm run tauri:dev` | Runs `tauri dev --features devtools`; requires the platform's Tauri 2 prerequisites. |
| Run only the Vite frontend | `npm run dev` | Fixed port `1420`; this does not provide the Rust commands/events used by most features. |
| Type-check and build the frontend | `npm run build` | Runs `tsc && vite build`; output is `dist/web`. This is the documented build check. |
| Run frontend unit tests | `npx vitest run` | There is no `test` package script or Vitest config. |
| Run Rust tests | `cargo test --manifest-path src-tauri/Cargo.toml` | Tests the library and the real desktop binary. On non-Windows systems this compiles platform stubs, not Windows behavior. |
| Preview the built frontend | `npm run preview` | Vite preview only. |
| Build a local Tauri bundle | `npm run tauri:build` | The default bundle target in `tauri.conf.json` is NSIS. CI overrides targets per platform. |

There is no configured lint script, formatter configuration, standalone type-check script, or end-to-end test suite. `rustfmt` and `clippy` are installed by `rust-toolchain.toml`, but neither is run by the current CI workflows.

Expected baseline behavior:

- `npm run build` succeeds, with an existing Vite warning about mixed static/dynamic imports of `@tauri-apps/api/event` and a large main chunk.
- `cargo test --manifest-path src-tauri/Cargo.toml` succeeds but emits existing platform-dependent unused-code warnings on non-Windows hosts. Do not treat those baseline warnings as failures caused by an unrelated change.

## Release and packaging workflow

- `.github/workflows/build-platforms.yml` is manual-only. It builds unsigned Windows x64 NSIS, Linux x64 DEB, and macOS ARM/Intel app bundles. It uses `.github/tauri-build-test.conf.json` to disable updater artifacts during build validation.
- `.github/workflows/release.yml` runs for `v*` tags or manual dispatch. It builds Windows NSIS, Linux DEB/AppImage, and macOS app/DMG bundles and creates a draft GitHub release.
- Release builds enable the updater and require `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Do not enable production updating until the endpoint, public key, and signing infrastructure are controlled by this fork; `.env.example` intentionally defaults `VITE_ENABLE_UPDATER=false`.
- The application version is duplicated in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`. Keep all four synchronized.
- The README still describes Linux as upcoming, while current build and release workflows produce Linux artifacts. Treat workflow configuration as the packaging implementation and update documentation deliberately if platform support changes.

## Architecture

### Process boundary

The React frontend in `src/` is a Tauri webview client. The Rust application in `src-tauri/src/` owns clipboard capture, persistence, native windows and hotkeys, paste behavior, file transfer, backup, encryption, and synchronization.

The boundary has two forms:

1. Frontend actions call Rust with `invoke("command_name", payload)`.
2. Rust publishes state changes with Tauri events; frontend hooks subscribe with `listen(...)` and clean up the returned unlisten functions.

The full desktop entry point and command registry are in `src-tauri/src/main.rs`. `src-tauri/src/lib.rs` is still a minimal template/mobile-style `greet` implementation and is not the desktop startup path. When adding a desktop command, define it with `#[tauri::command]`, expose its module as appropriate through `src-tauri/src/app/commands/mod.rs`, and add it to `tauri::generate_handler!` in `main.rs`; an `invoke` name alone does not register it.

Rust models serialize with snake_case, and frontend transport types intentionally preserve that shape (`content_type`, `source_app`, `is_pinned`, and so on). Do not add a camelCase conversion layer for individual fields unless the whole boundary contract is changed together.

### Frontend organization

- `src/main.tsx` selects one of three roots from the `window` query parameter: the main app, `compact-preview`, or `advanced-settings`. Changes to global styling, themes, or shared event behavior may affect all three webviews.
- `src/App.tsx` is the main composition root. Large state ownership is centralized in `features/app/hooks/useAppState.ts`; behavior is split into focused hooks in `src/shared/hooks/` and feature components under `src/features/`.
- `src/features/` owns feature-specific UI and types: app shell, clipboard, settings, emoji, tags, file transfer, and theme store.
- `src/shared/` owns cross-feature components, hooks, transport types, configuration, and pure utilities. Shared transport types are re-exported from `src/shared/types/index.ts`.
- Most frontend state starts as local React state, is initialized from Rust settings/commands, and is kept current through invoke results and Tauri events. Preserve both sides of that flow when adding settings or backend-driven state.
- Browser-only rendering is partially supported through `isTauriRuntime()` guards, but a Vite-only session cannot exercise native behavior.

Important event contracts include:

- `clipboard-updated`: insert or replace an entry in the UI.
- `clipboard-removed`: remove an entry by numeric ID.
- `clipboard-changed`: trigger broader history refresh/synchronization.
- `file-server-status-changed`: refresh file-server state.

Follow the existing ref-backed listener pattern in `useClipboardEvents.ts` when callbacks change frequently; it avoids tearing down subscriptions on every render while still invoking current callbacks.

### Rust organization and startup

- `app/`: Tauri command modules, startup orchestration, global hooks, and window management.
- `domain/`: serialized domain models.
- `infrastructure/repository/`: SQLite clipboard, settings, tag repositories, and schema migrations.
- `infrastructure/windows_api/` and `windows_ext.rs`: native Windows integrations. `infrastructure/mod.rs` supplies non-Windows stubs so cross-platform builds compile.
- `services/`: clipboard processing and paste operations, backup, cloud/MQTT sync, encryption, image analysis, file transfer, and paste queues.
- `app_state.rs`: Tauri-managed synchronized runtime state.
- `database.rs`: database initialization, shared data helpers, sensitive-key/tag rules, and defaults; new persistence logic generally belongs in repositories rather than adding more direct database access here.

`app::setup::init` resolves the data directory, applies a pending restore before opening SQLite, initializes logging and migrations/defaults, manages shared state, configures the main window, starts services, creates the tray, and applies the theme. Data-path resolution has two non-obvious overrides: `datapath.txt` can redirect storage, and an existing `data/` directory beside the executable activates portable storage.

SQLite is opened in WAL mode and migrations run before defaults are seeded. Repository/database tests commonly use in-memory SQLite and explicitly run the schema setup they need.

### Clipboard data flow

1. `services/clipboard_listener.rs` receives clipboard changes. Windows uses a message-only native window and a bounded worker so clipboard reads never block `WM_CLIPBOARDUPDATE`; non-Windows uses 500 ms text polling.
2. `services/clipboard/mod.rs` reads native formats, suppresses app-originated echoes, converts data to `ClipboardData`, and calls `process_new_entry`.
3. `services/clipboard/pipeline.rs` runs ordered stages: discovery, transformation, validation, persistence, then distribution.
4. Transformation applies cleanup policies, privacy tagging, line-ending normalization, and rich-text image handling. Validation handles paste echoes and deduplication. Persistence writes through the repository or stores session-only entries. Distribution updates queues, emits UI events, and requests cloud sync.
5. React hooks consume events and reconcile the displayed history; direct user actions call command/service handlers through `invoke`.

Do not trim captured text as a generic normalization step. The pipeline and text hash deliberately normalize CRLF/CR to LF while preserving leading and trailing whitespace, and regression tests protect this behavior.

Persistence mode changes ID semantics. Persisted entries have positive SQLite IDs; session-only entries use generated negative IDs, are capped at 500, and may be converted to persisted entries when pinned or tagged. Commands such as pin/tag updates can therefore return a replacement ID. Frontend callers must use that returned ID rather than assuming identity is stable.

## Code conventions

- React components and component files use PascalCase; hooks use `useXxx`; TypeScript variables/functions use camelCase.
- Rust modules and functions use snake_case; structs, enums, traits, and pipeline stages use PascalCase.
- Keep feature-specific code under its feature and promote code to `shared/` only when it genuinely crosses features.
- TypeScript is strict and rejects unused locals/parameters and fallthrough cases. Prefer type-only imports where the surrounding file does.
- Formatting is not globally normalized: most current frontend files use two spaces and double quotes, while some settings files use four spaces and single quotes. There is no ESLint or Prettier authority, so match the file being edited and avoid formatting unrelated lines.
- The repository normalizes text to LF; `.bat`, `.cmd`, and `.ps1` files retain CRLF through `.gitattributes`.
- Keep diffs focused. Existing source includes Chinese UI strings, tests, and comments; do not translate unrelated text as cleanup.

## Testing approach

Frontend unit tests are colocated as `*.test.ts`, import `describe`, `it`, and `expect` directly from Vitest, and emphasize explicit edge cases. The current suite is `src/shared/lib/utils.test.ts`; no browser or Tauri mocks are configured.

Rust unit tests live in inline `#[cfg(test)] mod tests` blocks near the implementation. Existing patterns include:

- in-memory SQLite for database and repository behavior;
- temporary directories with unique names for filesystem/sync tests;
- narrow regression tests for platform/source detection, rich clipboard formats, whitespace, CRLF normalization, backup validation, and sync conflict handling.

Validate at the narrowest useful level first, then run both `npm run build` and the applicable unit-test command when a change crosses the frontend/Rust boundary. Native clipboard, focus, material effects, hooks, and installer behavior still require validation on the affected operating system because non-Windows stubs intentionally return empty/no-op results.

## Themes and CSS

`docs/THEME-SYSTEM.md` is the authority for theme work. The constraints most likely to be missed are:

- Shared component CSS must remain theme-neutral. Express color, shape, shadow, spacing, and typography differences through tokens first; theme-specific effects stay in the theme stylesheet.
- Register built-in themes and localized labels in `src/shared/config/themes.ts`, not `src/locales.ts`.
- Theme CSS files in `src/styles/themes/` are eagerly auto-loaded by `load.ts`; do not add a manual import for each new pure-CSS theme.
- A pure CSS theme needs registration plus `src/styles/themes/<id>.css`. A theme requiring native window material must also extend `set_theme` in `src-tauri/src/app/commands/ui_cmd.rs`.
- Verify theme changes in the main window and compact preview, including light/dark and interactive states.

## Security, privacy, and configuration gotchas

- Never commit `.env`, databases, logs, signer keys, or generated build output. `.env.example` is the tracked environment template.
- The frontend edition defaults to `cloud`; only the literal `VITE_EDITION=local` disables cloud-sync UI behavior.
- Sensitive setting keys and built-in sensitive tag names are security behavior, not display-only constants. The Rust list in `database.rs` and frontend privacy checks in `App.tsx`, `useClipboardItemRenderer`, and `ClipboardItem` must remain aligned. Current built-in tags are `sensitive`, `密码`, and `password` with case-insensitive Rust matching.
- Sensitive-tag changes can enqueue encryption/decryption and remove plaintext OCR analysis. Preserve that transition when changing tag commands.
- The Tauri capability list and asset-protocol scopes in `src-tauri/tauri.conf.json` constrain frontend filesystem, URL, window, updater, and process access. New plugin/API use may require an explicit capability update.
- Hosted API, announcement, updater endpoint, and signing configuration were inherited or fork-specific infrastructure. Review ownership before enabling or publishing it.
- Cloud sync is requested after persistent clipboard mutations. Mutating repository state without the corresponding event/sync behavior can leave the UI or remote state stale; follow nearby command patterns.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **tiez-clipboard** (3860 symbols, 9347 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "master"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({search_query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/tiez-clipboard/context` | Codebase overview, check index freshness |
| `gitnexus://repo/tiez-clipboard/clusters` | All functional areas |
| `gitnexus://repo/tiez-clipboard/processes` | All execution flows |
| `gitnexus://repo/tiez-clipboard/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
