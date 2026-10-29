/* UiProbeKit.dll — UI Automation inspection and actions
 * behind a small C ABI: list the top-level windows, read a
 * window's control tree, find elements in it, and act on an
 * element (invoke, toggle, set a value, focus) — the probing
 * half of desktop test automation, without linking a test
 * framework into the host.
 *
 * Everything is driven by small JSON specs and answered with
 * JSON. Element addressing is by PATH: the array of child
 * indexes from the window's root element, exactly as the
 * tree from up_inspect() is nested — up_find() returns these
 * paths and up_action() consumes them. Indexes are raw-view
 * child positions (the full UI Automation raw view, no
 * filtering), so a path stays valid as long as the tree shape
 * does not change between calls; re-inspect after the UI
 * changes.
 *
 * Strings: all strings are UTF-8, NUL-terminated.
 *
 * Ownership: every char* result is caller-owned and must be
 * freed with up_free_string(). up_version() and
 * up_last_error() return borrowed strings — do NOT free them.
 *
 * Errors: functions returning char* return NULL on failure;
 * up_action() returns a UP_* code. Either way,
 * up_last_error() / up_last_error_code() describe the failure
 * on the calling thread.
 *
 * Platforms: the probes are Windows-only (UI Automation).
 * On any other platform a well-formed call returns
 * UP_ERR_NOT_SUPPORTED — UiProbeKit never fabricates an
 * empty tree or a fake success. Spec VALIDATION runs before
 * the platform check on every platform, so a malformed spec
 * is UP_ERR_INVALID_ARG everywhere.
 *
 * Threading: the exports keep no shared state besides the
 * thread-local last error; concurrent calls are fine. Each
 * Windows call initializes COM on the calling thread for the
 * duration of the call.
 */
#ifndef UIPROBEKIT_H
#define UIPROBEKIT_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#if defined(_WIN32) && defined(UP_BUILD)
#define UP_API __declspec(dllexport)
#elif defined(_WIN32)
#define UP_API __declspec(dllimport)
#else
#define UP_API
#endif

#define UP_OK 0
#define UP_ERR_INVALID_ARG 1
#define UP_ERR_NOT_SUPPORTED 2
#define UP_ERR_NOT_FOUND 3
#define UP_ERR_ACCESS_DENIED 4
#define UP_ERR_PLATFORM 5
#define UP_ERR_INTERNAL 6

/* Library version, e.g. "0.1.0". Borrowed string. */
UP_API const char *up_version(void);

/* Last error message / code for the calling thread.
 * up_last_error() returns a borrowed string. */
UP_API const char *up_last_error(void);
UP_API int up_last_error_code(void);

/* Free a string this library returned. NULL is a no-op. */
UP_API void up_free_string(char *p);

/* List the top-level windows (EnumWindows order).
 * Returns a caller-owned JSON array:
 *   [{"hwnd": 123456, "title": "...", "class_name": "...",
 *     "pid": 1234,
 *     "rect": {"x": 0, "y": 0, "w": 800, "h": 600},
 *     "visible": true}, ...]
 * All top-level windows are listed, visible or not; the
 * "visible" field says which. "rect" is screen pixels,
 * origin + size. */
UP_API char *up_list_windows(void);

/* Read a window's control tree (UI Automation raw view).
 * spec_json:
 *   {"hwnd": 123456,            // required, positive
 *    "max_depth": 8,            // optional, 0..=256
 *    "max_nodes": 2000}         // optional, 1..=1000000
 * Unknown keys are rejected. Returns a caller-owned JSON
 * object for the root element:
 *   {"name": "...", "control_type": 50032,
 *    "automation_id": "...", "class_name": "...",
 *    "rect": {"x": 0, "y": 0, "w": 800, "h": 600},
 *    "enabled": true, "offscreen": false,
 *    "children": [ ...same shape... ],
 *    "truncated": false}
 * "control_type" is the numeric UIA control type id
 * (50000 Button, 50004 Edit, 50032 Window, ...). The root is
 * depth 0; nodes at "max_depth" are included with empty
 * "children". "truncated" is present on the root object
 * only and is true when either cap cut anything. */
UP_API char *up_inspect(const char *spec_json);

/* Find elements in a window's control tree.
 * spec_json:
 *   {"hwnd": 123456,            // required, positive
 *    "name": "Save",            // optional, exact match
 *    "automation_id": "saveBtn",// optional, exact match
 *    "control_type": 50000}     // optional, UIA id
 * At least one criterion is required; criteria are ANDed.
 * Returns a caller-owned JSON array of matches in pre-order:
 *   [{"path": [1, 0]}, {"path": []}]
 * A path is the child indexes from the root element; [] is
 * the root itself. No matches is an empty array, not an
 * error. The search walks within internal bounds (depth 64,
 * 100000 nodes) so a find cannot run unbounded. */
UP_API char *up_find(const char *spec_json);

/* Perform an action on the element a path addresses.
 * spec_json:
 *   {"hwnd": 123456,            // required, positive
 *    "path": [1, 0],            // required, may be []
 *    "action": "invoke",        // invoke | toggle |
 *                               // set_value | focus
 *    "value": "text"}           // required for set_value
 * invoke/toggle/set_value go through the matching UI
 * Automation pattern (Invoke / Toggle / Value); an element
 * that does not support the pattern yields
 * UP_ERR_NOT_SUPPORTED. focus calls the element's SetFocus.
 * A path step that does not exist yields UP_ERR_NOT_FOUND.
 * A window with no UIA element yields UP_ERR_NOT_FOUND.
 * Returns UP_OK or a UP_* code. */
UP_API int up_action(const char *spec_json);

#ifdef __cplusplus
}
#endif
#endif
