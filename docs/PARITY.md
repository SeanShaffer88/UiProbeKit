# UiProbeKit — Parity (spec vs implementation)

Evidence: the Rust test suite in `src/lib.rs`, `src/ffi.rs`
and `src/json.rs` (19 tests, all passing at the v0.1.0
gate) plus the header contract in `include/uiprobekit.h`.
"Verified" below means a named test exercises the
behaviour. The Windows backend cannot run on the
development VM; its rows say exactly how far the evidence
goes, and its runtime behaviour is **Unverified** — a field
test on a real desktop is pending.

| Spec feature | Status | Evidence |
|---|---|---|
| Inspect spec: hwnd required + positive, caps default 8 / 2000, overrides accepted, hard bounds (depth ≤ 256, nodes 1..=1 000 000) | Verified | `inspect_spec_defaults_and_overrides`, `inspect_spec_rejects_bad_input` (zero/negative/string hwnd, out-of-range caps, unknown key, duplicate key, non-object, non-JSON) |
| Find spec: ≥1 criterion required, criteria typed (name/automation_id strings, control_type a non-negative integer) | Verified | `find_spec_validation` |
| Action spec: path required, action one of invoke/toggle/set_value/focus, value required for set_value | Verified | `action_spec_validation` |
| Path parsing/validation: array of non-negative integer indexes, empty path = root, length cap 512, per-index cap | Verified | `path_parsing_and_validation` |
| Tree JSON serialization: node shape (name, control_type, automation_id, class_name, rect{x,y,w,h}, enabled, offscreen, children) | Verified | `serialize_tree_shape`, `window_info_shape` (rect corner→origin/size conversion + JSON round-trip) |
| Caps in serialization: depth cap cuts children and marks truncated; node cap admits in pre-order and marks truncated; exactly-at-cap is NOT truncated | Verified | `serialize_tree_depth_cap_marks_truncated`, `serialize_tree_node_cap_marks_truncated`, `walk_state_admission` |
| Find in a built tree: by name / automation id / control type, criteria ANDed, root match is path [], no match is an empty list | Verified | `find_paths_in_tree` |
| Window list JSON shape (hwnd, title, class_name, pid, rect, visible) | Verified | `window_info_shape` |
| Argument errors win over the platform check on every platform; valid specs off Windows are a clean NOT_SUPPORTED (list / inspect / find / action) | Verified | lib `platform_contract_off_windows`, FFI `ffi_platform_contract`, `ffi_bad_specs_are_argument_errors_everywhere` |
| FFI contract: version string, NULL/bad spec → NULL + INVALID_ARG, up_free_string(NULL) is a no-op | Verified | `ffi_version_and_free_null`, `ffi_bad_specs_are_argument_errors_everywhere` |
| JSON parser edge cases (surrogates, depth cap, trailing garbage, bad numbers, duplicate keys) | Verified | json.rs tests: `parses_scalars_and_surrogates`, `rejects_garbage`, `depth_cap`, `parses_config_shaped_json` |
| `up_list_windows` via EnumWindows + GetWindowTextW/GetClassNameW/GetWindowThreadProcessId/GetWindowRect/IsWindowVisible | Implemented, not run — **Unverified** | `src/platform.rs` win module: hand-declared user32 bindings with #[link] on the block. Compiles + clippy-clean for `x86_64-pc-windows-msvc` at the gate; the real link is CI's, a real window list is a field test's |
| `up_inspect` tree walk via IUIAutomation::ElementFromHandle + RawViewWalker (GetFirstChildElement / GetNextSiblingElement), caps enforced during the walk via the shared WalkState | Implemented, not run — **Unverified** | `src/platform.rs` uia module. Every IID and vtable slot verified against the `windows` crate 0.62.2 generated bindings (Accessibility/mod.rs); the slot table is recorded in the module comment and AGENTS.md |
| `up_find` on Windows: same capped walk (internal bounds depth 64 / 100 000 nodes) feeding the pure `find_paths` | Implemented, not run — **Unverified** | Same uia walk as inspect; the matching/path half is the Verified `find_paths_in_tree` |
| `up_action`: path resolution via the raw walker; invoke/toggle/set_value via GetCurrentPattern + QueryInterface to the pattern interface (slot 3); focus via SetFocus; BSTR for set_value allocated with SysAllocString and freed after the call | Implemented, not run — **Unverified** | Same module; pattern IIDs (Invoke FB377FBE…, Toggle 94CF8058…, Value A94CD8B1…) and pattern ids (10000 / 10015 / 10002) verified against the same bindings |
| Header documents every export | Done | `include/uiprobekit.h` (spec shapes, caps and their defaults, truncated semantics, error contract, threading, platform contract) |

## Known gaps (also in README "Limitations")

- Windows runtime behaviour is Unverified until a field test on a real desktop reads a real tree and performs a real action; CI's Windows build is the link gate the local gate cannot provide.
- Screenshot capture is out of scope by design — CaptureKit covers captures (ROADMAP item 1).
- Raw view only; no control/content view selection and no condition-based find yet (roadmap items 3–4).
- Four actions only (invoke, toggle, set_value, focus); more patterns are roadmap item 2.
