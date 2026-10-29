# UiProbeKit Roadmap

## v0.1.0 (current)

Window/control probing behind one C ABI. `up_list_windows` enumerates the top-level windows via EnumWindows (title, class name, pid, rect, visibility). `up_inspect` builds a window's UI Automation **raw-view** control tree as nested JSON (name, numeric control type id, automation id, class name, rect, enabled, offscreen) with spec caps (`max_depth` default 8, `max_nodes` default 2000) enforced in the walk AND in the serializer, and a `"truncated"` flag on the root when a cap cut anything. `up_find` matches exact name / automation id / control type id (ANDed, at least one required) and returns root-relative child-index paths. `up_action` resolves a path and performs invoke / toggle / set_value through the matching UIA pattern, or focus through SetFocus. COM entry points (ole32) and BSTR helpers (oleaut32) are resolved dynamically at runtime — no dependencies, no new link requirements; every IID and vtable slot was verified against the `windows` crate 0.62.2 generated bindings. **Runtime is Unverified**: the Windows path is windows-target checked/clippy-clean at the gate, but the gate does not link and no tree has yet been read from a real desktop — that field test is the follow-up, along with CI's real Windows link.

## Next, in order

1. **Screenshot capture — covered elsewhere, not planned here.** Capturing pixels belongs to CaptureKit (the AI-assist suite tool that already turns capture lists into verified README screenshots). UiProbeKit deliberately stays structural: tree + state, no images. This item stays at the top of the list so the omission is a decision, not an oversight.
2. **More action patterns**: ExpandCollapse (expand/collapse), SelectionItem (select), ScrollItem (scroll into view), and Window (close/maximize/minimize) — each as a new `"action"` value through the same path addressing, added one pattern at a time with its IID/slots verified against the bindings the same way as v0.1's.
3. **Condition-based find**: use IUIAutomation property conditions (FindFirst/FindAll) as an alternative search that does not materialize the raw tree, for very large trees where the v0.1 walk bounds bite; results still reported as paths.
4. **Control/content view selection**: an inspect `"view"` option (`"raw"` default, `"control"`, `"content"`) selecting the corresponding tree walker.
5. **Event subscription**: focus-changed and structure-changed events delivered through a host callback, so a test harness can wait for the UI to settle instead of polling inspect.
6. **Cached property fetch**: a CacheRequest-based bulk read for big trees, reducing per-property COM round-trips during inspect.

## Explicitly not planned

- **Screenshots or image comparison**: CaptureKit's job (roadmap item 1).
- **Synthetic input (SendInput mouse/keyboard)**: UiProbeKit acts through accessibility patterns, which target the control that means it. Pixel-coordinate clicking is a different tool with different failure modes.
- **Cross-platform UI probing**: the probes are UI Automation, a Windows facility. Other platforms get the documented NOT_SUPPORTED, never a stubbed look-alike.
