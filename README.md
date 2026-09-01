<div align="center">

# mcs51-mcp

**A Model Context Protocol server for the 8051 (MCS-51) development loop.**

Detect the toolchain, compile firmware with SDCC, flash the chip with stcgal,
and hold a live serial conversation with the board — all driven by an AI assistant.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](https://www.rust-lang.org)
[![MCP SDK](https://img.shields.io/badge/rmcp-3.1.4-green.svg)](https://github.com/modelcontextprotocol/rust-sdk)
[![Platform](https://img.shields.io/badge/platform-macOS%20arm64-lightgrey.svg)](#requirements)
[![Tools](https://img.shields.io/badge/MCP%20tools-12-blueviolet.svg)](#tool-reference)

</div>

---

## Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Requirements](#requirements)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Tool reference](#tool-reference)
- [Response envelope](#response-envelope)
- [Configuration](#configuration)
- [Hardware safety](#hardware-safety)
- [Reference firmware](#reference-firmware)
- [Limitations and known caveats](#limitations-and-known-caveats)
- [Development](#development)
- [Project layout](#project-layout)
- [Contributing](#contributing)
- [License](#license)

---

## Overview

`mcs51-mcp` exposes the complete 8051 development workflow to an MCP client such as Claude
Desktop or Claude Code. Rather than switching between a compiler, a flashing utility, and a
serial terminal, you describe the goal in plain language and the assistant drives the tools.

> **The server is host-side.** It runs as an ordinary process on your Mac. Your MCP client
> speaks JSON-RPC to it over stdio; it invokes `sdcc`, `packihx` and `stcgal` as subprocesses
> and owns the `/dev/cu.*` serial handle. The microcontroller is not aware of MCP — it runs a
> small reference firmware that answers a line-oriented UART protocol.

### Capabilities

| | |
|---|---|
| **Toolchain detection** | `doctor` probes `sdcc`, `packihx` and `stcgal`, reporting paths, versions, and the exact command to install anything missing. |
| **Complete build path** | SDCC compilation → `packihx` → Intel HEX validation → `stcgal` flash. |
| **Persistent serial sessions** | Open a port, write lines, read replies, block until an expected substring arrives, then close. Sessions survive across tool calls. |
| **Electrical preflight** | `safety_preflight` evaluates a proposed pin assignment against the datasheet's current limits before anything is wired. |
| **Datasheet reference** | `pinout` answers DIP-40 pin questions without opening a PDF. |
| **Machine-actionable errors** | Every tool returns an identical structured envelope carrying a stable `error_code` and a `next_actions` list, so a failure can be recovered from rather than merely reported. |

---

## Architecture

```
   ┌──────────────────────┐
   │   MCP client         │   Claude Desktop · Claude Code
   └──────────┬───────────┘
              │  stdio / JSON-RPC  (MCP protocol 2025-11-25)
              ▼
   ┌──────────────────────────────────────────────┐
   │                 mcs51-mcp                    │   host process, macOS arm64
   │  12 tools · shared response envelope         │
   └────────┬────────────────────────┬────────────┘
            │ tokio::process         │ serialport
            ▼                        ▼
   ┌────────────────────┐   ┌──────────────────────┐
   │ sdcc · packihx     │   │  /dev/cu.usbserial-* │
   │ stcgal             │   │  USB-TTL adapter     │
   └────────────────────┘   └──────────┬───────────┘
                                       │ UART 9600 8N1
                                       ▼
                            ┌──────────────────────┐
                            │  STC89C52RC          │  P3.0 RXD / P3.1 TXD
                            │  reference firmware  │  PING · SET · GET · WRP · RDP
                            └──────────────────────┘
```

A single USB-TTL adapter serves both roles on the STC89: it carries the `stcgal` flash and the
subsequent serial conversation. This is the primary reason the STC89C52RC is the default target
rather than the AT89S52 — see [Limitations](#limitations-and-known-caveats).

---

## Requirements

| Component | Version | Notes |
|---|---|---|
| macOS | Apple Silicon (arm64) | Developed and verified on an M4. Intel Macs and Linux are untested. |
| Rust | 1.88 or later | Required by `rmcp` 3.x. Verified with 1.90. |
| SDCC | 4.6.0 | Provides both `sdcc` and `packihx`. |
| stcgal | 1.10 | STC serial-bootloader flasher. |
| USB-TTL adapter | CP2102 recommended | CP2102 requires no driver on Apple Silicon. CH340 needs the WCH DriverKit driver. |
| Target MCU | STC89C52RC (DIP-40) | AT89S52 is pin-compatible but cannot be flashed on macOS. |

Further reading:

| Document | Contents |
|---|---|
| [`todo.md`](todo.md) | Step-by-step setup checklist, in the order you actually perform it |
| [`circuits.md`](circuits.md) | Wiring diagrams, bill of materials, and electrical limits |
| [`firmware/PROTOCOL.md`](firmware/PROTOCOL.md) | Complete serial protocol specification |

---

## Installation

### 1. Install the toolchain

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# SDCC — the 8051 C compiler. packihx ships alongside it.
brew install sdcc

# stcgal — the STC89 serial-bootloader flasher.
brew install pipx
pipx install stcgal
```

Verify each tool:

| Tool | Command | Expected output |
|---|---|---|
| `sdcc` | `sdcc --version` | `SDCC : mcs51/... 4.6.0 #16555` |
| `packihx` | `which packihx` | `/opt/homebrew/bin/packihx` |
| `stcgal` | `stcgal --version` | `stcgal 1.10` |

> [!NOTE]
> `packihx` has no `--version` or `-h` flag and exits `0` regardless of its arguments. Confirm
> its presence with `which`, not by requesting a version. `doctor` accounts for this and reports
> it as `present: true, version: null`.

### 2. Build

```bash
git clone https://github.com/shashwatraj/mcp8051.git
cd mcp8051
cargo build --release
```

The binary is written to `target/release/mcs51-mcp`.

### 3. Register with your MCP client

**Claude Desktop** — edit `~/Library/Application Support/Claude/claude_desktop_config.json`.
Use the absolute path to the built binary; this project is not distributed via npm.

```json
{
  "mcpServers": {
    "mcs51": {
      "command": "/absolute/path/to/mcp8051/target/release/mcs51-mcp",
      "env": {
        "FIRMWARE_ROOT": "/absolute/path/to/mcp8051/firmware",
        "RUST_LOG": "info"
      }
    }
  }
}
```

Restart Claude Desktop after editing the file.

**Claude Code** — register the same binary from the CLI:

```bash
claude mcp add mcs51 \
  --env FIRMWARE_ROOT=/absolute/path/to/mcp8051/firmware \
  -- /absolute/path/to/mcp8051/target/release/mcs51-mcp
```

`FIRMWARE_ROOT` is optional but recommended; see [Configuration](#configuration).

---

## Quick start

The workflow is: verify the toolchain, build, flash, then talk to the board.

### Expressed as tool calls

```jsonc
doctor  {}
// → sdcc 4.6.0, packihx present (no version), stcgal 1.10

compile  { "source": "firmware.c" }
// → firmware.hex written and validated: non-empty, begins with ':', and contains
//   the Intel-HEX EOF record :00000001FF. packihx always exits 0, so its exit
//   status proves nothing and the artifact itself is checked.

flash  { "chip": "stc", "hex": "firmware.hex", "port": "/dev/cu.usbserial-10" }
// → stcgal begins waiting for the bootloader handshake.
//   Cut power to the board and reapply it now, while stcgal is waiting.
//   The STC89 enters its bootloader only on power-up, so the order matters.

serial_open  { "port": "/dev/cu.usbserial-10", "baud": 9600, "session": "board" }
serial_write { "session": "board", "data": "PING" }
serial_expect{ "session": "board", "pattern": "PONG", "timeout_ms": 2000 }
// → matched "PONG" after 41 ms
```

### Expressed the way you would actually ask

> "Run a doctor check first. If the toolchain is clean, compile `firmware.c` and flash it to the
> STC89 on `/dev/cu.usbserial-10` — tell me when to cut the power, because the STC89 only drops
> into its bootloader on a fresh power-up. Once it's written, open a serial session at 9600, send
> PING, and wait for PONG. If anything fails, read the error and tell me what to fix."

Both express the same sequence. The second is the intended mode of use.

### Other things worth asking

> "Check whether my 8051 toolchain is set up — I just installed sdcc and stcgal."

> "I want to hang an LED off P1.0. Which resistor, which way round, and is that within the
> current budget for that port?"

> "Which pin is XTAL1, and what happens if I leave EA floating?"

---

## Tool reference

Twelve tools, grouped by purpose. Optional parameters are marked; everything else is required.

### Toolchain and discovery

| Tool | Parameters | Description |
|---|---|---|
| `doctor` | — | Probes `sdcc`, `packihx` and `stcgal`; reports paths, versions and path-confinement state. |
| `list_serial_ports` | — | Enumerates serial devices, ranking `/dev/cu.*` above `/dev/tty.*`. |
| `pinout` | `pin` *(optional, 1–40)* | DIP-40 pin reference. Omit `pin` for the complete map. |

### Build and flash

| Tool | Parameters | Description |
|---|---|---|
| `compile` | `source`, `out` *(optional)* | Runs `sdcc -mmcs51`, then `packihx`, then validates the resulting Intel HEX. |
| `flash` | `chip`, `hex`, `port` | `chip: "stc"` invokes `stcgal -p <port> <hex>`. `chip: "at89s"` returns a documented stub. |

### Serial sessions

| Tool | Parameters | Description |
|---|---|---|
| `serial_open` | `port`, `session`, `baud` *(optional, 9600)* | Opens a port under a session identifier that you choose. |
| `serial_write` | `session`, `data` | Writes a line, appending a newline if absent. |
| `serial_read` | `session`, `timeout_ms` *(optional, 1000)* | Collects output for the window. |
| `serial_expect` | `session`, `pattern`, `timeout_ms` *(optional, 1000)* | Waits for a literal substring, returning the moment it appears. |
| `serial_close` | `session` | Closes the session and releases the device. |
| `serial_list_sessions` | — | Lists open sessions with port, baud, age and state. |

### Safety

| Tool | Parameters | Description |
|---|---|---|
| `safety_preflight` | `mcu_port` *(0–3)*, `bit` *(0–7)*, `level` *(`high`/`low`/`input`)*, `load_ma` *(optional)* | Evaluates a proposed pin assignment against the datasheet limits. |

> [!IMPORTANT]
> Two conventions worth internalising:
>
> - **Session identifiers are caller-supplied.** You pass `session` to `serial_open` and reuse
>   that same string in every subsequent serial call. The server does not mint one for you.
> - **`safety_preflight` takes `mcu_port`, not `port`.** Everywhere else in this server `port`
>   denotes a serial device path such as `/dev/cu.usbserial-10`. The distinct name prevents a
>   device path being passed where an 8051 port number (0–3, meaning P0–P3) belongs.

---

## Response envelope

Every tool returns the same structure, emitted twice: as pretty-printed JSON in the `content`
block for human readability, and as `structuredContent` for programmatic consumption. All twelve
tools share a single declared `outputSchema`.

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | Whether the operation succeeded. |
| `status` | `ok` \| `warning` \| `error` | Finer-grained outcome; `warning` still implies `ok: true`. |
| `tool` | string | The tool that produced the envelope. |
| `error_code` | string | Stable machine-readable code, e.g. `PATH_ESCAPES_FIRMWARE_ROOT`. |
| `error` | string | Human-readable description. |
| `remedy` | string | The corrective action, where one can be named. |
| `command` | string | The exact argv that was executed, when a subprocess was involved. |
| `exit_code` | integer | Subprocess exit status. |
| `duration_ms` | integer | Wall-clock duration. |
| `stdout` / `stderr` | object | Captured output, retained as a capped head and tail. |
| `data` | object | Tool-specific payload. |
| `next_actions` | array | Suggested follow-up or recovery calls, with arguments. |

`next_actions` is the field that makes the design worthwhile. A failed `compile` returns an
`error_code`, SDCC's diagnostics, and a concrete next call — enough for the assistant to correct
the source and retry without further instruction.

---

## Configuration

### Path confinement and logging

| Variable | Default | Description |
|---|---|---|
| `FIRMWARE_ROOT` | unset | Confines file access. Relative paths resolve beneath it, and every path must canonicalise to a location inside it, which closes symlink escapes. A path resolving outside returns `PATH_ESCAPES_FIRMWARE_ROOT`. If set but missing or not a directory, the server fails at startup rather than silently downgrading the boundary. When unset, paths are unrestricted and `doctor` reports `confinement: "off"`. |
| `RUST_LOG` | unset | Standard `tracing` filter, e.g. `info` or `mcs51_mcp=debug`. Diagnostics are written to stderr; stdout is reserved for the MCP channel. |

### Tuning

Each accepts a positive integer. Unparseable or zero values fall back to the default rather than
failing at startup. These are the names the server's own error remedies reference.

| Variable | Default | Description |
|---|---|---|
| `MCS51_MCP_COMPILE_TIMEOUT_MS` | `60000` | Budget for one `sdcc` or `packihx` invocation. |
| `MCS51_MCP_FLASH_TIMEOUT_MS` | `180000` | Budget for one `stcgal` invocation. Deliberately generous, as it waits for a manual power-cycle. |
| `MCS51_MCP_PROBE_TIMEOUT_MS` | `15000` | Budget for a `doctor` version probe. |
| `MCS51_MCP_CAPTURE_BYTES` | `65536` | Bytes retained per captured stream (head plus tail). Floored at 512. |
| `MCS51_MCP_SERIAL_READ_BYTES` | `65536` | Bytes retained from a single `serial_read`. Floored at 512. |
| `MCS51_MCP_MAX_SESSIONS` | `8` | Concurrent serial sessions permitted. Clamped to 1–256. |

---

## Hardware safety

### The asymmetry that governs 8051 design

An 8051 port pin sinks current well and sources almost none. The datasheet specifies `VOL` at
`IOL = 1.6 mA`, but specifies `VOH` at `IOH = −60 µA` — roughly a 26× gap at the specification
points. This single characteristic is why 8051 peripherals are conventionally wired **active-low**.

The reference LED circuit follows accordingly:

```
  +5V ──[ 330 Ω ]──▶|── P1.0 (pin 1)      sinks ≈ 7.7 mA
                    LED
```

`SET 1 0 0` turns the LED **on** by pulling the pin low; `SET 1 0 1` turns it off. The intuitive
alternative — `P1.0 → 220 Ω → LED → GND` — requires the pin to *source* roughly 10 mA from a
driver specified at 60 µA, and will appear dim or entirely dead.

### What `safety_preflight` evaluates

The tool sees one pin per call, which divides its rules into conditions it can decide and budgets
it can only report.

**Decided**, against the `level` and `load_ma` supplied:

| Condition | Threshold | Verdict |
|---|---|---|
| `mcu_port` / `bit` within range | 0–3 and 0–7 | **Blocker** outside — never wrapped or clamped |
| Sourcing a load | `IOH = −60 µA` specification; anything above 0.1 mA | **Blocker** — the pin cannot supply it |
| Per-pin sink current | 10 mA maximum `IOL` | **Blocker** above; **warning** above 8 mA |
| Port 0 driven high | No internal pull-up in I/O mode | **Warning** generally; **blocker** when driving a load, since writing 1 releases the pin rather than driving it |
| P3.0 / P3.1 as GPIO | RXD / TXD | **Blocker**; escalated when a serial session is open |
| P1.5 / P1.6 / P1.7 | MOSI / MISO / SCK | **Warning** — shared with an ISP programmer |
| P3.2 / P3.3, P0 / P2 | INT0 / INT1, AD0–AD7 / A8–A15 | **Info** — alternate functions |
| `load_ma` omitted | — | **Info** — current checks were skipped, not passed |

**Reported as advisories**, because a single pin is insufficient to evaluate them:

| Budget | Limit |
|---|---|
| Port 0 aggregate | 26 mA |
| Ports 1 / 2 / 3 aggregate | 15 mA each |
| All ports combined | 71 mA |
| Absolute maximum DC output | 15.0 mA (the enforced design ceiling remains 10 mA) |

`VIN` (−1.0 V to +7.0 V) is not evaluated here, as `safety_preflight` accepts no voltage
argument; it is reported by `pinout` for non-I/O pins. See [`circuits.md`](circuits.md) §8.

---

## Reference firmware

[`firmware/firmware.c`](firmware/firmware.c) implements a newline-terminated ASCII protocol over
UART at 9600 8N1. It compiles to 1,883 bytes — 23% of the STC89C52's 8 KB flash.

| Command | Response | Meaning |
|---|---|---|
| `PING` | `PONG` | Liveness check |
| `SET p b v` | `OK` | Drive bit `b` of port `p` to value `v` |
| `GET p b` | `0` / `1` | Read bit `b` of port `p` |
| `WRP p hh` | `OK` | Write byte `hh` to port `p` |
| `RDP p` | `hh` | Read port `p` |
| *(anything else)* | `ERR` | Rejected |

The firmware refuses writes to **P3.0 and P3.1**, returning `ERR`. Those pins carry RXD and TXD —
the only link between the server and the board — so a stray `SET 3 0 0` would strand the session
until the board was physically power-cycled. `WRP 3 hh` is rejected outright for the same reason,
since a whole-port write necessarily touches both. Individual writes to P3.2–P3.7 are permitted.

The complete grammar, worked exchanges and shadow-latch semantics are documented in
[`firmware/PROTOCOL.md`](firmware/PROTOCOL.md).

---

## Limitations and known caveats

### Verification status

**No 8051 was attached during development.** The distinction between what is automatically
verified and what is not matters, so it is stated precisely.

Automatically verified:

- **`cargo test` — 59 tests** covering the envelope contract, the path sandbox, the subprocess
  runner (including SIGTERM → SIGKILL → reap, and draining an unbounded writer), the session
  registry's checkout/check-in state machine against a scripted fake port, and every
  `safety_preflight` rule.
- **`scripts/smoke.py` — 43 assertions** driving the real release binary over stdio. It asserts
  the twelve-tool surface and that each declares an `outputSchema`, then invokes nine of them,
  checking the envelope contract, the safety verdicts, and that failure paths return structured
  errors rather than panicking.

Not covered:

- `serial_write`, `serial_expect` and `serial_close` are exercised by `cargo test` against a fake
  port, but are not called through the binary by the smoke harness — they require a board to
  return anything meaningful.
- A *successful* `compile` is not part of the harness; its only `compile` call is a deliberate
  miss. The reference firmware does build (`sdcc -mmcs51 firmware.c`, then `packihx`), but that
  is a manual check rather than an automated gate.
- An actual `stcgal` flash to physical hardware, and a real `PING`/`PONG` round trip over a
  physical UART.

The serial and flash paths are written against verified tool behaviour and the datasheet, but
have not met silicon.

### Design constraints

**The STC89 must be power-cycled to flash.** It enters its serial bootloader only on power-up.
`stcgal` accommodates this by waiting for the handshake, so the correct order is to issue `flash`
*first*, then cut power and reapply it. Power-cycling beforehand misses the window.

**One serial operation per session at a time.** A concurrent operation on the same session returns
a busy error rather than queueing. This suits a single operator and a single board, which is the
intended scope; it is not a multi-tenant design.

**One session per port.** `serial_open` refuses a port already held, returning
`PORT_HELD_BY_SESSION` and naming the holder. Beyond the fact that the OS generally refuses a
second open of a `/dev/cu.*` node, this keeps "which session holds this port?" a question with
exactly one answer — which `flash` relies upon when advising you which session to close.

**Read windows are capped at 120 s.** `serial_read` and `serial_expect` clamp `timeout_ms` to that
ceiling and echo the effective value in the envelope. A read occupies a blocking thread that
cannot be aborted once started, so an unbounded window could hold that thread — and the MCP
request — indefinitely.

**AT89S52 has no clean macOS flashing path.** `flash(chip: "at89s")` is a documented stub that
returns an explanatory error listing hardware alternatives. The AT89S programs over SPI ISP and
requires a dedicated programmer; it has no equivalent to the STC89's serial bootloader. This is
why the STC89C52RC is the default target.

**Path confinement is check-then-use.** `FIRMWARE_ROOT` canonicalises and compares before opening.
That window is TOCTOU-imperfect by construction. It reliably prevents a confused or
prompt-injected model from emitting a path such as `../../.ssh/id_rsa`, but it is not a sandbox
and will not stop a local attacker able to race the filesystem between check and open.

### Dependency notes

**rmcp version drift.** This targets rmcp 3.x, verified against 3.1.4. The SDK progressed from
0.x through 3.x rapidly, and older examples still reference `Content::text` and `rmcp::Error`,
neither of which exists in 3.x. When upgrading, re-verify the resolved API against the version
Cargo actually selects rather than adapting an example without checking which major release it
targeted.

**schemars derive pairing.** Should the `JsonSchema` derive report a crate-resolution error, use
`rmcp::schemars` and annotate the affected type with `#[schemars(crate = "rmcp::schemars")]` so
the derive and the SDK agree on which `schemars` is meant.

---

## Development

```bash
cargo build --release                        # build
cargo clippy --all-targets -- -D warnings    # lint; warnings are errors
cargo fmt                                    # format
cargo test                                   # 59 unit and integration tests
python3 scripts/smoke.py                     # 43-assertion end-to-end stdio harness
```

`scripts/smoke.py` drives the real release binary over stdio and asserts the twelve-tool surface,
the envelope contract, and the safety rules. It requires no hardware and should pass before every
commit.

To rebuild the reference firmware:

```bash
cd firmware
sdcc -mmcs51 firmware.c && packihx firmware.ihx > firmware.hex
```

SDCC writes its intermediates alongside the source; all are gitignored.

---

## Project layout

```
├── src/
│   ├── main.rs           binary entry point; tracing to stderr, then serve stdio
│   ├── lib.rs            crate root and canonical tool names
│   ├── server.rs         rmcp wiring: server state and the twelve tool shims
│   ├── envelope.rs       the shared response envelope and its output schema
│   ├── errors.rs         closed ErrorCode taxonomy and AppError
│   ├── config.rs         environment-derived configuration
│   ├── paths.rs          FIRMWARE_ROOT resolution and confinement
│   ├── proc/             the single subprocess runner: timeouts, capture, reaping
│   ├── serial/           session registry, blocking I/O, port enumeration
│   ├── tools/            the twelve tool implementations
│   └── hw/               DIP-40 pin table and datasheet constants
├── tests/                integration tests, no hardware required
├── firmware/
│   ├── firmware.c        SDCC reference firmware
│   └── PROTOCOL.md       serial protocol specification
├── scripts/smoke.py      end-to-end stdio harness
├── circuits.md           wiring, bill of materials, electrical limits
└── todo.md               setup checklist
```

`src/hw/pins.rs` is the single source of truth for the DIP-40 map and is consumed by both
`pinout` and `safety_preflight`, so the reference and the guardrails cannot diverge.

---

## Contributing

Contributions are welcome. Before opening a pull request:

1. Ensure `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
   and `cargo test` all pass.
2. Run `python3 scripts/smoke.py` and confirm all assertions pass.
3. Keep documentation synchronised with the code. Tool names, parameter names, error codes and
   pin numbers appear in `README.md`, `circuits.md`, `todo.md` and `firmware/PROTOCOL.md`; a
   change to any of them should be reflected everywhere it appears.
4. Cite a source for electrical claims — a datasheet reference or shown arithmetic. Values that
   cannot be traced should be marked as such.

Hardware validation is especially valuable: if you flash the reference firmware to a physical
STC89C52RC and exercise the serial round trip, please report the result in an issue.

---

## License

Released under the MIT License. See [`LICENSE`](LICENSE).

## Acknowledgements

Built on the [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) (`rmcp`),
[SDCC](https://sdcc.sourceforge.net/), [stcgal](https://github.com/grigorig/stcgal), and the
[`serialport`](https://github.com/serialport/serialport-rs) crate. The tool surface and
developer-experience conventions take inspiration from
[`arduino-mcp-server`](https://github.com/hardware-mcp/arduino-mcp-server), adapted for the
MCS-51 toolchain and its very different hardware constraints.
