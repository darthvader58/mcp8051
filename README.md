# mcs51-mcp

![license](https://img.shields.io/badge/license-MIT-blue)
![rust](https://img.shields.io/badge/rust-edition%202021-orange)
![mcp](https://img.shields.io/badge/MCP-rmcp%203.1.4-green)
![platform](https://img.shields.io/badge/platform-macOS%20arm64-lightgrey)

An MCP stdio server that hands an AI assistant the whole 8051 development loop — detect the
toolchain, compile C, flash the chip, then talk to it over the serial line and check what it
said back.

**It runs on your Mac, not on the microcontroller.** People get this backwards constantly, so
to be explicit: `mcs51-mcp` is a host-side process. Your MCP client (Claude Desktop, Claude
Code) speaks JSON-RPC to it over stdio; it shells out to `sdcc`, `packihx` and `stcgal`, and
holds the `/dev/cu.*` serial handle. The 8051 has no idea MCP exists. It runs a small reference
firmware that answers a line-oriented UART protocol (`PING` → `PONG`).

- **Toolchain-aware.** `doctor` probes `sdcc`, `packihx` and `stcgal` and tells you exactly
  what is missing and how to install it.
- **Full build path.** SDCC compile → `packihx` → validated Intel HEX → `stcgal` flash.
- **Real serial sessions.** Open a port, write lines, read replies, block until an expected
  string arrives, close.
- **Hardware-safety checks.** `safety_preflight` catches the electrical mistakes that
  actually kill 8051 pins before you wire them.
- **Datasheet on tap.** `pinout` answers DIP-40 pin questions without a PDF.
- **Recoverable failures.** Every tool returns the same structured envelope, so the assistant
  can act on an error instead of just relaying it.

---

## Architecture

```
   ┌──────────────────┐
   │  Claude Desktop  │
   │  or Claude Code  │
   └────────┬─────────┘
            │  stdio / JSON-RPC   (MCP 2025-11-25)
            ▼
   ┌──────────────────────────────────────┐
   │              mcs51-mcp               │  host-side: runs on YOUR Mac (arm64)
   └───────┬──────────────────────┬───────┘
           │ subprocess           │ serialport crate
           ▼                      ▼
   ┌───────────────────┐   ┌──────────────────┐
   │ sdcc · packihx    │   │  /dev/cu.usb*    │
   │ stcgal            │   │  USB-TTL (CP2102)│
   └───────────────────┘   └────────┬─────────┘
                                    │ UART 9600 8N1
                                    ▼
                           ┌──────────────────────┐
                           │  STC89C52            │  P3.0 RXD / P3.1 TXD
                           │  + reference firmware│  PING · SET · GET · WRP · RDP
                           └──────────────────────┘
```

Every tool returns the **same structured envelope**, emitted twice: as pretty-printed JSON in
the `content` block (so a human reading the transcript can see it) and as `structuredContent`
(so the model can parse it). The fields are:

| Field | Meaning |
|---|---|
| `ok` | boolean — did the operation succeed |
| `status` | `ok` \| `warning` \| `error` |
| `tool` | the tool that produced this envelope |
| `error_code` | stable machine-readable code, e.g. `PATH_ESCAPES_FIRMWARE_ROOT` |
| `error` | human-readable message |
| `command` | the exact subprocess command line that was run, when there was one |
| `exit_code` | subprocess exit status |
| `duration_ms` | wall-clock time |
| `stdout` / `stderr` | captured output, capped head + tail |
| `data` | tool-specific payload |
| `next_actions` | suggested recovery or follow-up steps |

That last field is the point. A failed `compile` does not just say "failed" — it comes back with
an `error_code`, the SDCC stderr, and a `next_actions` list, which is enough for the assistant to
fix the source and retry on its own.

---

## Quick Start

### 1. Install the toolchain (macOS, Apple Silicon — native arm64)

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# SDCC — the 8051 C compiler. packihx ships with it.
brew install sdcc

# stcgal — the STC89 serial-bootloader flasher.
brew install pipx
pipx install stcgal
```

Verified versions on this project:

| Tool | Check | Expected |
|---|---|---|
| `sdcc` | `sdcc --version` | `SDCC : mcs51/... 4.6.0 #16555` |
| `packihx` | `which packihx` | `/opt/homebrew/bin/packihx` |
| `stcgal` | `stcgal --version` | `stcgal 1.10` |

> `packihx` has **no** `--version` or `-h` flag, and exits 0 no matter what you pass it. Verify
> it with `which`, not by asking it for a version. `doctor` knows this and reports it as
> `present: true, version: null`.

### 2. Build

```bash
cargo build --release
```

The binary lands at `target/release/mcs51-mcp`.

### 3. Register it with your client

**Claude Desktop** — `~/Library/Application Support/Claude/claude_desktop_config.json`. Use the
**absolute path to the built binary**; there is no `npx` package for this.

```json
{
  "mcpServers": {
    "mcs51": {
      "command": "/Users/you/mcp8051/target/release/mcs51-mcp",
      "env": {
        "FIRMWARE_ROOT": "/Users/you/mcp8051/firmware",
        "RUST_LOG": "info"
      }
    }
  }
}
```

`FIRMWARE_ROOT` is optional — see [Configuration](#configuration). Restart Claude Desktop after
editing the file.

**Claude Code** — same binary, registered from the CLI:

```bash
claude mcp add mcs51 \
  --env FIRMWARE_ROOT=/Users/you/mcp8051/firmware \
  -- /Users/you/mcp8051/target/release/mcs51-mcp
```

---

## What you can say

Plain English, straight into the chat. No tool names required.

> "Check whether my 8051 toolchain is set up — I just installed sdcc and stcgal and I want to
> know if you can actually see them."

> "Compile the reference firmware, then flash it to the board on the CP2102 adapter. I'll
> power-cycle when you tell me to."

> "Open a serial session at 9600 baud, send PING, and tell me if the board answers PONG."

> "I want to hang an LED off P1.0. Which resistor, which way round, and is that within the
> current budget for that port?"

---

## Tools

Twelve tools, grouped by what they are for.

| Group | Tool | Parameters | What it does |
|---|---|---|---|
| Toolchain | `doctor` | — | Probes `sdcc`, `packihx`, `stcgal`; reports paths, versions, and `confinement` state |
| Discovery | `list_serial_ports` | — | Lists serial devices, ranking `/dev/cu.*` above `/dev/tty.*` and explaining why |
| Discovery | `pinout` | `pin` (optional, `1`–`40`) | DIP-40 pin reference; omit `pin` for the whole map, pass one for a single pin |
| Build | `compile` | `source` (path to `.c`) | `sdcc -mmcs51` → `packihx` → validated `.hex` |
| Build | `flash` | `port`, `hex`, `chip` (`stc` default, `at89s` stub) | `stcgal -p <port> <hex>` |
| Serial | `serial_open` | `port`, `baud` (9600 for the reference firmware) | Opens a session, returns a session id |
| Serial | `serial_write` | session id, data | Writes a newline-terminated line to the session |
| Serial | `serial_read` | session id, timeout | Reads whatever is buffered, up to the timeout |
| Serial | `serial_expect` | session id, expected string, timeout | Blocks until the string arrives or the timeout expires |
| Serial | `serial_close` | session id | Closes the session and releases the device |
| Serial | `serial_list_sessions` | — | Lists open sessions |
| Safety | `safety_preflight` | `mcu_port` (**0–3**, meaning P0–P3), plus the pin and load you intend to drive | Checks a proposed circuit against the datasheet limits |

<!-- TODO(lead): the verified brief freezes the 12 tool names, `pinout.pin`, `flash.chip`,
     `safety_preflight.mcu_port`, and the fact that the serial tools are session-scoped. It does
     NOT freeze the remaining argument *names* or the shape of `serial_open`'s return value —
     specifically: the session-id field name, timeout units (`timeout_ms`?), `compile`'s output
     path argument, and `safety_preflight`'s load arguments. Reconcile BOTH this Parameters
     column AND the jsonc block under "Usage flow" against src/ before tagging a release. -->

### A note on `safety_preflight`'s first parameter

It is called **`mcu_port`**, not `port`, deliberately. Everywhere else in this server `port`
means a *serial device path* (`/dev/cu.usbserial-10`). If `safety_preflight` also used `port`,
callers would keep passing a device path where the 8051 port **number** belongs. `mcu_port`
takes `0`–`3`, meaning P0–P3.

---

## Usage flow

The loop is: check the toolchain, build, flash, then talk to the board.

### As literal tool calls

```jsonc
doctor               {}
// → ok: sdcc 4.6.0, packihx present (no version), stcgal 1.10

compile              { "source": "firmware.c" }
// → ok: firmware.hex written and validated: non-empty, starts with ':', and
//   contains the Intel-HEX EOF record :00000001FF (packihx always exits 0, so
//   its exit code proves nothing and the .hex itself must be checked)

flash                { "chip": "stc", "port": "/dev/cu.usbserial-10", "hex": "firmware.hex" }
// stcgal now BLOCKS waiting for the bootloader handshake.
// ---- cut power to the board and reapply it NOW, while stcgal is waiting. ----
// ---- The STC89 only enters its bootloader on power-up. Order matters.     ----
// → ok: stcgal handshake succeeded, image written

serial_open          { "port": "/dev/cu.usbserial-10", "baud": 9600, "session": "s1" }
// → ok. The session id is supplied by YOU, not minted by the server, so the
//   assistant can pick a stable handle and reuse it across later calls.

serial_write         { "session": "s1", "data": "PING" }
// → ok: a trailing newline is appended if you did not supply one

serial_expect        { "session": "s1", "pattern": "PONG", "timeout_ms": 2000 }
// → ok: matched "PONG" after 41 ms — expect returns the instant the substring
//   appears, it does not burn the full timeout window on success
```

### As the thing you would actually type

> "Run a doctor check first. If the toolchain is clean, compile `firmware.c` and flash it to
> the STC89 on `/dev/cu.usbserial-10` — tell me when to cut the power, because the STC89 only
> drops into its bootloader on a fresh power-up. Once it's written, open a serial session at
> 9600, send PING, and wait for PONG. If anything fails, read the error and tell me what to fix."

Both do the same thing. The second one is how this server is meant to be used.

The firmware speaks a newline-terminated ASCII protocol — `PING`/`PONG`, `SET p b v`, `GET p b`,
`WRP p hh`, `RDP p`, and `ERR` for anything invalid. Full grammar in
[`firmware/PROTOCOL.md`](firmware/PROTOCOL.md).

---

## Safety guardrails

### The asymmetry that governs every 8051 design

An 8051 port pin **sinks current well and sources almost none**. The datasheet specifies `VOL`
at `IOL = 1.6 mA`, but specifies `VOH` at `IOH = −60 µA`. That is not a typo and not a rounding
difference — it is roughly a 26× gap at the specification points, and it is the single reason
8051 peripherals are wired **active-low**.

So the demo LED is wired:

```
  +5V ──[ 330 Ω ]──▶|── P1.0 (pin 1)        sinks ~7.7 mA
                   LED
```

`SET 1 0 0` turns it **on** (pin pulled low, sinking). `SET 1 0 1` turns it **off**.

The naive arrangement — `P1.0 → 220 Ω → LED → GND` — asks the pin to *source* about 10 mA from
a driver rated at 60 µA. It will look dim or dead, and you will spend an hour blaming your code.

### What `safety_preflight` checks

| Check | Limit | Why |
|---|---|---|
| Per-pin sink current | **10 mA** max `IOL` | Exceeding it degrades the output driver |
| Port 0 total | **26 mA** | Per-port budget, separate from the per-pin one |
| Ports 1 / 2 / 3 total | **15 mA** each | Tighter than Port 0 |
| All ports combined | **71 mA** | Whole-chip output budget |
| Absolute max DC output | **15.0 mA** | Beyond this is damage, not degradation |
| Input voltage on any pin | **−1.0 V to +7.0 V** | Absolute maximum rating |
| Sourcing a load | `IOH = −60 µA` | Flags any design that expects the pin to source |
| Port 0 pull-ups | Open-drain, **no internal pull-up** in I/O mode | Port 0 needs external pull-ups (10 kΩ typical); P1/P2/P3 have weak internal ones |

The reference firmware adds one guard of its own: **writes to P3.0 and P3.1 return `ERR`**.
Those are RXD and TXD — the only link the server has to the board. A stray `SET 3 0 0` would
strand the session until someone physically power-cycles the board, so the firmware refuses it.

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `FIRMWARE_ROOT` | unset (unrestricted) | Confines file paths. Relative paths resolve **under** it, and every path must `fs::canonicalize` to a location **inside** it — which closes the symlink escape. A path that resolves outside returns `PATH_ESCAPES_FIRMWARE_ROOT`. If it is set but missing or not a directory, the server **fails at startup, loudly** — it never silently downgrades a security boundary. When unset, paths are unrestricted and `doctor` reports `confinement: "off"` so the state is visible rather than assumed. |
| `RUST_LOG` | unset | Standard `tracing` filter, e.g. `info` or `mcs51_mcp=debug`. Logs go to **stderr** — stdout is the MCP channel and must stay clean. |

---

## Caveats & limitations

Read this section. It is the one most projects leave out.

**Not verified on real hardware.** This is the big one. What *is* tested: the build, toolchain
detection, the reference firmware compiling end to end, and the full MCP surface — all twelve
tools, the envelope contract, and the safety rules, driven through the real binary over stdio by
`scripts/smoke.py`. What is **not** tested: an actual `stcgal` flash to a physical STC89C52, and
a real `PING`/`PONG` round-trip over a real UART. No 8051 was attached during development. The
serial and flash paths are written against verified tool behavior and the datasheet, but they
have not met silicon.

**rmcp version drift.** This targets **rmcp 3.x**, verified against **3.1.4**. The SDK went
0.x → 1.x → 2.x → 3.x quickly, and older examples on the internet still use `Content::text` and
`rmcp::Error` — neither of which exists any more. If you bump the dependency, re-verify the
resolved API against the version you actually got and keep the build green. Do not port an
example in from a blog post without checking which major it was written for.

**schemars derive pairing.** If the `JsonSchema` derive complains about crate resolution, use
`rmcp::schemars` and annotate the type with `#[schemars(crate = "rmcp::schemars")]` so the derive
and the SDK agree on which `schemars` they mean.

**The STC89 must be power-cycled to flash.** It only enters its serial bootloader on power-up.
`stcgal` handles this by waiting for the handshake — so the order is: issue `flash` *first*, then
cut power to the board and reapply it. If you power-cycle before issuing the command you will
miss the window and `stcgal` will sit there timing out.

**One serial operation per session at a time.** A second concurrent operation on the same session
returns a clean busy error rather than queueing behind the first. That is correct for one user and
one board, which is what this is. It is not a multi-tenant design and does not pretend to be.

**AT89S has no clean macOS flashing path.** `flash(chip="at89s")` is a **documented stub** — it
returns an explanatory error, not a flash. The AT89S programs over SPI ISP and needs a hardware
programmer; there is no equivalent of the STC89's serial bootloader. Use an STC89C52RC, which is
why it is the default: one USB-TTL adapter both flashes it and talks to it.

**Path confinement is check-then-use.** `FIRMWARE_ROOT` canonicalizes and compares, then opens.
That window is TOCTOU-imperfect by construction. It is designed to stop a confused or
prompt-injected model from emitting `../../.ssh/id_rsa`, and it does that reliably. It is **not**
a sandbox and will not stop a local attacker who can race the filesystem between the check and
the open.

---

## Development

```bash
cargo build --release                        # build
cargo clippy --all-targets -- -D warnings    # lint, warnings are errors
cargo fmt                                    # format
cargo test                                   # unit tests
python3 scripts/smoke.py                     # end-to-end stdio harness
```

`scripts/smoke.py` drives the real release binary over stdio and asserts the twelve-tool surface,
the envelope contract, and the safety rules. **It needs no hardware** — run it before every
commit.

---

## See also

- [`circuits.md`](circuits.md) — wiring diagrams and the bill of materials
- [`firmware/PROTOCOL.md`](firmware/PROTOCOL.md) — the serial line protocol grammar
- [`todo.md`](todo.md) — setup checklist, in the order you actually do it

## License

MIT. See [`LICENSE`](LICENSE).
