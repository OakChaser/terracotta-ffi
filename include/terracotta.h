/*
 * terracotta.h — C ABI for conic-terracotta
 *
 * conic-terracotta is the optional multiplayer backend for Conic Launcher.
 * It provides a Terracotta-compatible runtime (room codes, EasyTier overlay,
 * scaffolding protocol) behind a stable, opaque-handle C interface.
 *
 * Conventions:
 *   - Every function is safe to call from any thread, but only ONE logical
 *     caller is expected for command + poll + state (a launcher UI thread).
 *   - Command functions (create_room / join_room / set_waiting) validate and
 *     enqueue synchronously; they return immediately. Actual work happens on
 *     the library's own background runtime thread.
 *   - terracotta_poll_event() is non-blocking. Call it periodically (e.g.
 *     every 16ms) to drain incremental updates. Recover a full snapshot with
 *     terracotta_get_state().
 *   - All strings returned by the library are owned by the library and must
 *     be released with terracotta_free_string(). The terracotta_string.data
 *     is NUL-terminated; terracotta_string.len is the byte length excluding
 *     the NUL.
 *
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

#ifndef CONIC_TERRACOTTA_TERRACOTTA_H
#define CONIC_TERRACOTTA_TERRACOTTA_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32) && defined(CONIC_TERRACOTTA_BUILDING_DLL)
#  define TERRA_API __declspec(dllexport)
#elif defined(_WIN32)
#  define TERRA_API __declspec(dllimport)
#elif defined(__GNUC__) || defined(__clang__)
#  define TERRA_API __attribute__((visibility("default")))
#else
#  define TERRA_API
#endif

/* ------------------------------------------------------------------ */
/* Handle                                                              */
/* ------------------------------------------------------------------ */

typedef void* terracotta_handle;
#define TERRA_INVALID_HANDLE ((terracotta_handle)0)

/* ------------------------------------------------------------------ */
/* Result codes                                                        */
/* ------------------------------------------------------------------ */

typedef enum terracotta_result {
    TERRA_OK                  = 0,

    TERRA_ERR_INVALID_HANDLE  = -1,  /* handle is NULL or already destroyed  */
    TERRA_ERR_INVALID_ARGUMENT= -2,  /* NULL/empty string, bad length, etc.  */
    TERRA_ERR_BAD_STATE       = -3,  /* command not allowed in current state */
    TERRA_ERR_INVALID_ROOM_CODE = -4, /* malformed U/XXXX-... code            */
    TERRA_ERR_ALREADY_ACTIVE  = -5,  /* a room session is already running    */
    TERRA_ERR_INTERNAL        = -6,  /* internal error (panic swallowed)     */
    TERRA_ERR_OUT_OF_MEMORY   = -7,
    TERRA_ERR_NO_EVENT        = -8,  /* poll_event: queue is empty           */
    TERRA_ERR_SHUTTING_DOWN   = -9,  /* destroy in progress                 */
} terracotta_result;

/* ------------------------------------------------------------------ */
/* Strings owned by the library                                        */
/* ------------------------------------------------------------------ */

typedef struct terracotta_string {
    const char* data;  /* UTF-8, NUL-terminated */
    uint32_t    len;   /* byte length excluding NUL */
} terracotta_string;

/* ------------------------------------------------------------------ */
/* Session states (mirrors Terracotta's AppState)                      */
/* ------------------------------------------------------------------ */

typedef enum terracotta_state_id {
    TERRA_STATE_WAITING          = 0,
    TERRA_STATE_HOST_SCANNING    = 1,
    TERRA_STATE_HOST_STARTING    = 2,
    TERRA_STATE_HOST_OK          = 3,
    TERRA_STATE_GUEST_CONNECTING = 4,
    TERRA_STATE_GUEST_STARTING   = 5,
    TERRA_STATE_GUEST_OK         = 6,
    TERRA_STATE_EXCEPTION        = 7,
} terracotta_state_id;

/* ------------------------------------------------------------------ */
/* Events (incremental updates)                                        */
/* ------------------------------------------------------------------ */

typedef enum terracotta_event_type {
    TERRA_EVENT_STATE_CHANGED         = 1, /* payload: {"state":id,"version":n}          */
    TERRA_EVENT_PLAYER_JOINED         = 2, /* payload: {"profile":{"machine_id",         */
                                           /*          "name","vendor","kind":           */
                                           /*          "HOST"|"LOCAL"|"GUEST"}}          */
    TERRA_EVENT_PLAYER_LEFT           = 3, /* payload: {"machine_id"}                   */
    TERRA_EVENT_CONNECTION_DIFFICULTY = 4, /* payload: {"difficulty":0..4}              */
    TERRA_EVENT_HOST_READY            = 5, /* payload: {"room":"U/...","port":N}        */
    TERRA_EVENT_GUEST_READY           = 6, /* payload: {"url":"127.0.0.1:port"}         */
    TERRA_EVENT_ERROR                 = 7, /* payload: {"code":N,"message":"..."}       */
} terracotta_event_type;

typedef struct terracotta_event {
    uint64_t              sequence;   /* global monotonic sequence            */
    terracotta_event_type type;
    terracotta_string     payload;    /* JSON, owned by the library           */
} terracotta_event;

/* ------------------------------------------------------------------ */
/* Full state snapshot                                                 */
/* ------------------------------------------------------------------ */

typedef struct terracotta_state {
    uint64_t          version;    /* monotonic; increases on every change   */
    terracotta_state_id state;    /* current session state                  */
    terracotta_string room_code;  /* "U/XXXX-XXXX-XXXX-XXXX" or empty       */
    terracotta_string detail;     /* JSON: state-specific extras            */
} terracotta_state;

/*
 * detail content by state:
 *   TERRA_STATE_HOST_OK:    {"port": N, "profiles":[{machine_id,name,vendor,kind}]}
 *   TERRA_STATE_GUEST_OK:   {"url":"127.0.0.1:port","profiles":[...]}
 *   TERRA_STATE_EXCEPTION:  {"error": {"code": N, "message": "..."}}
 *   others:                 {}
 *   profiles[].kind:        "HOST" | "LOCAL" | "GUEST"
 */

/* ------------------------------------------------------------------ */
/* Configuration                                                       */
/* ------------------------------------------------------------------ */

typedef struct terracotta_config {
    const terracotta_string* public_nodes;      /* nullable array of extra nodes  */
    uint32_t                 public_nodes_count;
    terracotta_string        data_dir;          /* nullable; persistent files     */
                                                /* (machine-id). Default: temp    */
    terracotta_string        motd;              /* nullable; reserved MOTD used to */
                                                /* identify Terracotta virtual    */
                                                /* servers in LAN broadcasts      */
} terracotta_config;

/* ------------------------------------------------------------------ */
/* Lifecycle                                                           */
/* ------------------------------------------------------------------ */

/*
 * Creates a context: spawns the internal tokio runtime thread, command queue
 * and event queue. Returns TERRA_INVALID_HANDLE on failure.
 */
TERRA_API terracotta_handle terracotta_create(void);

/*
 * Destroys a context. Resets any active session first (killing the EasyTier
 * subprocess and stopping the internal fake server), stops the internal
 * scaffolding service, then gracefully stops the runtime thread and releases
 * all resources. Safe to call once; afterwards the handle is invalid.
 */
TERRA_API void terracotta_destroy(terracotta_handle handle);

/*
 * Applies configuration. Must be called before any command; fails with
 * TERRA_ERR_BAD_STATE if a room session is already active.
 */
TERRA_API terracotta_result terracotta_configure(
    terracotta_handle        handle,
    const terracotta_config* config);

/* ------------------------------------------------------------------ */
/* Commands (validated synchronously, executed asynchronously)         */
/* ------------------------------------------------------------------ */

/*
 * Hosts a room. room_code == NULL generates a new room; otherwise it must be
 * a valid "U/XXXX-XXXX-XXXX-XXXX" code and the room is created on that
 * network. player_name == NULL uses the default anonymous name.
 * Fails with TERRA_ERR_ALREADY_ACTIVE if a session is running.
 */
TERRA_API terracotta_result terracotta_create_room(
    terracotta_handle handle,
    const char*       player_name,  /* nullable */
    const char*       room_code);   /* nullable */

/*
 * Joins an existing room. room_code is required and validated.
 */
TERRA_API terracotta_result terracotta_join_room(
    terracotta_handle handle,
    const char*       room_code,
    const char*       player_name); /* nullable */

/*
 * Aborts any active session and returns to TERRA_STATE_WAITING.
 * No-op if already waiting.
 */
TERRA_API terracotta_result terracotta_set_waiting(terracotta_handle handle);

/* ------------------------------------------------------------------ */
/* Queries                                                             */
/* ------------------------------------------------------------------ */

/*
 * Returns a full state snapshot. Caller must release with
 * terracotta_free_state(). Returns TERRA_ERR_NO_EVENT-style errors on bad
 * handle / out-of-memory.
 */
TERRA_API terracotta_result terracotta_get_state(
    terracotta_handle   handle,
    terracotta_state*   out);

/*
 * Pops the next pending event, if any. Non-blocking.
 * Returns TERRA_OK and fills *out on success (release with
 * terracotta_free_event()), or TERRA_ERR_NO_EVENT when the queue is empty.
 */
TERRA_API terracotta_result terracotta_poll_event(
    terracotta_handle   handle,
    terracotta_event*   out);

/*
 * Verifies a room code without creating a context.
 * Returns 3 for a valid scaffolding room code, -1 for invalid.
 * (0 is reserved for future room kinds.)
 */
TERRA_API int terracotta_verify_room_code(const char* room_code);

/* Returns the library version string (e.g. "0.1.0"). Never NULL. */
TERRA_API const char* terracotta_version(void);

/* ------------------------------------------------------------------ */
/* Memory                                                              */
/* ------------------------------------------------------------------ */

TERRA_API void terracotta_free_string(terracotta_string* value);
TERRA_API void terracotta_free_state(terracotta_state*   value);
TERRA_API void terracotta_free_event(terracotta_event*   value);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* CONIC_TERRACOTTA_TERRACOTTA_H */
