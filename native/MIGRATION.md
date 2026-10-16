# Complete Windows native migration

## Completion rule and source inventory

The final target is a full replacement using WinUI 3 + one Rust core DLL, without WebView2, Tauri startup, React/Vite rendering, hidden HTML hosts or another browser engine. A passing stage is not a complete replacement. Preserve legacy sources and production data until migration/rollback and full acceptance pass. Do not drop a capability because its native implementation is difficult.

[compatibility.json](compatibility.json) inventories the current sources: 132 registered commands, 78 seeded setting keys, 170 conservative setting-like candidates, 39 UI modules and 7 built-in themes. Regenerate with Python: native/tools/inventory.py; verify with the same command plus --check. Candidate settings require manual classification (including internal sync state, literals that are not settings, and keys without defaults). Dynamic calls, plugins, hooks, menus and background services require the behavioral checks below too. All entries remain pending unless the ledger explicitly describes their implementation and evidence.

Five literal frontend calls are absent from the current command registry: get_download_url, save_temp_image, set_display_ip (file transfer), restore_last_focus (focus helper), and set_dock_visible (platform-specific settings). Four have existing Rust functions. These are legacy wiring gaps to resolve during migration, not permission to omit the corresponding capabilities; dock behavior must be classified for the Windows target.

## Capability and acceptance ledger

| Capability | Legacy authority | Native state / required acceptance |
| --- | --- | --- |
| Native entry, shared core, separate main/settings lifecycle | app/setup.rs, app/window_manager.rs, main.tsx | Partial: WinUI main/history plus independent settings share one Rust core. Fourteen settings, read-only binding, conflicts/failed drafts and capture start/pause/restart/settings reopen/exit are covered by 39-phase window smoke. Full localization, remaining windows and settings remain pending. |
| Capture and bounded worker, echo suppression | services/clipboard_listener.rs, services/clipboard/mod.rs | Partial: core now owns the bounded Windows listener and capture worker, explicit start/stop/status ABI, startup sequence baseline, policy revision acknowledgement and stop/Drop/join. Seven synthetic-input worker tests and real-DLL/window lifecycle checks pass; echo markers are local to the service. Real-content OS capture, native copy/paste echo wiring and delayed-provider acceptance remain pending. |
| Pipeline cleanup, per-app policies, sensitive rules, dedup | services/clipboard/pipeline.rs, utils.rs | Partial: same-source discovery/transform/probes run in the native worker with validated runtime capture/privacy/cleanup policies, rich-format burst suppression and history dedup. Content default/serial portable suites pass 107 tests each; core default/portable suites pass 27 each; legacy Windows suite passes 277. Initial parallel localhost-image fixture flake remains unexplained. Native clipboard round trips, sync/distribution and full performance acceptance remain pending. |
| Text, code, URLs, HTML/rich text, images, files/video paths | windows_api/win_clipboard.rs, services/clipboard_ops.rs | Partial: real multi-format reader is wired into the core worker with sequence/cancellation guards, shared production probes and original rich-format/animation preservation. WinUI exposes file/rich-text capture policy. Format-by-format OS round trips, format-aware native preview/paste and target-app acceptance remain pending. |
| Migrations, hashes, attachments, limits, tombstones | database.rs, infrastructure/repository/* | Partial: native/storage compiles the same schema/repository/DPAPI sources; 39 tests pass. Capture save, encryption/OCR purge and count eviction share one transaction; owned attachment cleanup is deferred until commit. Equal timestamps evict by ascending ID. Copied-data migration/rollback and full tombstone/network acceptance remain pending; production data remains untouched. |
| Session-only history | app_state.rs, pipeline.rs, clipboard_cmd.rs | Partial: core worker and history cover negative IDs, cap 500, dedup, persistence/pin/tag promotion and rollback preserving sessions/revisions. WinUI polls ID/revision changes. Real capture-to-UI promotion and rapid live-content acceptance remain pending. |
| History pages/counts/search/type/tag filtering | history_cmd.rs, useHistoryFetch.ts, useFilteredHistory.ts | Partial: core ABI exposes bounded summary pages, full-content fetch, filters/decrypted/OCR search, session merge and revision guards. WinUI incremental virtualized ListView/search/type/exact-tag/pinned controls are implemented; real-window smoke covers search/type/tag and 241 synthetic records (200 loaded, 33 realized containers). Empty-result and stale-read guards pass. Real large-history/DPI/IME acceptance and optimized projections remain pending. |
| Pin/order/tag/color, create/rename/delete tags | clipboard_cmd.rs, history_cmd.rs, tag_repo.rs | Partial: core pin/tag changes and ID promotion, transactional encryption/OCR purge, bounded change feed and pending-sync flag. WinUI pin/tag editing, sensitive conceal/reveal and change-feed redaction pass real-window smoke. Global tag/order/color APIs, their UI and actual sync worker remain pending. |
| Edit/manual entry/delete/clear/batch selection | ClipboardItem.tsx, useClipboardActions.ts, repositories | Partial: core manual entry/edit/single delete, ID results and failure/no-op tests. WinUI add/edit/confirmed single delete, failed-save draft preservation, original whitespace/unchanged CRLF and selection reconciliation pass real-window smoke. Batch/clear remain pending. |
| Rich/plain/direct/transient paste, open/copy | clipboard_ops.rs, content_handler.rs | Pending: destination focus verified before input; preserve formats/files and verify in controlled target apps. |
| Sequential queue, quick paste, modifiers | paste_queue.rs, app/hooks/*, useKeyboardNavigation.ts | Pending: queue/order, delete-after-paste, move-to-top, number navigation and repeat handling. |
| Hotkey registration/conflicts/recording | hotkey_cmd.rs, app/hooks/*, useHotkeyConfig.ts | Pending: native registration, rollback on conflict, all paste/search/relay/queue keys and recording exclusion. |
| Tray/show/hide/focus/pinning/blur/edge/follow mouse | window_manager.rs, setup.rs, window_tracker.rs | Pending: own-PID exclusion, target restoration, no duplicate hooks, silent start, tray and explicit quit. |
| Compact preview, advanced settings, dialogs/overlays | CompactPreviewWindow.tsx, AdvancedSettingsWindow.tsx, useOverlays.ts | Pending: separate native windows/state, preview readiness/cancellation; cover every UI module in inventory. |
| Rich previews/tables/links/images | HtmlContent.tsx, richTextSnapshot.ts, TABLE-PASTE-AUDIT.md | Pending: native structured document + RichTextBlock/Grid/Image. Preserve original rich formats. Complex CSS fidelity is unresolved, not waived; preview and paste need separate acceptance. |
| Drag/drop, file save, open-with/apps/icons | windows_api/drag_drop.rs, apps.rs, file_cmd.rs | Pending: native OLE drag/drop, app picker/icons and shell actions. |
| All settings/validation/side effects/reset | settings_cmd.rs, useSettings*.ts, settings components | Partial: four prototype keys. All other keys and runtime side effects pending; errors/conflicts/partial side effects must be explicit. |
| Built-in/custom themes/backgrounds/materials | THEME-SYSTEM.md, themes.ts, styles/themes/* | Pending: seven themes, semantic XAML resources, light/dark/system and all interaction/accessibility states, presets and materials. |
| Theme store/auth/upload/rating/search | features/theme-store/* | Pending: native store client; CSS package conversion/compatibility design required, no browser host. |
| Localization/input/accessibility | locales.ts, useInputFocus.ts | Pending: zh/en/tw resources, IME composition, keyboard/Narrator/focus/high contrast/reduced motion. |
| Sensitive encryption/jobs/OCR purge | encryption.rs, encryption_queue.rs, sensitive_align.rs | Pending: exact DPAPI/portable compatibility, failure behavior, plaintext OCR removal and stop/join; no secret logging. |
| Backup/inspect/pending restore/data paths/portable | backup.rs, setup.rs, system_cmd.rs | Pending: safe archives, copied fixture migration, attachments/encryption and rollback before real data, restart semantics. |
| MQTT/cloud/WebDAV | mqtt_sub.rs, cloud_sync.rs, sync modules | Pending: credentials/TLS, snapshots/blobs/cursors/tombstones/hash versions, conflicts/offline, controlled-peer acceptance. |
| Clipboard relay/keys/replay ledger | clipboard_relay.rs, relay_key.rs, relay_cmd.rs | Pending: shared-key lifecycle, authentication/expiry/replay, rich/image/file payloads and native status/settings. |
| LAN transfer/chat/received files | services/file_transfer/*, features/file-transfer/* | Pending: native desktop interface plus protocol/server behavior. Remote HTTP clients remain external peers, never desktop UI renderers or a local WebView dependency. |
| Emoji favorites/import/remote images | features/emoji/*, file_cmd.rs | Pending: native picker, favorites/cache/download, animation and clipboard/file formats. |
| OCR/QR/image analysis | image_analysis.rs, useAiActions.ts | Pending: native results, Windows OCR/QR and sensitive-content exclusion. |
| AI profiles/connectivity/transforms | ai_cmd.rs, AiProfileModal.tsx, useAiActions.ts | Pending: native profile/secret controls, requests/cancellation/errors, all actions and output copy/paste. |
| Announcements/update/help/notifications/sounds | useAnnouncements.ts, useAutoUpdate.ts, ui_cmd.rs, useSoundEffects.ts | Pending: native UI, controlled endpoints/signatures, notification settings and user shell actions. |
| Windows autostart/admin/registry integration | system_cmd.rs, settings_cmd.rs | Pending: clipboard policy, Win+V optimization, Explorer/admin restart, autostart, consent/errors. |
| DPI/multimonitor/large-history/performance | window_manager.rs, window_tracker.rs, native host | Pending: attach/detach, mixed DPI/edge docking, large virtualized history, repeated window cycles, controlled same-condition measurements. |
| Production package and WebView-free clean OS | workflows, Tauri config, native host | Partial: native debug output excludes WebView2 and passes windows smoke; installer, clean OS and all-feature acceptance, updater replacement remain pending. |

Source paths refer to src-tauri/src or src unless explicit. Offline tests cannot establish installed-data migration, live native behavior, multi-peer services or complete replacement.

## Implementation order

1. Same-source model/schema/repositories/DPAPI in a Tauri-free storage crate; isolated SQLite/synthetic compatibility tests.
2. Core-owned history and C ABI: pages/full content, IDs, CRUD/pin/tags, revisioned change feed/bounded payloads; native searchable virtualized list and large-history checks.
3. Format capture/pipeline, listener, target tracking/paste, stop/join; native tray/hotkeys/queue; controlled-target round trips.
4. All windows/previews/settings/resources and native rich rendering; remaining native feature interfaces. Side effects require rollback or explicit partial failure.
5. Crypto jobs, backups/migration, sync/relay/transfer/AI/store/system features; local/controlled-peer evidence for each.
6. Native-only packaging; copy migration/backup/rollback, then authorized real-data transition. Clean Windows without WebView2, full functional/DPI/IME/accessibility/performance acceptance before changing production defaults.

## Open design risks

- DOM/SVG snapshots have no direct XAML equivalent. Use structured native previews and original-format paste; determine complex table/CSS fidelity with representative fixtures before acceptance.
- CSS store packages cannot be loaded as XAML themes. Map semantic tokens and provide a conversion/native package path; do not drop store capabilities.
- Tauri AppHandle/State/events currently couple services to the shell. Replace these with core-owned state/events and explicit host services. C# must not independently own database/encryption/business settings.
- Initial settings smoke checks do not validate capture/paste, network peers, registry changes or real user data.
