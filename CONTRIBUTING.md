# Contributing to mcs51-mcp

Thanks for considering a contribution. This is a small, focused project — an MCP server
wrapping the 8051 development loop — and it is easier to contribute to than most embedded
tooling, because **almost all of it can be worked on without a microcontroller on your desk.**

By contributing you agree that your work is licensed under the [MIT License](LICENSE), the
same terms the project is released under.

---

## Contents

- [Where help is most wanted](#where-help-is-most-wanted)
- [Getting set up](#getting-set-up)
- [The checks that must pass](#the-checks-that-must-pass)
- [Invariants a change must preserve](#invariants-a-change-must-preserve)
- [Adding a tool](#adding-a-tool)
- [Tests](#tests)
- [Documentation](#documentation)
- [Electrical claims](#electrical-claims)
- [Commits and pull requests](#commits-and-pull-requests)
- [Reporting a bug](#reporting-a-bug)
- [Reporting a security issue](#reporting-a-security-issue)
- [Code of conduct](#code-of-conduct)

---

## Where help is most wanted

**Hardware validation — the single most valuable contribution.** No 8051 was attached during
development. `cargo test` and `scripts/smoke.py` cover the envelope contract, the path sandbox,
the subprocess runner and every safety rule, but the following have never met silicon:

- an actual `stcgal` flash to a physical STC89C52RC,
- a real `PING` → `PONG` round trip over a physical UART,
- `serial_write` / `serial_expect` / `serial_close` driven through the binary rather than
  against the fake port used by the tests.

If you own the hardware, flashing [`firmware/firmware.c`](firmware/firmware.c) and exercising the
serial round trip — **and reporting the result either way** — closes the project's largest gap.
See [Reporting a bug](#reporting-a-bug) for what to include; a report that everything worked is
just as useful as one that did not.

**Other welcome contributions:**

| Area | Notes |
|---|---|
| Platform support | Linux and Intel macOS are untested, not deliberately excluded. Serial device naming and `doctor`'s tool discovery are the likely friction points. |
| AT89S52 flashing | `flash(chip: "at89s")` is a documented stub. A workable macOS SPI-ISP path would remove a real limitation. |
| Additional MCUs | The design generalises, but `src/hw/` currently encodes one part. Discuss the shape in an issue before writing it. |
| Firmware protocol | Extensions to the line protocol in [`firmware/PROTOCOL.md`](firmware/PROTOCOL.md), keeping the RXD/TXD guard intact. |
| Documentation | Corrections, clarifications, and worked examples. Small doc PRs are welcome without a prior issue. |

For anything that changes the tool surface, the response envelope, or the `ErrorCode` set, please
**open an issue first.** Those are contracts a model caller depends on, and a design conversation
is cheaper than a rewritten PR.

---

## Getting set up

```bash
git clone https://github.com/darthvader58/mcp8051.git
cd mcp8051
cargo build --release
cargo test
```

That is enough for most work. Rust 1.88 or later is required (`rmcp` 3.x needs it).

You only need SDCC and stcgal if you are working on `compile`, `flash`, or `doctor` — see
[Requirements](README.md#requirements) in the README for installation. `doctor` reports missing
tools cleanly rather than failing, so the server runs without them.

You do not need any hardware to run the full test suite.

---

## The checks that must pass

Run all five before opening a pull request. CI aside, these are the project's definition of
"green":

```bash
cargo build --release
cargo clippy --all-targets -- -D warnings   # warnings are errors
cargo fmt --check
cargo test                                  # 59 unit and integration tests
python3 scripts/smoke.py                    # 43-assertion end-to-end stdio harness
```

`scripts/smoke.py` drives the **release** binary over stdio, so build before running it. It
requires no hardware. If you add or remove assertions or tools, update the counts quoted in the
README and in this file, and the `MCP tools` badge at the top of the README.

---

## Invariants a change must preserve

These are the things that are easy to break without noticing, because nothing in a diff points
at them.

**stdout belongs to the MCP protocol.** The server speaks JSON-RPC over stdio. A stray
`println!`, `dbg!` or `print!` anywhere in the crate corrupts the protocol stream and the client
sees a malformed frame rather than a useful error. All diagnostics go to stderr via `tracing`;
use `tracing::{info, debug, warn, error}` and nothing else.

**Every tool returns the shared envelope.** All twelve tools emit the same structure — twice, as
pretty-printed JSON in `content` and as `structuredContent` — and share a single declared
`outputSchema`. A new field, or a tool that returns a bespoke shape, breaks a caller that was
written against the common contract. Tool-specific data belongs under `data`.

**`ErrorCode` is a closed enum, and its wire spellings are public API.** `src/errors.rs` states
the reason plainly: the code lands in `structuredContent` and in the declared schema, so callers
branch on it rather than string-matching prose. Add variants freely; **do not rename or
repurpose an existing one**, and keep `as_str()` in sync with the enum.

**`next_actions` is the point of the design, not decoration.** A failure should carry enough for
the caller to recover unassisted: what went wrong, the code, and a concrete next call with
arguments. When you add a failure path, add its recovery.

**`src/hw/pins.rs` is the single source of truth for the DIP-40 map.** Both `pinout` and
`safety_preflight` read from it, which is what keeps the reference and the guardrails from
diverging. Do not introduce a second pin table.

**One subprocess runner.** Everything that spawns a process goes through `src/proc/`, which owns
timeouts, output capture, and the SIGTERM → SIGKILL → reap sequence. Do not call
`tokio::process::Command` directly from a tool.

**Every caller-supplied path goes through `src/paths.rs`.** `FIRMWARE_ROOT` confinement
canonicalises and compares before opening, which closes symlink escapes. A path that bypasses
this is a hole. Note the documented limit: it is check-then-use and therefore TOCTOU-imperfect
by construction — it stops a confused or prompt-injected model, not a local attacker racing the
filesystem.

**Serial session semantics.** One operation per session at a time (busy, not queued); one
session per port; read windows clamped to 120 s because a blocking read cannot be aborted once
started. Each of these has a reason recorded in
[Limitations](README.md#limitations-and-known-caveats) — if you change one, change the reason too.

**The firmware's RXD/TXD guard.** `firmware.c` refuses writes to P3.0 and P3.1 and rejects
`WRP 3 hh` outright. Those pins are the only link to the board; a write that strands the session
can only be recovered by physically power-cycling. Keep the guard.

---

## Adding a tool

A tool is not one file. Touch all of these together:

1. `src/lib.rs` — add the canonical name in `names`, and to the `ALL` array. **The array is
   length-annotated (`[&str; 12]`), so forgetting it is a compile error rather than a silent
   omission** — update the length.
2. `src/tools/` — the tool body, returning an `Envelope`.
3. `src/tools/mod.rs` — export it.
4. `src/server.rs` — the thin `#[tool]` shim, using the constant from `names`, never a
   retyped string literal.
5. `src/errors.rs` — any new `ErrorCode` variants, plus their `as_str()` spellings.
6. `tests/` — cover the success path and each failure path.
7. `scripts/smoke.py` — the tool-surface assertion, the `outputSchema` assertion, and an
   invocation if it can run without hardware.
8. `README.md` — the [tool reference](README.md#tool-reference) table, the tool count in the
   prose, and the `MCP tools` badge.

Parameter naming: `port` means a serial device path such as `/dev/cu.usbserial-10`, everywhere.
An 8051 port number (0–3) is `mcu_port`, as in `safety_preflight`. Keeping these distinct is
deliberate — it prevents a device path being passed where a port number belongs.

---

## Tests

Unit tests live beside the code they exercise; integration tests are in [`tests/`](tests/),
organised by concern (`envelope`, `paths`, `runner`, `safety`, `serial`).

**No test may require hardware.** Serial behaviour is tested against a scripted fake port —
follow that pattern. If a change genuinely cannot be verified without a board, say so in the PR
and add it to the "not covered" list in the README rather than leaving it silently untested.

`scripts/smoke.py` is the end-to-end layer: it starts the real release binary and speaks MCP to
it over stdio. Add to it when you change the tool surface or the envelope.

---

## Documentation

Tool names, parameter names, error codes and pin numbers appear in
[`README.md`](README.md), [`circuits.md`](circuits.md), [`todo.md`](todo.md) and
[`firmware/PROTOCOL.md`](firmware/PROTOCOL.md). **A change to any of them should be reflected
everywhere it appears.** Documentation that has drifted from the code is worse than none, because
it is the layer a model caller reads first.

---

## Electrical claims

Cite a source for any electrical claim — a datasheet reference, or the arithmetic shown. Values
that cannot be traced to a source should be marked as such.

This matters more here than in most projects: `safety_preflight` returns verdicts that people
wire boards against, and the difference between a specification point and a design ceiling is
load-bearing. The existing rules distinguish blockers, warnings, informational notes, and
budgets that a single pin is insufficient to evaluate. Preserve that distinction; do not promote
an advisory to a blocker without a source that supports it.

---

## Commits and pull requests

**Commits** are one line, imperative mood, describing the change and its purpose. No trailers, no
body unless something genuinely needs explaining. Existing history is the reference:

```
Fix symlink escape in output paths, enforce one session per port, clamp serial timeouts
Stamp sessions with a generation so a stale check-in cannot hijack a reused id
```

**Pull requests** should state what changed and why, note anything you could not verify (an
untested hardware path, say), and confirm the five checks pass. Keep unrelated changes in
separate PRs — a refactor bundled with a fix is hard to review and harder to revert.

---

## Reporting a bug

Open an issue including:

- what you asked the assistant to do, or the raw tool call and its arguments,
- **the full response envelope**, which carries `error_code`, `command`, `exit_code` and captured
  `stderr` — this is usually sufficient on its own,
- `doctor` output, for anything touching the toolchain,
- your macOS version and architecture, `rustc --version`, and the versions of `sdcc`, `stcgal`
  and the resolved `rmcp`,
- for hardware issues: the exact MCU part, the USB-TTL adapter chipset (CP2102, CH340, …), the
  device path, and the wiring.

Setting `RUST_LOG=debug` yields more detail on stderr. Redact anything you would not want public
— envelopes can contain file paths.

---

## Reporting a security issue

Please do **not** open a public issue for a security vulnerability. Report it privately through
GitHub's [security advisory](https://github.com/darthvader58/mcp8051/security/advisories/new)
form, which is visible only to the maintainers until a fix is published.

Worth knowing what is in scope: `FIRMWARE_ROOT` confinement is documented as check-then-use and
therefore TOCTOU-imperfect. A local filesystem race between check and open is a known and stated
limitation rather than a new finding. A path that escapes confinement without such a race is a
genuine bug — please report it.

---

## Code of conduct

Participation in this project is governed by the [Code of Conduct](CODE_OF_CONDUCT.md). By taking
part, you agree to uphold it.
