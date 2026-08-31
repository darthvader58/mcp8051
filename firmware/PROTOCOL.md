# mcs51-mcp firmware serial protocol

Reference firmware for an 8052-core board (STC89C52RC, AT89S52, …) that exposes the
chip's four I/O ports over a serial line. It is what `mcs51-mcp`'s `serial_*` tools talk
to, and it is small enough to read end to end — see [`firmware.c`](firmware.c).

The board never initiates anything. It reads a line, it sends a line back. That is the
whole design.

---

## 1. Link settings

| Setting | Value |
|---|---|
| Baud | 9600 |
| Data bits | 8 |
| Parity | none |
| Stop bits | 1 |
| Flow control | none |
| Crystal | **11.0592 MHz** |
| Timer | Timer 1, mode 2 (8-bit auto-reload), `TH1 = TL1 = 0xFD`, `TR1 = 1` |
| Serial mode | `SCON = 0x50` — mode 1 (8-bit UART, timer baud), `REN = 1` |
| `SMOD` | 0 (no baud doubling; `PCON` bit 7 cleared) |

### Why the crystal is not negotiable

Baud rate on an 8051 comes from Timer 1 overflowing:

```
baud = crystal / (12 × 32 × (256 − TH1))          with SMOD = 0
```

The 12 is the machine cycle (12 oscillator periods per cycle), the 32 is the UART's
divide-by-16 doubled because `SMOD` is 0. With `TH1 = 0xFD` the reload count is
`256 − 253 = 3`:

```
11 059 200 / (12 × 32 × 3) = 11 059 200 / 1152 = 9600.000   exactly, 0.00% error
```

That exactness is the entire reason 8051 boards ship with the odd-looking 11.0592 MHz
crystal instead of a round number. Swap in a "nicer" 12 MHz part and the same `TH1`
gives `12 000 000 / 1152 = 10416.7` baud — **8.5% off**. A UART tolerates roughly 2%
before the receiver samples in the wrong bit cell, so every byte comes back as garbage.
For most common crystals there is no integer `TH1` that lands on 9600 at all.

If you must change the crystal, recompute `TH1` from the formula above and check the
error is under about 2% — do not just hope.

---

## 2. Wire format

- **Requests**: ASCII, terminated by CR (`\r`), LF (`\n`), or CRLF. All three work.
- **Replies**: ASCII, always terminated by **CRLF** (`\r\n`), so a dumb terminal renders
  them on their own lines. A host parser should just trim the line ending.
- **Exactly one reply line per command line.** No banners, no echo, no unsolicited
  output — every byte the host receives is an answer to something it sent.
- Fields are separated by one or more spaces or tabs. Leading and trailing whitespace is
  ignored, so `"  GET 1 0  "` parses identically to `"GET 1 0"`.
- **Command keywords are case-insensitive** (`set`, `SET` and `Set` all work) — this is a
  convenience for typing at a terminal. Hex *input* also accepts either case. Everything
  the board **sends** is uppercase.
- A **blank line produces no reply at all** (see [§6](#6-error-cases-and-edge-cases)).
- The input line buffer is 32 bytes. A longer line cannot overflow it; it gets one `ERR`.

Parameter ranges:

| Symbol | Meaning | Range | Format |
|---|---|---|---|
| `p` | port number (P0–P3) | 0–3 | exactly one decimal digit |
| `b` | bit number within the port | 0–7 | exactly one decimal digit |
| `v` | logic level | 0 or 1 | exactly one decimal digit |
| `hh` | whole-port byte | 00–FF | exactly two hex digits |

"Exactly one digit" is literal: `SET 1 00 1` is `ERR`, and so is `SET 01 0 1`. Likewise
`hh` must be two hex digits — `WRP 1 F` and `WRP 1 0FF` are both `ERR`.

---

## 3. Command reference

| Request | Reply | Meaning |
|---|---|---|
| `PING` | `PONG` | Liveness check |
| `SET p b v` | `OK` | Drive bit `b` of port `p` to `v` |
| `GET p b` | `0` or `1` | Sample bit `b` of port `p` (reads the **pin**) |
| `WRP p hh` | `OK` | Drive all 8 bits of port `p` |
| `RDP p` | `hh` | Sample all 8 bits of port `p` (reads the **pins**) |
| anything else | `ERR` | Unknown command, bad arguments, or a refused write |

`→` is host-to-board, `←` is board-to-host, in every example below.

### `PING` → `PONG`

Liveness and baud-rate sanity check. If you get `PONG` back cleanly, the crystal, the
divisor and the wiring are all correct. Garbage here almost always means a wrong baud
rate or the wrong crystal.

```
→ PING
← PONG
```

### `SET p b v` → `OK`

Drives one bit of one port. The other seven bits keep whatever they were last driven to
(see [§4](#4-shadow-latches)).

Light the demo LED on P1.0, then turn it off:

```
→ SET 1 0 0
← OK
→ SET 1 0 1
← OK
```

The demo LED is **active-low**: `+5V → 330 Ω → LED → P1.0 (pin 1)`. The pin **sinks** the
current, so **`SET 1 0 0` turns it ON** and `SET 1 0 1` turns it off. This is not a
stylistic choice — an 8051 port pin sinks well (`V_OL` is specified at 1.6 mA, 10 mA
absolute per pin) but barely sources at all (`V_OH` is specified at **−60 µA**). The
obvious-looking `P1.0 → 220 Ω → LED → GND` wiring would need the pin to source ~10 mA
when it can deliver 60 µA, and the LED would be invisibly dim.

Rejected because it would cut the serial link — see [§5](#5-the-p30--p31-guard):

```
→ SET 3 0 0
← ERR
```

### `GET p b` → `0` | `1`

Samples one bit. This reads the **physical pin**, not the shadow latch — that is the
whole point of an input.

Read a button on P3.2 (with the usual `button to GND` wiring plus the port's internal
pull-up, so idle reads 1 and pressed reads 0):

```
→ GET 3 2
← 1
→ GET 3 2
← 0
```

Reads are never refused. `GET 3 0` and `GET 3 1` are legal — sampling RXD/TXD is
harmless, it just tells you the line's instantaneous level.

**To use a pin as an input its latch must hold 1.** A latch holding 0 actively drives the
pin low and it will read 0 forever. Reset leaves every latch at 1, so pins are
input-ready out of the box; if you have driven a pin low, release it with `SET p b 1`
first. The firmware deliberately does **not** do this for you — a read that silently
changes an output would be a nasty surprise.

### `WRP p hh` → `OK`

Drives all eight bits of a port at once, and replaces the shadow for that port wholesale.

```
→ WRP 1 FE
← OK
```

`0xFE` is `11111110`: P1.0 low (demo LED on), P1.1–P1.7 high. Lower-case hex is accepted
on input:

```
→ WRP 2 0f
← OK
```

`WRP 3 hh` is **always** `ERR` — see [§5](#5-the-p30--p31-guard).

### `RDP p` → `hh`

Samples all eight pins of a port, returned as **two uppercase hex digits**, always padded
to two:

```
→ RDP 1
← FE
→ RDP 0
← 00
```

Reading `00` from P0 with nothing attached is normal: **Port 0 is open-drain with no
internal pull-up**, so it floats unless you fit external pull-ups (10 kΩ typical).
P1/P2/P3 have weak internal pull-ups and read `FF` when unconnected.

---

## 4. Shadow latches

Every 8051 port pin is a latch driving a weak pull-up and a strong pull-down. The SFR
name is overloaded: **reading `P1` reads the pins, assigning to `P1` writes the latches.**
Those are not the same value.

If anything external holds a pin low — a closed switch, another driver, an LED — then
reading that port returns 0 in that bit position even though the latch holds 1. So the
tempting one-liner:

```c
P1 = P1 | 0x01;      /* WRONG */
```

reads the pins, copies every externally-forced 0 back into the latch, and permanently
clamps those pins low. The bug is invisible until you wire up an input, and then it looks
like the input "stopped working".

This firmware keeps its own record of what it last **drove**:

```c
static unsigned char shadow[4] = { 0xFF, 0xFF, 0xFF, 0xFF };
```

`0xFF` is the 8051's port state after reset — all latches high, every pin free to be
pulled down from outside — and `main()` drives that same value out once at boot so
`shadow == latch` holds from the first instruction.

The rules that follow from this:

- **`SET` starts from the shadow**, sets or clears one bit, and writes the result. It
  never re-reads the pins.
- **`WRP` overwrites the shadow** for that port with the byte you gave it.
- **`GET` and `RDP` read the pins**, never the shadow. They do not touch it.
- The shadow is per-port and lives only in RAM: it is `{FF,FF,FF,FF}` again after any
  reset or power cycle, which matches what the hardware does.

A worked consequence — driving one bit low does not disturb an input on the same port:

```
→ SET 1 0 0        drive P1.0 low (LED on); shadow[1] = 0xFE
← OK
→ GET 1 7          P1.7 is wired to a switch that is currently closed
← 0
→ SET 1 1 0        drive P1.1 low too
← OK
→ RDP 1            P1.7 still reads 0 because the switch holds it, not because
← 7C               we latched it low; shadow[1] is 0xFC, the pins read 0x7C
```

Without the shadow, the `SET 1 1 0` in the middle would have latched P1.7 low and the
switch would have been stuck reading 0 for good.

---

## 5. The P3.0 / P3.1 guard

**P3.0 is RXD and P3.1 is TXD.** They are the only link the host has to the board.

Driving either as a plain GPIO cuts the wire the conversation is happening over. A stray
`SET 3 0 0` would strand the session with no way back except a power cycle — and no error
message, because the error message cannot get out either.

So: **any write that would touch P3.0 or P3.1 replies `ERR` and changes nothing.**

```
→ SET 3 0 0
← ERR
→ SET 3 1 1
← ERR
→ WRP 3 FF
← ERR
```

Three details worth being explicit about:

1. **The guard is checked before anything is modified.** An `ERR` from the guard means the
   shadow and the port are exactly as they were.
2. **`WRP 3` is rejected outright**, not mask-preserved. `WRP` writes all eight bits at
   once, so it must either refuse or quietly write something different from what was
   asked. The refusal is the better failure: answering `OK` to a partially-applied write
   would leave the host believing all 8 bits landed. **P3.2–P3.7 remain fully writable
   one bit at a time** with `SET 3 b v`, which is the intended way to use the rest of
   port 3.
3. **Reads are never guarded.** `GET 3 0`, `GET 3 1` and `RDP 3` all work — sampling a pin
   changes nothing.

The guard is not in the abstract command table, but it is consistent with it: a refused
write is an invalid request, and invalid requests answer `ERR`.

---

## 6. Error cases and edge cases

Everything the board does not recognise answers `ERR`. Specifically:

| Input | Reply | Why |
|---|---|---|
| `HELLO` | `ERR` | Unknown keyword |
| `SET` | `ERR` | Wrong argument count |
| `SET 1 0` | `ERR` | Wrong argument count |
| `SET 1 0 1 1` | `ERR` | Too many tokens |
| `PING PING` | `ERR` | `PING` takes no arguments |
| `SET 4 0 1` | `ERR` | Port out of range (0–3) |
| `SET 1 8 1` | `ERR` | Bit out of range (0–7) |
| `SET 1 0 2` | `ERR` | Value must be 0 or 1 |
| `SET 1 0 x` | `ERR` | Not a decimal digit |
| `SET 01 0 1` | `ERR` | `p` must be exactly one digit |
| `GET 9 0` | `ERR` | Port out of range |
| `WRP 1 G0` | `ERR` | Not hex |
| `WRP 1 F` | `ERR` | `hh` must be exactly two hex digits |
| `RDP 4` | `ERR` | Port out of range |
| `SET 3 0 v`, `SET 3 1 v`, `WRP 3 hh` | `ERR` | RXD/TXD guard ([§5](#5-the-p30--p31-guard)) |
| a line longer than 31 characters | `ERR` (one, not many) | Buffer bound |

Out-of-range values are **rejected**, never wrapped, masked or shifted into range. There
is no input that produces undefined behaviour.

Two cases that are deliberately *not* `ERR`:

- **A blank line gets no reply at all.** This is what makes CRLF work. The firmware
  accepts CR or LF as end-of-line, so a terminal sending `\r\n` delivers one command
  followed by one empty line. If empty lines answered `ERR`, every Enter keypress from a
  terminal — and every CRLF from a host library — would emit a spurious `ERR` and destroy
  the one-reply-per-command pairing that host-side parsing depends on. Silence keeps the
  pairing exact for LF, CR and CRLF alike.
- **An over-long line produces exactly one `ERR`**, not one per excess character. The
  firmware keeps consuming to the end of the line before answering, so a runaway host
  cannot trigger a cascade.

---

## 7. Build and flash

Build, from this directory:

```sh
sdcc -mmcs51 firmware.c && packihx firmware.ihx > firmware.hex
```

`sdcc` produces `firmware.ihx` (plus `.asm`, `.lst`, `.map`, `.mem`, `.rel`, `.rst`,
`.sym`); `packihx` repacks it into the denser, more widely accepted `firmware.hex`.

Current build: **1883 bytes of code** (`0x0000`–`0x075A`), 23% of the STC89C52's 8 KB
flash. No external RAM, no paged RAM; internal RAM is 8 bytes of register bank, 64 bytes
of data, 2 bytes of overlay, and the rest is stack (182 bytes free from `0x4A`).

> `packihx` **always exits 0, even on failure** — it has no `--version` and no `-h`, and
> a bare invocation reads stdin. Never trust its exit code. Validate the output file
> instead. This is exactly what `compile` does: non-empty, starts with `:`, and contains
> the Intel-HEX EOF record `:00000001FF`.

Flash it to an STC89C52 over the same USB-TTL adapter you talk to it with — this is the
same argv `flash(chip="stc")` spawns:

```sh
stcgal -p /dev/cu.usbserial-XXXX firmware.hex
```

(`-P stc89` skips stcgal's model autodetect if you want it; the server does not pass it.)

`stcgal` waits for a power cycle — start the command, then power-cycle the board.

---

## 8. Talking to it by hand

Find the adapter (prefer `/dev/cu.*` — `/dev/tty.*` is the dial-in device and blocks
waiting for carrier detect):

```sh
ls /dev/cu.usbserial-* /dev/cu.wchusbserial-*
```

Open a terminal at 9600 baud:

```sh
screen /dev/cu.usbserial-XXXX 9600
```

You will see nothing at first — the firmware has no boot banner and says nothing until
spoken to. Type a command and press Enter:

```
PING
```

and the board answers `PONG`. `screen` sends CR on Enter, which the firmware accepts, so
this just works. Note that `screen` does not echo what you type unless local echo is on,
so you will see your keystrokes only if the far end echoes them — this firmware does not.
Type carefully, or use `screen -L` and read the log.

A short session to try, with the demo LED wired `+5V → 330 Ω → LED → P1.0`:

```
PING            → PONG
SET 1 0 0       → OK     LED ON  (P1.0 pulled low, sinking ~7.7 mA)
RDP 1           → FE     P1.0 low, the rest high
SET 1 0 1       → OK     LED OFF
RDP 1           → FF
SET 3 0 0       → ERR    refused — that is the RXD pin you are talking over
GET 3 2         → 1      sample the INT0 pin
WRP 2 00        → OK     drive all of P2 low
RDP 2           → 00
WRP 2 FF        → OK     release P2 again
```

To leave `screen`: `Ctrl-A` then `K`, then `y`. (If you close the terminal window instead,
the lock file may linger — `screen -wipe` clears it.)

Same thing from a script, without a terminal emulator:

```sh
stty -f /dev/cu.usbserial-XXXX 9600 cs8 -cstopb -parenb -crtscts raw
printf 'PING\r\n' > /dev/cu.usbserial-XXXX
head -c 6 < /dev/cu.usbserial-XXXX          # PONG\r\n
```

---

## 9. Quick reference card

```
PING          → PONG                 liveness
SET p b v     → OK   | ERR           p 0-3, b 0-7, v 0/1     writes latch via shadow
GET p b       → 0|1  | ERR           reads PIN
WRP p hh      → OK   | ERR           hh = 2 hex digits       replaces shadow
RDP p         → hh   | ERR           reads PINS, uppercase hex
<blank>       → (silence)
<anything>    → ERR

Refused writes: SET 3 0 v | SET 3 1 v | WRP 3 hh      (RXD/TXD)
Demo LED:       +5V → 330Ω → LED → P1.0     active-low: SET 1 0 0 = ON
Link:           9600 8N1, 11.0592 MHz, TH1=TL1=0xFD, SCON=0x50, SMOD=0
Build:          sdcc -mmcs51 firmware.c && packihx firmware.ihx > firmware.hex
```
