# circuits.md — reference hardware for `mcs51-mcp`

The minimum board the server expects on the other end of the serial port: an 8051 running
`firmware/firmware.c`, an 11.0592 MHz crystal, a power-on reset, one USB-TTL adapter, and one
demo LED on P1.0.

Every number below is either lifted from the AT89S52 / STC89C52 datasheet or derived from a
calculation shown inline. Where this design deviates from the naive/intuitive one, the reason is
stated on the same line. See [Provenance](#provenance) at the bottom for which figures come from
where.

**Target:** STC89C52RC in DIP-40 (default), pin-for-pin compatible with the AT89S52 DIP-40.
One pinout table serves both.

---

## 1. Bill of materials

Ordering table. Values are exact; substitute only where a tolerance is given.

| Ref | Part | Value / spec | Qty | Why this part |
|---|---|---|---|---|
| U1 | **MCU — STC89C52RC** (preferred) | `STC89C52RC-40I-PDIP40`, DIP-40, 600 mil, 5 V part | 1 | Has a **serial bootloader in mask ROM**, so the same USB-TTL adapter both flashes it and talks to it. No programmer hardware, and it is the only clean macOS path (`stcgal`). |
| U1 alt | MCU — AT89S52 | `AT89S52-24PU`, DIP-40, 600 mil | 1 | Drop-in pin-compatible, but has **no serial bootloader** — it needs SPI ISP hardware on P1.5/P1.6/P1.7. Choose only if you already own a programmer and a non-macOS host. |
| Y1 | Crystal | **11.0592 MHz**, HC-49S (or HC-49/US) case, fundamental, parallel-resonant, CL ≈ 20 pF, ±30 ppm | 1 | Divides *exactly* to 9600 baud — see §6. A "rounder" 12 MHz crystal cannot (§11). |
| C1, C2 | Crystal load caps | **33 pF** ceramic, C0G/NP0, 50 V | 2 | Datasheet specifies C1 = C2 = 30 pF ±10 pF for a crystal; 33 pF is the standard E12 value inside that window. C0G because its capacitance must not drift with temperature or bias. |
| C3 | Decoupling cap | **0.1 µF** ceramic, X7R, 50 V | 1 | Local charge reservoir across pins 40/20 for the port drivers' switching transients. Must sit *at the pins*, not at the rail end of the breadboard. |
| C4 | Reset cap | **10 µF** electrolytic, 16 V or 25 V, radial, **polarised** | 1 | With R1 it forms the power-on reset pulse. τ = 10 µF × 10 kΩ = **100 ms** (§5). `+` leg goes to **+5 V**, not to RST. |
| R1 | Reset pulldown | **10 kΩ**, 1/4 W, 5 % | 1 | Discharges C4 and holds RST at 0 V once the pulse has passed. Weak enough (0.5 mA at 5 V) that an ISP programmer can still drive RST high. |
| SW1 | Reset button *(optional)* | 6 mm momentary tactile, 4-pin THT, NO | 1 | Manual warm reset: +5 V → RST. Optional — the RC alone gives you a reset at every power-up. |
| R2 | LED series resistor | **330 Ω**, 1/4 W, 5 % | 1 | Sets LED current to ≈7.7 mA, under the 10 mA per-pin sink limit (§7, §8). Dissipates I²R = (7.7 mA)² × 330 Ω ≈ **20 mW**, so 1/4 W is 12× derated. |
| D1 | Demo LED | 5 mm through-hole, red, Vf ≈ **2.0 V**, 20 mA rated | 1 | Red because its ~2.0 V forward drop leaves the most headroom in the 5 V loop. A blue/white LED (Vf ≈ 3.2 V) would need R2 recomputed. |
| — | DIP socket | **40-pin DIP, 600 mil (0.6 in) row spacing**, dual-wipe or machined | 1 | The MCU's legs survive maybe a dozen breadboard insertions; the socket's don't matter. Also lets you pull the chip to recover a bricked ISP config. |
| — | Breadboard | Full-size, 830 tie-point, with power rails | 1 | The DIP-40 is 2 in long and 600 mil wide — it straddles the centre channel and eats most of a half-size board before the crystal and adapter go in. Slightly off-centre placement is normal (the channel is 0.3 in, the part is 0.6 in). |
| — | Jumper wires | Male–male, 22 AWG solid or pre-formed | ~20 | Solid core only. Stranded "Dupont" wire works loose in breadboard clips and gives intermittent serial. |
| — | **USB-TTL adapter** | **CP2102** module, with a 5 V / 3.3 V VCCIO jumper, exposing 5V / GND / TXD / RXD | 1 | CP2102 works **driverless on Apple Silicon** and enumerates as `/dev/cu.usbserial-XXXX`. A CH340 board (`/dev/cu.wchusbserial-XXXX`) needs the WCH DriverKit driver installed first. |
| RN1 | Port 0 pull-ups — **only if you use Port 0** | 10 kΩ × 8 **bussed** SIP resistor network, 9-pin (marked `9A103J` / `A103J`) | 1 | Port 0 is open-drain with **no internal pull-up** (§9). Without these, P0 pins read and drive nothing but a float. |
| RN1 alt | Port 0 pull-ups, discrete | 10 kΩ, 1/4 W, 5 % | 8 | Same job, 8 parts instead of 1. Use if you don't have the network. |

### Optional, strongly recommended

| Ref | Part | Value / spec | Qty | Why |
|---|---|---|---|---|
| C5 | Bulk cap | 10–100 µF electrolytic, 16 V | 1 | Across the breadboard's +5 V/GND rails at the entry point. Breadboard rails have real inductance; C3 handles nanoseconds, C5 handles milliseconds. |
| SW2 | Power switch / jumper | SPDT slide switch, or just a removable jumper wire in the +5 V feed | 1 | **You will power-cycle this board on every STC flash** (§10). Reaching for the USB plug 40 times gets old and wears the connector. |
| R3 | TXD level resistor | 1 kΩ, 1/4 W | 1 | Only if your USB-TTL is 3.3 V-only with no VCCIO jumper. In series with 8051 pin 11 → adapter RXD (§6). |

---

## 2. Full pin-by-pin connection table (DIP-40)

All 40 pins, in pin order. "no connection (free I/O)" means exactly that — the pin is unused in this
reference design and is available to your firmware.

| Pin | Name | Alt function | Connect to | Notes |
|---|---|---|---|---|
| 1 | P1.0 | T2 (Timer 2 ext. clock) | **D1 cathode** (D1 anode → R2 330 Ω → +5 V) | Demo LED. **ACTIVE-LOW** — see §7. `SET 1 0 0` = ON. |
| 2 | P1.1 | T2EX (Timer 2 capture/reload) | no connection (free I/O) | Weak internal pull-up, reads high at reset. |
| 3 | P1.2 | — | no connection (free I/O) | |
| 4 | P1.3 | — | no connection (free I/O) | |
| 5 | P1.4 | — | no connection (free I/O) | |
| 6 | P1.5 | **MOSI** (AT89S ISP) | no connection (free I/O) | Keep loads off it if you ever want AT89S SPI ISP. |
| 7 | P1.6 | **MISO** (AT89S ISP) | no connection (free I/O) | Same. |
| 8 | P1.7 | **SCK** (AT89S ISP) | no connection (free I/O) | Same. |
| 9 | RST | — | Reset RC node: C4 10 µF to +5 V, R1 10 kΩ to GND, optional SW1 to +5 V | **ACTIVE-HIGH** reset (§5). Do not tie low "to be safe" — it must idle at 0 V. |
| 10 | P3.0 | **RXD** | USB-TTL **TXD** | Crossover — §6. Firmware refuses `SET 3 0 v` to protect this line. |
| 11 | P3.1 | **TXD** | USB-TTL **RXD** | Crossover — §6. Firmware refuses `SET 3 1 v`. |
| 12 | P3.2 | INT0 (external interrupt 0) | no connection (free I/O) | |
| 13 | P3.3 | INT1 (external interrupt 1) | no connection (free I/O) | |
| 14 | P3.4 | T0 (Timer 0 ext. input) | no connection (free I/O) | |
| 15 | P3.5 | T1 (Timer 1 ext. input) | no connection (free I/O) | Timer 1 itself is used internally as the baud generator; the *pin* is free. |
| 16 | P3.6 | WR (external data memory write strobe) | no connection (free I/O) | |
| 17 | P3.7 | RD (external data memory read strobe) | no connection (free I/O) | |
| 18 | XTAL2 | Oscillator amp output | Y1 leg B, **and** C2 33 pF → GND | Short leads. §4 diagram. |
| 19 | XTAL1 | Oscillator amp input / ext. clock in | Y1 leg A, **and** C1 33 pF → GND | Short leads. |
| 20 | GND (VSS) | — | **GND rail** | C3 0.1 µF from here to pin 40, physically adjacent. |
| 21 | P2.0 | A8 (ext. memory address) | no connection (free I/O) | |
| 22 | P2.1 | A9 | no connection (free I/O) | |
| 23 | P2.2 | A10 | no connection (free I/O) | |
| 24 | P2.3 | A11 | no connection (free I/O) | |
| 25 | P2.4 | A12 | no connection (free I/O) | |
| 26 | P2.5 | A13 | no connection (free I/O) | |
| 27 | P2.6 | A14 | no connection (free I/O) | |
| 28 | P2.7 | A15 | no connection (free I/O) | |
| 29 | PSEN | Program Store Enable (ext. code fetch strobe) | **no connection** | An *output*. Only meaningful with external program memory. Never drive it. |
| 30 | ALE / PROG | Address Latch Enable | **no connection** | An *output*, free-running at fosc/6 = 11.0592/6 = **1.8432 MHz**. Not usable as GPIO. |
| 31 | EA / VPP | External Access enable | **+5 V** | **Mandatory.** Low or floating = fetch code from external memory = chip appears dead. §3. |
| 32 | P0.7 | AD7 (ext. addr/data bus) | no connection (free I/O) | Open-drain: needs a 10 kΩ pull-up to +5 V before use. §9. |
| 33 | P0.6 | AD6 | no connection (free I/O) | Open-drain, needs pull-up. |
| 34 | P0.5 | AD5 | no connection (free I/O) | Open-drain, needs pull-up. |
| 35 | P0.4 | AD4 | no connection (free I/O) | Open-drain, needs pull-up. |
| 36 | P0.3 | AD3 | no connection (free I/O) | Open-drain, needs pull-up. |
| 37 | P0.2 | AD2 | no connection (free I/O) | Open-drain, needs pull-up. |
| 38 | P0.1 | AD1 | no connection (free I/O) | Open-drain, needs pull-up. |
| 39 | P0.0 | AD0 | no connection (free I/O) | Open-drain, needs pull-up. |
| 40 | VCC | — | **+5 V rail** | C3 0.1 µF from here to pin 20, right at the pins. |

> **Port 0 numbering descends.** Pin 32 = P0.**7**, pin 39 = P0.**0**. Every other port ascends with
> pin number. Miscounting here is the second most common wiring error after RX/TX.

---

## 3. Power

```
                 +5 V rail  ──┬──────┬─────────────────► reset RC, R2/LED,
                              │      │                    P0 pull-ups, USB-TTL 5V
                              │      │
                        ┌─────┴──┐   │
                        │ pin 40 │   └──────────────┐
                        │  VCC   │                  │
                        └────┬───┘             ┌────┴─────┐
                             │                 │  pin 31  │
                            ═╪═  C3  0.1 µF    │  EA/VPP  │   tied HIGH
                             │   ceramic       └──────────┘
                        ┌────┴───┐
                        │ pin 20 │
                        │  GND   │
                        └────┬───┘
                             │
                 GND rail  ──┴──────────────────────────► common with USB-TTL GND
```

- **C3 goes at the chip, not at the rail.** The whole point is to supply the port drivers' switching
  current from ~10 mm away instead of through 15 cm of breadboard strip. Bend the leads and tuck it
  under the socket between pins 20 and 40 if you can.
- Budget roughly **25–30 mA** for the MCU itself in active mode, plus 7.7 mA for the LED. A USB port
  sourcing 500 mA through the adapter is not remotely stressed.
- Power the board from **one** source. If the adapter's 5V pin feeds the rail, do not also connect a
  bench supply — you'll be back-driving one regulator from the other.

### EA (pin 31) must be tied to +5 V

`EA` = **E**xternal **A**ccess. It is a *static level input read at reset*, not a strap you can leave
to chance:

| EA level | Where the CPU fetches code from |
|---|---|
| **HIGH (+5 V)** | On-chip flash (8 KB on both the STC89C52RC and the AT89S52). **This is what you want.** |
| LOW (GND) | External program memory over the P0/P2 bus, exclusively — internal flash is ignored entirely. |
| **Floating** | Undefined. It will latch one way or the other at power-up, unpredictably. |

There is no external memory on this board, so with EA low or floating the CPU fetches garbage off a
bus that isn't connected, executes it, and the board looks **completely dead** — no LED, no `PONG`,
no oscilloscope activity on P3.1. It's a silent failure with no diagnostic, which is why it earns its
own line in §11.

Tie pin 31 **directly to +5 V** with a wire. A pull-up resistor also works but adds nothing here.

> **Note for later:** pin 31 is also `VPP`, the +12 V programming input for *parallel* programmers.
> Nothing in this design applies 12 V, but if you ever put this chip in a parallel programmer, that
> wire must come off first.

---

## 4. Clock

```
                    ┌────────────────────────┐
     pin 19 ────────┤   Y1                   ├──────── pin 18
     XTAL1          │   11.0592 MHz  HC-49S  │         XTAL2
       │            └────────────────────────┘           │
       │                                                 │
      ═╪═ C1                                        C2  ═╪═
       │  33 pF                                  33 pF   │
       │                                                 │
      GND ───────────────────────────────────────────── GND
```

- The crystal is **not polarised** — either leg to either pin.
- **Keep the leads short.** This is a ~1 MHz-bandwidth analogue amplifier hanging off two pins;
  long breadboard jumpers add stray capacitance and pick up switching noise from the port drivers.
  Target under 15 mm from crystal body to pin, and put C1/C2's ground legs into the *nearest*
  ground holes, not the far rail.
- Both caps return to **GND**, not to each other.
- Derived timings at 11.0592 MHz:
  - Machine cycle = 12 oscillator periods = 12 / 11.0592 MHz = **1.085 µs**
  - ALE output (pin 30) = fosc / 6 = **1.8432 MHz**
  - UART at 9600 baud, exactly — see §6.

### Why 11.0592 MHz and not a round number

It is 9600 × 1152 = 9600 × (12 × 32 × 3). The 8051's baud generator divides the oscillator by 12
(machine cycle) × 32 (fixed UART prescaler with SMOD = 0) × the Timer 1 reload divisor. Because
1152 factors cleanly, the reload divisor is the integer **3** and the baud rate error is
**exactly zero**. Every other "nicer" crystal leaves a fractional divisor. See §11 for what 12 MHz
actually costs you.

---

## 5. Reset

```
      +5 V ───┬────────────────┬
              │                │
              │  C4            │      ┌────────┐
             ═╪═ 10 µF         └──────┤  SW1   ├────┐   (optional, momentary)
              │  electrolytic         └────────┘    │
              │  + leg to +5 V                      │
              │                                     │
              ├─────────────────────────────────────┴──────────► pin 9  RST
              │
             ┌┴┐
             │ │ R1  10 kΩ
             └┬┘
              │
             GND
```

### This is an ACTIVE-HIGH reset

Unusual, and the single most counter-intuitive thing on the chip if you come from AVR, STM32, PIC,
ESP32 or basically anything made after 1985 — all of which use an active-**low** `/RESET` or `NRST`.

On the 8051:

- **RST idles at 0 V.** That is the *run* state.
- **Driving RST to +5 V resets the chip**, and it stays in reset for as long as the pin is high.
- Therefore: a pull**down** (R1), not a pull-up. And the button connects RST to **+5 V**, not to GND.

Wiring it the "normal" way — pull-up to +5 V, button to GND — holds the chip in permanent reset. It
looks identical to the EA failure: dead board, no output, no clue.

### How the RC gives you a power-on pulse

At the instant power is applied, an uncharged C4 is a short circuit, so RST is yanked to +5 V — the
chip enters reset. C4 then charges through R1, and the RST voltage decays:

```
  V_RST(t) = 5 V × e^(−t / τ)      where  τ = R1 × C4 = 10 kΩ × 10 µF = 100 ms
```

- The 8051 needs RST held high for **≥ 2 machine cycles** after the oscillator stabilises:
  2 × 1.085 µs = **2.17 µs**.
- RST is a Schmitt-trigger input, so the exact release point sits between the datasheet's `VIH1`
  (0.7·VCC = 3.5 V) and `VIL` (0.2·VCC − 0.1 = 0.9 V). That brackets the pulse width:

  ```
    release at 3.5 V:   t = τ · ln(5 / 3.5) = 0.1 s × 0.357 =  ≈36 ms   (shortest case)
    release at 0.9 V:   t = τ · ln(5 / 0.9) = 0.1 s × 1.715 = ≈171 ms   (longest case)
  ```

Either end of that bracket is 16 000×–79 000× the 2.17 µs minimum — deliberately generous, so the
crystal has time to start (tens of ms on a breadboard) before the CPU is released. Peak current
through R1 at t = 0 is 5 V / 10 kΩ = 0.5 mA.

**Consequence for flashing:** because τ = 100 ms, a *fast* power cycle can leave C4 partly charged
and produce a short or absent reset pulse. When power-cycling for `stcgal` (§10), leave the board off
for about **one second** (10 τ = full discharge).

> Two optional hardening tweaks, neither required on a breadboard:
> a **1N4148 across C4** (cathode to +5 V) discharges the cap instantly at power-down, making fast
> power cycles reliable; and a **100 Ω in series with SW1** stops the button contacts from taking the
> full capacitor dump every press.

---

## 6. Serial link (UART ↔ USB-TTL)

9600 baud, 8 data bits, no parity, 1 stop bit, **no flow control**. Four wires.

```
   USB-TTL adapter (CP2102)                          8051 DIP-40
   ────────────────────────                          ───────────
   TXD  ─────────────────────────────────────────►   pin 10   P3.0 / RXD
                                                     (adapter transmits, MCU listens)

   RXD  ◄─────────────────────────────────────────   pin 11   P3.1 / TXD
                                                     (MCU transmits, adapter listens)

   GND  ─────────────────────────────────────────    pin 20   GND
                                                     (mandatory common return)

   5V   ─────────────────────────────────────────    pin 40   VCC
                                                     (optional: power the board from USB)
```

> ## ⚠ RX AND TX CROSS OVER
>
> **Adapter TXD → MCU pin 10 (RXD). MCU pin 11 (TXD) → adapter RXD.**
>
> **TX to RX. RX to TX. Never TX to TX.**
>
> This is the single most common wiring mistake on this board. It is not a matter of convention or
> "trying the other way if it doesn't work" — a transmitter is a driven output and a receiver is a
> high-impedance input. Wire TX to TX and you have two outputs fighting each other on one node while
> both receivers sit unconnected, listening to nothing. Nothing is damaged; nothing works. `PING`
> goes out and no `PONG` ever comes back, and there is no error message anywhere to tell you why.
>
> The labels on both ends are named **from the point of view of the device they are printed on**.
> The adapter's "TXD" is the adapter's transmitter. The MCU's pin 11 "TXD" is the MCU's transmitter.
> Two transmitters must never meet.

### GND must be common

The 5 V TTL signals are voltages *relative to ground*. If the adapter's GND is not tied to pin 20,
the two sides have no shared reference and the receiver sees an undefined, floating, drifting level.
Symptom: nothing works, or it works intermittently and produces framing errors and garbage bytes.

This bites hardest when the board is powered from a separate supply — then it is easy to forget the
GND jumper because the adapter "only needs two wires for data." It needs three.

### Adapter logic level

The adapter must present **5 V logic**, or be 5 V-tolerant:

- **If your CP2102 board has a 3.3 V / 5 V jumper (most do): set it to 5 V.** Done.
- **MCU → adapter (pin 11 → RXD)** is the risky direction: the 8051 drives its TXD toward 5 V and a
  3.3 V-only receiver has to swallow it. In steady state the 8051's high side is only a weak pull-up
  (§8), so very little current reaches the receiver's clamp diode, and CP2102 inputs are generally
  5 V-tolerant. **But** the quasi-bidirectional port fires a *strong* pull-up for two oscillator
  periods on every 0→1 transition — a brief milliamp-scale push that the weak-pull-up argument does
  not cover. If your board has no VCCIO jumper, put **R3 (1 kΩ) in series** in this line. It bounds
  that transient to well under a milliamp, costs nothing, and does not affect a 9600-baud edge.
- **Adapter → MCU (TXD → pin 10)** is safe even from a 3.3 V adapter: the 8051's `VIH` is
  0.2·VCC + 0.9 = **1.9 V** at VCC = 5 V, and 3.3 V clears that with 1.4 V of margin.

### Baud rate derivation

```
  baud = fosc / (12 × 32 × (256 − TH1))         with SMOD = 0

  11 059 200 / (12 × 32 × (256 − 0xFD))
  = 11 059 200 / (12 × 32 × 3)
  = 11 059 200 / 1152
  = 9600.000   baud exactly,  0 % error
```

Firmware sets `TH1 = TL1 = 0xFD`, Timer 1 in mode 2 (8-bit auto-reload), `SCON = 0x50`.

### macOS device node

The adapter appears **twice**: `/dev/cu.usbserial-XXXX` and `/dev/tty.usbserial-XXXX`.
**Always use `/dev/cu.*`.** The `tty.*` (dial-in) node blocks on open waiting for carrier detect,
which this adapter never asserts, so opening it hangs. `list_serial_ports` ranks `cu.*` first for
this reason.

---

## 7. Demo LED on P1.0 — and why it is ACTIVE-LOW

```
      +5 V
        │
       ┌┴┐
       │ │  R2  330 Ω
       └┬┘
        │
        ▼   anode   (long leg — the one toward the resistor)
       ───
        ┴   D1, red LED,  Vf ≈ 2.0 V
        │   cathode  (short leg / flat side of the rim)
        │
        └──────────────────────►  pin 1   P1.0
```

**Current path:** +5 V → R2 → LED → **into pin 1**, where the chip sinks it to ground internally when
P1.0 is driven **low**.

| Firmware command | P1.0 level | LED |
|---|---|---|
| `SET 1 0 0` | 0 (pin sinks current) | **ON** |
| `SET 1 0 1` | 1 (pin releases; only the weak pull-up remains) | **OFF** |

Yes — writing **0** turns it **on**. That inversion is not a quirk of the firmware; it falls straight
out of the silicon.

### Why the intuitive wiring does not work

The obvious circuit is `P1.0 → 220 Ω → LED → GND`: drive the pin high, current flows out of the pin,
LED lights. On almost every modern MCU that is correct. On the 8051 it produces a **dark LED**,
because the port pins are wildly asymmetric:

| Direction | Datasheet spec | Practical capability |
|---|---|---|
| **Sinking** (pin low, current flows *in*) | `VOL` = 0.45 V max at `IOL` = 1.6 mA; **10 mA max per pin** | Strong. Drives LEDs, transistors, relays via drivers. |
| **Sourcing** (pin high, current flows *out*) | `VOH` = 2.4 V min at **`IOH` = −60 µA** | Essentially nothing. |

**60 microamps.** The high side of an 8051 port is a weak pull-up resistor, not a driver. An LED
needs on the order of 5–10 mA to be visible — that is **83× to 167× the current the pin is specified
to source** (5 mA / 60 µA = 83; 10 mA / 60 µA = 167). Strictly, −60 µA is the *test condition* at
which the datasheet guarantees `VOH` ≥ 2.4 V, not a hard ceiling: the pin will pass somewhat more
than that, but only by collapsing its output voltage. Which is the same thing from the LED's point of
view. Ask the pin for 10 mA out and its output voltage falls toward ground; the LED sits at
maybe 0.5 V across it and stays dark. Nothing is damaged, nothing complains, and you spend an hour
checking your firmware.

So on this chip you hang the load between **+5 V and the pin**, and switch it by pulling the pin
**low**. The +5 V rail supplies the current; the pin only has to sink it, which is the thing it is
good at. Every load on an 8051 is wired this way. This is not a workaround — it is *the* 8051 idiom,
and it is why the firmware's `SET p b 0` is the "on" case for anything you attach.

### Current calculation

The loop is 5 V, minus the LED's forward drop, minus whatever the pin sits at when sinking:

```
        Vcc − Vf(LED) − VOL(pin)       5.0 V − 2.0 V − 0.45 V       2.55 V
  I  =  ────────────────────────  =    ──────────────────────  =   ────────  =  7.7 mA
                  R2                            330 Ω                330 Ω
```

**7.7 mA**, against a per-pin limit of **10 mA** — 23 % of headroom, and comfortably inside the
15 mA Port-1 group budget with a single LED (§8).

> **Honest caveat:** `VOL` = 0.45 V is the datasheet maximum *at `IOL` = 1.6 mA*. At 7.7 mA the real
> `VOL` will be somewhat higher (the datasheet notes that if `IOL` exceeds the test condition, `VOL`
> may exceed its specification). A higher `VOL` means *less* current, not more, so 7.7 mA is a
> conservative **upper bound** — the safe direction against the 10 mA limit. Expect roughly
> 6–7.7 mA in practice, and an LED marginally dimmer than the arithmetic suggests. If you want it
> brighter, do **not** drop below ~270 Ω without re-checking against the 10 mA per-pin ceiling.

R2 dissipation: (7.7 mA)² × 330 Ω = **20 mW**. A 1/4 W resistor is 12× derated.

---

## 8. Electrical limits

### The three-tier current budget

All three tiers apply **simultaneously**. Satisfying one does not excuse the others.

| Tier | Limit | What it constrains |
|---|---|---|
| **Per pin** | **10 mA** `IOL` | Sink current through any one port pin. |
| **Per port** | **26 mA** for Port 0<br>**15 mA** for Ports 1, 2, 3 | Sum of `IOL` across the eight pins of that one port. |
| **Whole chip** | **71 mA** | Sum of `IOL` across *all* output pins together. |

Worked consequences, so the numbers mean something:

- 8 pins × 10 mA = 80 mA would satisfy the per-pin tier and blow through **both** the port tier and
  the chip tier. The per-pin number is a ceiling, not an allowance you can claim eight times.
- At the §7 LED current of 7.7 mA: **one** LED on Port 1 is fine (7.7 of 15 mA). **Two** is already
  15.4 mA — over the Port-1 budget. Spread multiple LEDs across different ports, or raise the series
  resistors. Port 0 (26 mA) takes three at 23.1 mA.
- Chip-wide, 71 mA / 7.7 mA = **9 LEDs maximum**, and only if they are distributed to respect the
  per-port tiers.

### Absolute maximum ratings

| Parameter | Limit | Note |
|---|---|---|
| DC output current, any pin | **15.0 mA** | *Absolute maximum* — beyond the 10 mA design limit is already out of spec; this is where damage begins. |
| Voltage on any pin (`VIN`) | **−1.0 V to +7.0 V** | Applies to inputs and outputs alike. Nothing on this board approaches it, but it is what makes a 5 V-driven pin survivable and a 12 V one not. |
| Logic family | **5 V TTL** | `VIH` = 0.2·VCC + 0.9 = 1.9 V; `VIL` = 0.2·VCC − 0.1 = 0.9 V at VCC = 5 V. |

### The sink-vs-source asymmetry, stated once

Every 8051 port pin is a **quasi-bidirectional** structure: a real N-channel transistor pulling
*down*, and a weak resistive pull-up (or, on Port 0, nothing at all) pulling *up*.

```
                    +5 V
                      │
                    [weak pull-up]      ← ~60 µA of source capability
                      │                    (absent entirely on Port 0)
        pin ──────────┤
                      │
                    ─┤├─  strong N-FET  ← up to 10 mA of sink capability
                      │
                     GND
```

The datasheet says this in two lines that are easy to skim past:

- `VOL` is characterised at `IOL` = **1.6 mA** (and the pin is rated to sink up to **10 mA**).
- `VOH` is characterised at `IOH` = **−60 µA**.

That is a **~167:1 ratio** between what the pin can pull down and what it can push up. It drives
every design decision on this chip:

1. **Loads hang from +5 V and are switched by pulling the pin low.** LEDs, opto-couplers, transistor
   bases, buzzers — everything is active-low (§7).
2. **A pin at logic 1 is functionally an input.** Writing 1 doesn't "drive high" so much as *let go*,
   leaving the weak pull-up to define the level. This is exactly why the 8051 can read an input pin
   with no direction register: you write 1 first, then read. It is also why an external device can
   safely pull a "1" pin low.
3. **Reset leaves every port at 0xFF** (all pins released), which is why `firmware.c` initialises its
   shadow latches to `{0xFF, 0xFF, 0xFF, 0xFF}` — that is the state the hardware actually starts in.
4. **Port 0 has no pull-up at all** (see below), so on P0 a logic 1 is a genuine float, not a weak
   high.

---

## 9. Port characteristics

| Port | Pins | Pull-up | Alternate role | Usable as GPIO without extra parts? |
|---|---|---|---|---|
| **P0** | 39…32 (P0.0…P0.7) | **NONE — open-drain** | **AD0–AD7**, multiplexed low address + data bus for external memory | **No.** Needs 8 × 10 kΩ external pull-ups to +5 V. |
| **P1** | 1…8 (P1.0…P1.7) | Weak internal | P1.0 = T2, P1.1 = T2EX, **P1.5 = MOSI, P1.6 = MISO, P1.7 = SCK** (AT89S SPI ISP) | Yes. |
| **P2** | 21…28 (P2.0…P2.7) | Weak internal | **A8–A15**, high address byte for external memory | Yes. |
| **P3** | 10…17 (P3.0…P3.7) | Weak internal | RXD, TXD, INT0, INT1, T0, T1, WR, RD | Yes — but **P3.0/P3.1 are the serial link**, see below. |

### Port 0 is open-drain

In I/O mode Port 0 has **no internal pull-up whatsoever**. Writing a 1 to a P0 pin does not raise its
voltage — it simply turns off the N-FET and leaves the pin floating at whatever the outside world
decides. Consequences:

- Reading an unconnected P0 pin returns noise.
- An LED wired the "normal" way to a P0 pin will not light — there is nothing to source current.
- `RDP 0` will return arbitrary values.

**Fix: 10 kΩ from each P0 pin to +5 V** (RN1 in the BOM).

```
                       +5 V
                        │
        ┌───┬───┬───┬───┼───┬───┬───┬───┐
       ┌┴┐ ┌┴┐ ┌┴┐ ┌┴┐ ┌┴┐ ┌┴┐ ┌┴┐ ┌┴┐
       │ │ │ │ │ │ │ │ │ │ │ │ │ │ │ │    RN1: 8 × 10 kΩ
       └┬┘ └┬┘ └┬┘ └┬┘ └┬┘ └┬┘ └┬┘ └┬┘    (bussed SIP network, common pin to +5 V)
        │   │   │   │   │   │   │   │
      p39 p38 p37 p36 p35 p34 p33 p32
      P0.0 .1  .2  .3  .4  .5  .6  P0.7
```

Sizing: at 10 kΩ each pull-up sources 5 V / 10 kΩ = **0.5 mA** into a pin that is pulled low —
8 pins = **4 mA**, well inside Port 0's 26 mA budget. The rising edge is RC-limited: into ~20 pF of
stray capacitance the time constant is 10 kΩ × 20 pF = **200 ns** (so ~440 ns for a 10–90 % rise).
Irrelevant for LEDs and buttons; drop to 4.7 kΩ or 1 kΩ only if you need genuinely fast edges.

Note that even *with* pull-ups, Port 0's high-side drive is only 0.5 mA. Still not enough for an LED.
**Wire loads active-low on Port 0 too.**

### P2 and external memory

P2 emits A8–A15 whenever the CPU fetches from external program memory. Because **EA is tied high**
(§3) and there is no external memory on this board, that never happens and all 8 pins are yours.

### P3.0 / P3.1 are reserved by the firmware

They are RXD and TXD — the only channel `mcs51-mcp` has to the chip. `firmware.c` therefore
**rejects `SET 3 0 v` and `SET 3 1 v` with `ERR`**, because a stray write would strand the session
until a power cycle. `WRP 3 hh` is **rejected outright with `ERR`**, not mask-preserved: `WRP`
writes all eight bits at once, so answering `OK` to a partially-applied write would leave the host
believing all 8 bits landed. P3.2–P3.7 remain fully writable one bit at a time with `SET 3 b v`.

### P1.5 / P1.6 / P1.7 and AT89S ISP

On the AT89S52 these three pins are **MOSI, MISO and SCK** for SPI in-system programming. They are
ordinary I/O at runtime, but anything you hang on them (especially anything low-impedance) will fight
the programmer. The demo LED is deliberately on **P1.0**, clear of all three. On the STC89C52RC these
pins have no ISP role at all — it flashes over the UART.

---

## 10. Flashing

### STC89C52RC — over the same USB-TTL adapter

The STC part carries a serial bootloader in mask ROM. No programmer, no extra pins, no extra wiring:
the four wires from §6 are the whole story.

**The catch: the bootloader only runs immediately after a cold power-on reset.** A warm reset (the
RST button, or `stcgal` toggling anything) does *not* re-enter it — the chip goes straight to your
program. This is why `stcgal` stops and waits for you.

The sequence a human actually performs:

```
 1.  Board wired per §6.  Adapter plugged into the Mac.  Board POWERED.
 2.  Run the flash:
         stcgal -p /dev/cu.usbserial-XXXX firmware.hex
     (this is exactly the argv  flash(chip="stc")  spawns — directly, never via a shell.
      Add  -P stc89  if you want to skip stcgal's model autodetect; the server does not.)
 3.  stcgal prints:            Waiting for MCU, please cycle power:
     …and sits there polling the port. It will wait indefinitely.
 4.  ► CUT POWER TO THE BOARD.  Pull the +5 V jumper, or flip SW2.
     ► WAIT ABOUT ONE SECOND.   (≈10 τ, so C4 fully discharges — §5.)
     ► REAPPLY POWER.
 5.  stcgal catches the bootloader's handshake in that first few-ms window and prints
     the detected model, firmware version and clock. Then: erase → write → verify.
 6.  On success stcgal releases the chip, which boots into the new program.
 7.  serial_open the same port at 9600 8N1, send PING, expect PONG.
```

Practical notes:

- **Do not unplug the USB adapter to cut power** — that tears down the `/dev/cu.*` node that `stcgal`
  is holding open, and the flash aborts. Cut power to the *board*, leaving the adapter enumerated.
  This is precisely why SW2 (or at minimum a dedicated, easy-to-reach +5 V jumper wire) is worth the
  30 seconds it takes to add.
- If the adapter's 5V pin is what powers the board, then that jumper wire **is** your power switch —
  pull it at pin 40, not at the adapter.
- Miss the window and `stcgal` just keeps waiting. Cycle power again; there is no penalty.
- `stcgal` does have an autoreset mode that pulses DTR, but it needs extra hardware (a transistor in
  the power feed or on RST) that this reference design deliberately does not include. Manual is fine.

### AT89S52 — SPI ISP, and there is no clean macOS path

The AT89S52 has **no serial bootloader**. Flashing it requires a programmer that speaks its SPI ISP
protocol on:

| Signal | Pin | Name |
|---|---|---|
| MOSI | 6 | P1.5 |
| MISO | 7 | P1.6 |
| SCK | 8 | P1.7 |
| RST | 9 | must be held **HIGH** for the entire programming session |
| VCC / GND | 40 / 20 | |

Our R1 (10 kΩ) is weak enough that a programmer can drive RST high against it; the programmer just
has to charge C4 first, which costs a few hundred milliseconds.

**State plainly: there is no clean macOS path.** `avrdude` carries `at89s51`/`at89s52` part entries,
but only for a narrow set of programmer types, and it is widely reported not to work in practice; the
common tools (ProgISP, the USBasp 8051 variants) are Windows binaries. Nothing is packaged, current,
and maintained for macOS arm64.

Consequently `mcs51-mcp` ships **`flash(chip="at89s")` as a documented stub** — it returns a clear
error explaining the situation rather than pretending. If you must use an AT89S52, flash it from a
Windows or Linux machine with a USBasp, then move the chip back. **Prefer the STC89C52RC.**

---

## 11. Common mistakes

Work down this list before debugging firmware. Every one of these produces a *silent* failure — no
error, no smoke, just a board that does nothing.

- [ ] **RX and TX not crossed.** Adapter TXD → pin 10; pin 11 → adapter RXD. TX-to-TX is two outputs
      shouting at each other and two deaf receivers. **§6.**
- [ ] **EA (pin 31) left floating.** Must be hard-wired to +5 V. Floating or low = the CPU fetches
      code from external memory that isn't there = totally dead board. **§3.**
- [ ] **No common ground.** The adapter's GND must connect to pin 20 even when the board is
      separately powered. TTL levels are meaningless without a shared reference. **§6.**
- [ ] **LED wired active-high** (`pin → R → LED → GND`). The pin sources ~60 µA; the LED needs
      ~7.7 mA. It will not light. Wire `+5 V → 330 Ω → LED → pin`. **§7.**
- [ ] **Port 0 used without external pull-ups.** P0 is open-drain with no internal pull-up; a "1"
      is a float, not a high. 10 kΩ per pin to +5 V. **§9.**
- [ ] **Wrong crystal.** A 12 MHz crystal cannot produce 9600 baud:

      ```
        best available divisor:  12 000 000 / (12 × 32 × 9600) = 3.255  →  round to 3  (TH1 = 0xFD)
        actual baud:             12 000 000 / (12 × 32 × 3)    = 10 416.7
        error:                   (10 416.7 − 9600) / 9600      = +8.5 %
      ```

      UARTs tolerate roughly ±2–3 % before the sampling point walks off the end of the frame. At
      +8.5 % you get garbage characters, framing errors, and intermittent `ERR` replies that look
      like a firmware bug. **Use 11.0592 MHz. §4.**
- [ ] **Reset wired active-low** (pull-up to +5 V, button to GND). Holds the chip in permanent
      reset. RST needs a pull**down** and a button to **+5 V**. **§5.**
- [ ] **Decoupling cap at the rail instead of at the chip.** C3 must bridge pins 40 and 20 directly.
- [ ] **Opened `/dev/tty.usbserial-*` instead of `/dev/cu.usbserial-*`.** The `tty.*` node blocks on
      carrier detect and hangs forever. **§6.**
- [ ] **C4 fitted backwards.** It is polarised: `+` to +5 V, `−` to the RST node.
- [ ] **Port 0 pin numbering counted upward.** Pin 32 is P0.**7**, pin 39 is P0.**0**. **§2.**

---

## Provenance

| Figure | Source |
|---|---|
| Port 0 open-drain, no internal pull-up; P1/P2/P3 weak pull-ups | AT89S52 datasheet, extracted for this project |
| 10 mA per pin · 26 mA Port 0 / 15 mA Ports 1–3 · 71 mA total | AT89S52 datasheet, extracted for this project |
| `VOL` at `IOL` = 1.6 mA; `VOH` at `IOH` = −60 µA | AT89S52 datasheet, extracted for this project |
| Absolute max DC output current 15.0 mA; `VIN` −1.0 V to +7.0 V | AT89S52 datasheet, extracted for this project |
| DIP-40 pinout and alternate functions | AT89S52 datasheet, extracted for this project |
| 9600 baud = 11.0592 MHz / (12 × 32 × 3), TH1 = 0xFD | AT89S52 datasheet formula, arithmetic shown in §6 |
| LED current 7.7 mA; R2 dissipation 20 mW | Calculated in §7 from `Vf` = 2.0 V and `VOL` = 0.45 V |
| Reset τ = 100 ms; 36–171 ms pulse bracket; 2.17 µs minimum | Calculated in §5 |
| Machine cycle 1.085 µs | Calculated in §4 from fosc |
| 12 MHz baud error **+8.5 %** | Calculated in §11. Some references quote "~8.7 %" for this case; the arithmetic shown gives +8.507 %, and the shown arithmetic is what this document stands behind. Either way it is far outside a UART's ±2–3 % tolerance, so the conclusion does not turn on the third digit. |
| Crystal load caps 30 pF ±10 pF | Standard 8051 oscillator spec; 33 pF chosen as the E12 value inside it |

### Figures NOT in the extracted datasheet set — verify before designing against them

These are standard 8051-family references, not numbers pulled from the datasheet extract this
project verified. They are correct to the best of my knowledge and none of them is load-bearing for
the reference design, but check them against your specific part before relying on one in a marginal
circuit.

| Figure | Where used | Status |
|---|---|---|
| `VIH` = 0.2·VCC + 0.9 = 1.9 V; `VIL` = 0.2·VCC − 0.1 = 0.9 V | §5, §6, §8 | Standard 8051 DC characteristics. Used to argue a 3.3 V adapter can drive pin 10, and to bracket the reset pulse. |
| `VIH1` (RST, XTAL1) = 0.7·VCC = 3.5 V | §5 reset pulse bracket | Standard 8051 DC characteristics. Only affects the *estimated* pulse width, which is ≥16 000× the requirement at either end of the bracket. |
| ALE frequency = fosc / 6 = 1.8432 MHz | §2 pin 30, §4 | Standard 8051 architecture. Informational only — pin 30 is unconnected. |
| Strong pull-up asserted for 2 oscillator periods on a 0→1 port transition | §6 adapter logic level | Standard 8051 quasi-bidirectional port behaviour. This is the reason R3 is recommended rather than dismissed. |
| On-chip flash = 8 KB (both parts) | §3 EA table | Standard part spec. Informational. |
| MCU active current ≈25–30 mA | §3 power budget | Standard 8051 figure for supply budgeting. A USB port has 15× the margin, so precision does not matter here. |
| STC89 enters its bootloader **only** on a cold power-on reset, not a warm RST | §5, §10 | Inferred from `stcgal`'s documented behaviour (it prints `Waiting for MCU, please cycle power:` and blocks), not read out of a datasheet. It is the operative assumption behind the whole §10 flashing sequence — if it were wrong, pressing SW1 would be enough. It is not; cycle power. |
| Breadboard centre channel = 0.3 in vs. a 600 mil part | §1 BOM | Mechanical, varies by board. Hence "slightly off-centre placement is normal". |
