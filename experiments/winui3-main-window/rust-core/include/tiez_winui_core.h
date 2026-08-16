#pragma once

#include <stdbool.h>
#include <stdint.h>

#if defined(_WIN32) && defined(TIEZ_CORE_EXPORTS)
#define TIEZ_CORE_API __declspec(dllexport)
#elif defined(_WIN32)
#define TIEZ_CORE_API __declspec(dllimport)
#else
#define TIEZ_CORE_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef struct TiezCoreHandle TiezCoreHandle;

enum
{
    TIEZ_CORE_ABI_VERSION = 20,
};

typedef void (*TiezChangedCallback)(void* user_data, uint64_t generation);

TIEZ_CORE_API uint32_t tiez_core_abi_version(void);
TIEZ_CORE_API TiezCoreHandle* tiez_core_create(void);
TIEZ_CORE_API void tiez_core_destroy(TiezCoreHandle* handle);

// Returns a newly allocated UTF-8 JSON string. The caller must release it with
// tiez_core_string_free. A null query is treated as an empty query.
TIEZ_CORE_API char* tiez_core_get_snapshot_json(
    TiezCoreHandle* handle,
    const char* query_utf8);

// Returns full content metadata for one stable entry ID as newly allocated
// UTF-8 JSON. Sensitive or encrypted payloads are returned as unavailable.
TIEZ_CORE_API char* tiez_core_get_content_json(
    TiezCoreHandle* handle,
    int64_t entry_id);

// Returns cached OCR/QR analysis as JSON. The `analysis` property is null when
// the image has not yet been analyzed.
TIEZ_CORE_API char* tiez_core_get_image_analysis_json(
    TiezCoreHandle* handle,
    int64_t entry_id);

// Runs Windows OCR and QR decoding. Read-only or sensitive results remain
// memory-only and never write recognized plaintext to the database.
TIEZ_CORE_API char* tiez_core_analyze_image_json(
    TiezCoreHandle* handle,
    int64_t entry_id,
    bool force);

// Returns a validated URL or local-file launch plan as newly allocated JSON.
// Sensitive and unavailable entries are rejected. The native frontend owns
// the final OS launch and must release the result with tiez_core_string_free.
TIEZ_CORE_API char* tiez_core_prepare_open_content_json(
    TiezCoreHandle* handle,
    int64_t entry_id);

// Creates a consistent database + attachment backup. Backup operations use
// production data owned by this process and return allocated metadata JSON.
TIEZ_CORE_API char* tiez_core_create_backup_json(
    TiezCoreHandle* handle,
    const char* destination_utf8);

// Fully validates archive structure, declared sizes, and SHA-256 checksums.
TIEZ_CORE_API char* tiez_core_inspect_backup_json(
    TiezCoreHandle* handle,
    const char* path_utf8);

// Copies a validated archive into the managed pending-restore slot. Writable
// WinUI startup applies it before opening SQLite and keeps a rollback copy.
TIEZ_CORE_API char* tiez_core_schedule_restore_json(
    TiezCoreHandle* handle,
    const char* path_utf8);

// Supported actions: pin, delete, clear, paste/copy-plain, and paste/copy-rich.
TIEZ_CORE_API bool tiez_core_apply_action(
    TiezCoreHandle* handle,
    int64_t entry_id,
    const char* action_utf8);

// Applies an action and returns a newly allocated structured JSON result with
// requested/effective/replacement IDs, removal state, generation, and message.
TIEZ_CORE_API char* tiez_core_apply_action_json(
    TiezCoreHandle* handle,
    int64_t entry_id,
    const char* action_utf8);

// Pastes arbitrary UTF-8 text without creating a history row. The native host
// owns window hiding/focus restoration; the Rust core owns clipboard + Ctrl+V.
TIEZ_CORE_API bool tiez_core_paste_text(
    TiezCoreHandle* handle,
    const char* text_utf8);

// Returns the ordered image Emoji favorites from the existing SQLite setting
// and managed data directory as newly allocated UTF-8 JSON.
TIEZ_CORE_API char* tiez_core_get_emoji_favorites_json(
    TiezCoreHandle* handle);

// Validates and copies a local image into the managed Emoji favorites folder,
// updates the compatible setting, and returns the mutation as allocated JSON.
TIEZ_CORE_API char* tiez_core_import_emoji_favorite_json(
    TiezCoreHandle* handle,
    const char* source_path_utf8);

// Removes one favorite from the compatible setting. Only files inside the
// managed Emoji favorites directory may be deleted.
TIEZ_CORE_API char* tiez_core_remove_emoji_favorite_json(
    TiezCoreHandle* handle,
    const char* favorite_path_utf8);

// Pastes an image that is currently registered as an Emoji favorite. The
// native host owns hiding/focus restoration; Rust owns clipboard + Ctrl+V.
TIEZ_CORE_API bool tiez_core_paste_emoji_favorite(
    TiezCoreHandle* handle,
    const char* favorite_path_utf8);

// Returns saved and in-use tags with counts, colors, protected status, and
// adapter metadata as newly allocated UTF-8 JSON.
TIEZ_CORE_API char* tiez_core_get_tag_catalog_json(
    TiezCoreHandle* handle);

// Returns metadata-only entries for one exact tag. Sensitive previews remain
// redacted. Results are capped and include the uncapped total.
TIEZ_CORE_API char* tiez_core_get_tag_entries_json(
    TiezCoreHandle* handle,
    const char* tag_utf8);

// Creates one saved tag without manufacturing a clipboard-history entry.
TIEZ_CORE_API char* tiez_core_create_tag_json(
    TiezCoreHandle* handle,
    const char* name_utf8);

// Renames a non-protected tag across history through the shared secure tag
// mutation path, then merges the compatible saved-tag metadata.
TIEZ_CORE_API char* tiez_core_rename_tag_json(
    TiezCoreHandle* handle,
    const char* old_name_utf8,
    const char* new_name_utf8);

// Permanently deletes every history entry using a non-protected tag, then
// removes its saved metadata. Per-entry deletion keeps tombstone and attachment
// cleanup semantics.
TIEZ_CORE_API char* tiez_core_delete_tag_json(
    TiezCoreHandle* handle,
    const char* name_utf8);

// Sets a compatible #RRGGBB tag color. An empty color clears the custom value.
TIEZ_CORE_API char* tiez_core_set_tag_color_json(
    TiezCoreHandle* handle,
    const char* name_utf8,
    const char* color_utf8);

// Adds a manual UTF-8 text history entry to one non-protected tag without
// trimming its content. Protected tags must use the atomic item-tag update.
TIEZ_CORE_API char* tiez_core_create_tagged_text_json(
    TiezCoreHandle* handle,
    const char* tag_utf8,
    const char* content_utf8);

// Replaces an entry's tags from a UTF-8 JSON string array. Session-only
// entries receive a positive replacement ID when the tag update persists them.
TIEZ_CORE_API char* tiez_core_update_tags_json(
    TiezCoreHandle* handle,
    int64_t entry_id,
    const char* tags_json_utf8);

// Replaces the complete pinned entry order from a top-to-bottom JSON ID array.
TIEZ_CORE_API char* tiez_core_update_pinned_order_json(
    TiezCoreHandle* handle,
    const char* ordered_ids_json_utf8);

// Returns only the allowlisted, non-secret settings used by native surfaces.
TIEZ_CORE_API char* tiez_core_get_settings_json(TiezCoreHandle* handle);

// Updates one allowlisted setting after type/range validation.
TIEZ_CORE_API char* tiez_core_update_setting_json(
    TiezCoreHandle* handle,
    const char* key_utf8,
    const char* value_utf8);

// Returns and updates only the legacy app.search_hotkey value. The native
// host registers a candidate with Windows before persisting it.
TIEZ_CORE_API char* tiez_core_get_search_hotkey_json(
    TiezCoreHandle* handle);
TIEZ_CORE_API char* tiez_core_update_search_hotkey_json(
    TiezCoreHandle* handle,
    const char* value_utf8);

// Returns AI settings and profile summaries without API keys.
TIEZ_CORE_API char* tiez_core_get_ai_settings_json(TiezCoreHandle* handle);

// Transactionally updates AI profiles, assignments, and preferences. API keys
// are write-only and an omitted key preserves the existing encrypted value.
TIEZ_CORE_API char* tiez_core_update_ai_settings_json(
    TiezCoreHandle* handle,
    const char* request_json_utf8);

// Tests one saved profile without returning its key. This call performs
// blocking network I/O and must run off the native UI thread.
TIEZ_CORE_API char* tiez_core_probe_ai_profile_json(
    TiezCoreHandle* handle,
    const char* profile_id_utf8);

// Runs task, mouthpiece, or translate against one non-sensitive text-like
// history entry without modifying it. This call performs blocking network I/O.
TIEZ_CORE_API char* tiez_core_run_ai_action_json(
    TiezCoreHandle* handle,
    int64_t entry_id,
    const char* action_utf8);

// Returns WebDAV cloud-sync settings without returning stored passwords.
TIEZ_CORE_API char* tiez_core_get_cloud_sync_settings_json(
    TiezCoreHandle* handle);

// Transactionally updates WebDAV settings from a UTF-8 JSON object. A password
// can be replaced or explicitly cleared, but it is never echoed in the result.
TIEZ_CORE_API char* tiez_core_update_cloud_sync_settings_json(
    TiezCoreHandle* handle,
    const char* request_json_utf8);

// Performs a read-only PROPFIND against the configured endpoint. Redirects are
// not followed and no remote collection or clipboard payload is written.
TIEZ_CORE_API char* tiez_core_probe_cloud_sync_json(
    TiezCoreHandle* handle);

// Returns background runner state and transfer counts without credentials.
TIEZ_CORE_API char* tiez_core_get_cloud_sync_status_json(
    TiezCoreHandle* handle);

// Starts the runner, or reloads its automatic schedule after settings change.
TIEZ_CORE_API bool tiez_core_start_cloud_sync(TiezCoreHandle* handle);

// Requests an immediate pass and forces a full snapshot publication.
TIEZ_CORE_API bool tiez_core_request_cloud_sync(TiezCoreHandle* handle);

// Cancels and joins the runner before the native host or DLL is unloaded.
TIEZ_CORE_API void tiez_core_stop_cloud_sync(TiezCoreHandle* handle);

// Returns sanitized readiness for the relay/v1 WebDAV protocol. The stored
// shared key and WebDAV password never cross this boundary.
TIEZ_CORE_API char* tiez_core_get_relay_status_json(TiezCoreHandle* handle);

// Shared keys are strict 64-character lowercase hexadecimal strings and are
// persisted under the same OS-vault identity used by the Tauri frontend.
TIEZ_CORE_API char* tiez_core_set_relay_shared_key_json(
    TiezCoreHandle* handle,
    const char* shared_key_utf8);
TIEZ_CORE_API char* tiez_core_generate_relay_shared_key_json(
    TiezCoreHandle* handle);
TIEZ_CORE_API char* tiez_core_clear_relay_shared_key_json(
    TiezCoreHandle* handle);

// Returns and updates only the non-secret Tauri-compatible relay shortcut
// keys. The native host registers a candidate before persisting it.
TIEZ_CORE_API char* tiez_core_get_relay_hotkeys_json(
    TiezCoreHandle* handle);
TIEZ_CORE_API char* tiez_core_update_relay_hotkey_json(
    TiezCoreHandle* handle,
    const char* key_utf8,
    const char* value_utf8);

// Blocking relay calls. The native host must execute them away from the UI
// thread. Fetch copies exact UTF-8 text and records an at-most-once receipt.
TIEZ_CORE_API char* tiez_core_send_relay_clipboard_json(
    TiezCoreHandle* handle);
TIEZ_CORE_API char* tiez_core_fetch_relay_clipboard_json(
    TiezCoreHandle* handle);

// Returns compatible preferences, pairing URL/QR, service state, online
// devices, and capped chat history. Pairing secrets exist only while running.
TIEZ_CORE_API char* tiez_core_get_file_transfer_json(TiezCoreHandle* handle);

// Validates and persists a JSON settings patch. An explicit enabled toggle
// starts or stops the native server after the transaction succeeds.
TIEZ_CORE_API char* tiez_core_update_file_transfer_json(
    TiezCoreHandle* handle,
    const char* request_json_utf8);

// Starts or synchronously stops the authenticated LAN server.
TIEZ_CORE_API bool tiez_core_start_file_transfer(TiezCoreHandle* handle);
TIEZ_CORE_API void tiez_core_stop_file_transfer(TiezCoreHandle* handle);

// Sends text to paired devices or registers local files as authenticated,
// streaming downloads. JSON strings are allocated by Rust and caller-freed.
TIEZ_CORE_API char* tiez_core_send_transfer_text_json(
    TiezCoreHandle* handle,
    const char* text_utf8);
TIEZ_CORE_API char* tiez_core_share_transfer_files_json(
    TiezCoreHandle* handle,
    const char* paths_json_utf8);

// History-changed notifications may arrive on a background worker thread.
TIEZ_CORE_API void tiez_core_set_changed_callback(
    TiezCoreHandle* handle,
    TiezChangedCallback callback,
    void* user_data);

// Starts native clipboard capture. File/rich formats follow native settings;
// the current clipboard is primed, not ingested.
TIEZ_CORE_API bool tiez_core_start_capture(TiezCoreHandle* handle);

// Returns and clears the calling thread's last error. The caller must release
// the returned UTF-8 string with tiez_core_string_free.
TIEZ_CORE_API char* tiez_core_take_last_error(void);
TIEZ_CORE_API void tiez_core_string_free(char* value);

#ifdef __cplusplus
}
#endif
