# Setup checklist — mcs51-mcp on Apple Silicon

A start-to-finish checklist for getting `mcs51-mcp` running on a Mac M-series (M1–M4), in the
order you actually do it. Everything here is native arm64 — no Rosetta.

If you have never touched an 8051 before, follow it top to bottom. Each item stands on its own.

---

## 1. Install the toolchain

- [ ] **Install Rust** via rustup:
      `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- [ ] Open a new shell (or `source "$HOME/.cargo/env"`) so `cargo` is on your `PATH`.
- [ ] Verify: `cargo --version` prints a version. Any recent stable is fine.

- [ ] **Install SDCC** — the 8051 C compiler: `brew install sdcc`
- [ ] Verify: `sdcc --version` → first stdout line reads
      `SDCC : mcs51/z80/... 4.6.0 #16555 (Mac OS X ppc)`.
      The `ppc` in the build string is cosmetic; the binary is native.
- [ ] Verify `packihx` came along with SDCC: `which packihx` → `/opt/homebrew/bin/packihx`
- [ ] Do **not** run `packihx --version`. It has **no `--version` flag** — it treats every
      argument as a filename, prints `packihx: cannot open --version`, and still exits 0.
      `which packihx` is the only reliable check.

- [ ] **Install pipx**: `brew install pipx`
- [ ] **Install stcgal** — the STC89 serial-bootloader flasher: `pipx install stcgal`
- [ ] If `stcgal` is not found afterwards, run `pipx ensurepath` and open a new shell.
      pipx installs to `~/.local/bin`.
- [ ] Verify: `stcgal --version` → `stcgal 1.10`

---

## 2. USB-TTL driver

You need a USB-to-serial adapter. Which one you bought decides whether there is any driver work.

- [ ] **If your adapter is a CP2102** (recommended): nothing to install. It works driverless on
      Apple Silicon. Skip to the next section.
- [ ] **If your adapter is a CH340/CH341**: install the WCH DriverKit driver from
      https://www.wch-ic.com/downloads/CH34XSER_MAC_ZIP.html
- [ ] After installing a CH340 driver, **reboot**, then approve the driver in
      System Settings → Privacy & Security if macOS prompts you.

---

## 3. Find your serial device

- [ ] Plug the adapter into the Mac.
- [ ] List the candidates: `ls /dev/cu.usbserial* /dev/cu.wchusbserial*`
- [ ] Note the exact path you get, e.g. `/dev/cu.usbserial-10`. You will paste it into every
      `flash` and `serial_open` call.
- [ ] **Use `/dev/cu.*`, never `/dev/tty.*`.** macOS enumerates the *same physical device* twice:
      `tty.*` is the dial-in device and `cu.*` is the callout device. Opening `tty.*` blocks
      waiting for carrier detect (DCD), which a plain 3-wire USB-TTL adapter never asserts — so
      your open just hangs with no error. `cu.*` does not wait.
- [ ] Sanity check: unplug the adapter, re-run the `ls`, and confirm the path disappears. That
      proves you identified the right device and not some other serial port.

---

## 4. Hardware to buy

The full bill of materials for the reference build. Wiring is in
[`circuits.md`](circuits.md).

- [ ] **STC89C52RC, DIP-40** — the MCU. DIP-40 specifically, so it fits a breadboard socket.
- [ ] **11.0592 MHz crystal** — **this exact frequency, not 12 MHz.** The UART baud rate divides
      down from the crystal: `11.0592 MHz / (12 × 32 × (256 − 0xFD)) = 9600` **exactly**. A
      12 MHz crystal gives `12 MHz / 1152 = 10416.7` baud — **+8.5% error**, far outside the
      ~2–3% a UART tolerates. You get garbled bytes on every line and it looks like a wiring fault.
- [ ] **2 × 33 pF ceramic capacitors** — crystal load caps, one per crystal leg to GND.
- [ ] **1 × 10 µF electrolytic capacitor** — power-on reset.
- [ ] **1 × 10 kΩ resistor** — reset pull-down.
- [ ] **1 × 0.1 µF ceramic capacitor** — VCC decoupling, as close to pin 40 as you can get it.
- [ ] **1 × 330 Ω resistor** — the demo LED's current limiter. Sinks ~7.7 mA.
- [ ] **1 × LED** — any standard 5 mm indicator LED.
- [ ] **1 × 40-pin DIP socket** — so you are not pushing the chip in and out of a breadboard.
- [ ] **1 × solderless breadboard**
- [ ] **Jumper wires** — male-to-male, assorted.
- [ ] **1 × CP2102 USB-TTL adapter** — must expose 5V, GND, TXD and RXD pins.
- [ ] Confirm the adapter can supply **5 V**, not only 3.3 V. The STC89C52RC is a 5 V part.

---

## 5. Build and wire

- [ ] Clone/open the project and build the server: `cargo build --release`
- [ ] Confirm the binary exists: `ls -l target/release/mcs51-mcp`
- [ ] Wire the board exactly as shown in [`circuits.md`](circuits.md) — crystal + load caps on
      XTAL1/XTAL2 (pins 19/18), reset network on pin 9, decoupling on pins 40/20, LED on P1.0
      (pin 1).
- [ ] Wire the LED **active-low**: `+5V → 330 Ω → LED anode`, `LED cathode → P1.0 (pin 1)`.
      The pin **sinks** the current; it must never be asked to source it. An 8051 pin sources
      only ~60 µA.
- [ ] **RX/TX cross over.** Adapter **TXD → MCU P3.0 (RXD, pin 10)**, adapter **RXD → MCU P3.1
      (TXD, pin 11)**. Straight-through TX→TX is the single most common wiring mistake here and
      produces a board that flashes fine and then says nothing.
- [ ] Connect adapter **GND → MCU GND (pin 20)**. Shared ground is not optional.
- [ ] Connect adapter **5V → MCU VCC (pin 40)**.
- [ ] Tie **EA (pin 31) to VCC** so the chip runs from internal flash.
- [ ] Double-check pin 32 is **P0.7** and pin 39 is **P0.0** — Port 0's pin numbering runs
      *backwards* relative to the other ports. Easy to miscount.
- [ ] Power up and confirm nothing gets hot. If anything does, cut power immediately and recheck
      VCC/GND before doing anything else.

---

## 6. Register with your client

- [ ] Get the absolute path to your binary: `echo "$PWD/target/release/mcs51-mcp"`
- [ ] Open `~/Library/Application Support/Claude/claude_desktop_config.json`
      (create it if it does not exist).
- [ ] Add the server, using the **absolute path** — not a relative one, and not `npx`:

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

- [ ] `FIRMWARE_ROOT` is optional. If you set it, it must point at an existing directory — the
      server **fails at startup** rather than silently running unconfined.
- [ ] Restart Claude Desktop completely (quit, do not just close the window).
- [ ] **For Claude Code instead**, register from the CLI:

      ```bash
      claude mcp add mcs51 \
        --env FIRMWARE_ROOT=/Users/you/mcp8051/firmware \
        -- /Users/you/mcp8051/target/release/mcs51-mcp
      ```

- [ ] Confirm the client lists 12 tools from `mcs51`.

---

## 7. First run

- [ ] Ask the assistant to run **`doctor`**. Confirm it finds `sdcc` 4.6.0, `packihx`
      (present, no version — that is correct), and `stcgal` 1.10.
- [ ] Ask it to run **`list_serial_ports`**. Confirm your `/dev/cu.*` path is listed and ranked
      first.
- [ ] Ask it to **`compile`** the reference firmware. Confirm you get a `.hex` back.
- [ ] Ask it to **`flash`** to your port with `chip: "stc"`.
- [ ] **Power-cycle the board now** — cut power and reapply it *after* the flash command is
      issued. The STC89 only enters its serial bootloader on power-up, and `stcgal` is sitting
      there waiting for that handshake. Doing it in the other order times out.
- [ ] Confirm `stcgal` reports the image written.
- [ ] Ask it to **`serial_open`** your port at **9600** baud.
- [ ] Ask it to **`serial_write`** `PING` and **`serial_expect`** `PONG`.
- [ ] If you get `PONG`, the whole loop works. If you get nothing, check the RX/TX crossover
      first — it is almost always the crossover.
- [ ] Try `SET 1 0 0` to light the LED and `SET 1 0 1` to turn it off. Active-low: `0` is on.
- [ ] Ask it to **`serial_close`** the session when you are done, so the device is released.

---

## Optional

- [ ] Run the smoke harness: `python3 scripts/smoke.py`. It drives the real binary over stdio and
      asserts the 12-tool surface, the envelope contract, and the safety rules. Needs no hardware.
- [ ] Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt` before committing.
