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
    TIEZ_CORE_ABI_VERSION = 3,
};

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

// Supported prototype actions: pin, delete, paste-plain, and paste-rich.
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

// Returns and clears the calling thread's last error. The caller must release
// the returned UTF-8 string with tiez_core_string_free.
TIEZ_CORE_API char* tiez_core_take_last_error(void);
TIEZ_CORE_API void tiez_core_string_free(char* value);

#ifdef __cplusplus
}
#endif
