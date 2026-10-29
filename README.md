# UiProbeKit

## TL;DR

UiProbeKit is one small DLL that lets a program look at another program's window the way Windows itself sees it: list the top-level windows, read a window's control tree (names, control types, automation ids, rectangles, enabled/offscreen), find a control in that tree, and act on it — invoke a button, toggle a checkbox, set a text value, give it focus. It is the probing half of desktop test automation, behind a small JSON C ABI, with no dependencies and no test framework dragged into your app.

```c
#include "uiprobekit.h"
#include <stdio.h>

int main(void) {
    /* Every top-level window, as JSON. */
    char *wins = up_list_windows();
    printf("%s\n", wins);
    up_free_string(wins);

    /* The control tree of one window (hwnd from the list). */
    char *tree = up_inspect("{\"hwnd\":123456,\"max_depth\":4}");
    printf("%s\n", tree);   /* {..., "truncated":false} */
    up_free_string(tree);

    /* Where is the Save button? Paths are child indexes. */
    char *hits = up_find("{\"hwnd\":123456,\"name\":\"Save\"}");
    printf("%s\n", hits);   /* [{"path":[1,0]}] */
    up_free_string(hits);

    /* Press it. */
    int rc = up_action("{\"hwnd\":123456,\"path\":[1,0],\"action\":\"invoke\"}");
    if (rc != UP_OK) printf("failed: %s\n", up_last_error());
    return 0;
}
```

## Key features

- **Window list in one call.** `up_list_windows` enumerates the top-level windows (EnumWindows) with title, class name, owning pid, screen rectangle and a visibility flag for each — the starting point for "which window is my app?".
- **Control trees with honest caps.** `up_inspect` walks the UI Automation **raw view** from the window's root element and returns nested JSON: name, numeric control type id, automation id, class name, rectangle, enabled, offscreen. `max_depth` (default 8) and `max_nodes` (default 2000) bound the walk, and the root object carries `"truncated":true` when a cap cut something — a probe that silently returns half a tree is worse than one that says so.
- **Find by what you know.** `up_find` matches on exact name, automation id and/or control type id (ANDed) and returns root-relative paths in pre-order. No matches is an empty array, not an error. An unfiltered find is refused — it would match every element.
- **Actions through the real patterns.** `up_action` resolves a path and performs `invoke`, `toggle` or `set_value` through the matching UI Automation pattern (Invoke / Toggle / Value), or `focus` through the element's own SetFocus. An element that does not support the pattern returns UP_ERR_NOT_SUPPORTED; a path that does not resolve returns UP_ERR_NOT_FOUND.
- **Stable addressing.** Paths index the same raw view `up_inspect` prints, so what you read is what you address. Re-inspect after the UI changes shape — a path is only as stable as the tree it came from.
- **No fake success.** On non-Windows platforms every probe returns UP_ERR_NOT_SUPPORTED. A failed property read on a live element degrades that one field to empty/zero instead of aborting the whole walk — live desktops have elements being torn down mid-walk, and a probe that dies on the first stale element is useless there.
- **Zero dependencies.** Hand-rolled JSON (duplicate keys in one object are a parse error, by design), hand-declared Win32 bindings, COM resolved dynamically at runtime (ole32/oleaut32), small C ABI.

## The API in one screen

| Call | What it does |
|---|---|
| `up_list_windows()` | JSON array of top-level windows: hwnd, title, class_name, pid, rect, visible |
| `up_inspect(spec_json)` | Control-tree JSON for `{"hwnd", "max_depth"?, "max_nodes"?}`; root object carries `"truncated"` |
| `up_find(spec_json)` | `[{"path":[...]}]` for `{"hwnd", "name"?, "automation_id"?, "control_type"?}` (≥1 criterion) |
| `up_action(spec_json)` | `{"hwnd", "path", "action", "value"?}` → UP_OK or a UP_* code |
| `up_version` / `up_last_error` / `up_last_error_code` / `up_free_string` | Version, thread-local error, string ownership |

All `char*` results are caller-owned; free them with `up_free_string()`. Failures return NULL (or the error code, for `up_action`) and set the thread-local last error.

## Build

```sh
cargo build --release   # ui_probe_kit.dll / .so / .a + include/uiprobekit.h
cargo test              # full test suite
```

Rust 1.80+, edition 2021, no dependencies.

## Limitations, honestly

- **Windows runtime is Unverified.** The pure logic (spec parsing, tree serialization + caps, find, path validation, the off-Windows error contract) is covered by the test suite on any platform. The Windows backend compiles and is clippy-clean for `x86_64-pc-windows-msvc`, and every IID and vtable slot was verified against the `windows` crate 0.62.2 generated bindings — but the local gate does not link, and no tree has yet been read from a real desktop. That field test, plus CI's real Windows link, is the follow-up.
- **No screenshots.** Capturing pixels is deliberately out of scope — CaptureKit covers captures. UiProbeKit reports structure and state, not images.
- **Raw view only in v0.1.** There is no control/content view selection and no condition-based search yet; find walks the raw tree within internal bounds (depth 64, 100 000 nodes).
- **Four actions only.** Invoke, Toggle, Value and Focus. More patterns (ExpandCollapse, SelectionItem, Scroll, Text, …) are roadmap items, not silent omissions.
- **Property reads degrade per field.** If one element's name cannot be read (it vanished mid-walk), that field comes back empty and the walk continues. The tree shape — and therefore every path — still reflects the walk as it happened.
