# UiProbeKit — Quality Scorecard

Scored against the v0.1.0 gate run on the development VM:
`cargo fmt --check`, clippy (host + `x86_64-pc-windows-msvc`,
`-D warnings`), windows-target check, and the full test
suite — 19/19 passing. Scores are only claimed where that
evidence exists; Windows runtime behaviour is not claimed
from Linux evidence.

The UI Automation COM backend is **Unverified**: its IIDs,
pattern ids and vtable layouts were verified against the
`windows` crate 0.62.2 generated bindings, and it compiles
+ clippy-checks for `x86_64-pc-windows-msvc` at the gate —
but the gate does not LINK, and no control tree has yet
been read from a real desktop. The UIA dimension is
therefore held at 2/5 and is NOT counted into the total
below until a field test (and CI's real Windows link)
provide runtime evidence.

| Dimension | Score /5 | Evidence |
|---|---|---|
| Spec parsing & validation | 5 | All three specs tested: defaults, overrides, hard bounds, unknown keys, duplicate keys (parser-level), wrong types, missing hwnd/path/action, find without a criterion, set_value without a value — every rejection is INVALID_ARG with a specific message |
| Tree serialization & caps | 5 | Node shape asserted field-by-field; depth cap and node cap each cut + flag; exactly-at-cap is not flagged; WalkState admission tested directly (depth refusal, node refusal, flag stickiness) |
| Find & path logic | 5 | Paths asserted exactly (nested match [1,0], sibling [1,1], root match []), criteria ANDing, no-match = empty list; path parser bounds (negative, fractional, string, over-long, over-large index) all tested |
| Error honesty | 5 | Off Windows: NOT_SUPPORTED at both lib and FFI layers for all four probes, tested; argument errors demonstrably win over the platform check; pattern absence / unresolvable path / missing element are three distinct codes in the Windows backend by construction |
| API & documentation | 4 | Header documents every export with spec shapes, caps, defaults and the truncated contract; README has a runnable C example. No compiled C consumer test in the suite (FFI is exercised from Rust) — one point held back |
| Code health | 5 | Zero dependencies, fmt + clippy clean on both targets at the gate, no unwinding across the FFI boundary, no shared mutable state (thread-local last error only) |
| Platform coverage | 3 | Pure logic fully tested on Linux; the Windows MSVC build is check/clippy-verified here, but the real link and any real-desktop tree are CI's / a field test's to prove — honestly mid-scored |
| UIA COM backend (provisional — not in total) | 2 | Walk/action glue verified only against the `windows` crate's generated bindings and the windows-target check/clippy (no link, no run) — held low until a field test reads a real tree |

**Total: 89/100** (counted dimensions only: 31/35)

## Waivers / open items

- The local gate does not link; a windows-gnu link was attempted on the VM but the mingw toolchain was not installed and apt was locked by another process at the time. CI's `cargo build --release` on windows-latest is the link gate, and a field test on a real desktop is the follow-up that can raise the platform and UIA scores.
- No dimension is scored on Windows runtime evidence; there is none yet.
