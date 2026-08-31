//! The DIP-40 pin table — the single source of truth for pin identity.
//!
//! Both `pinout` (which reports it) and `safety_preflight` (which reasons over
//! it) read this table, so the documentation and the rules can never disagree.
//!
//! Note Port 0 **descends** across the package: pin 32 is P0.7 and pin 39 is
//! P0.0. Getting that backwards is the classic 8051 breadboard mistake.

use serde::Serialize;

/// What a pin fundamentally is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    /// A bit of one of the four bidirectional I/O ports.
    Io,
    /// Supply or ground.
    Power,
    /// Crystal oscillator.
    Clock,
    /// A dedicated control pin (RST, PSEN, ALE, EA).
    Control,
}

/// One physical pin of the 40-pin DIP package.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PinInfo {
    /// Physical pin number, 1-40.
    pub pin: u8,
    /// Primary name, e.g. `P1.0` or `VCC`.
    pub name: &'static str,
    /// 8051 port number 0-3, for I/O pins.
    pub port: Option<u8>,
    /// Bit within the port, 0-7, for I/O pins.
    pub bit: Option<u8>,
    pub kind: PinKind,
    /// Alternate functions multiplexed onto this pin.
    pub alt: &'static [&'static str],
    /// Why the pin matters in practice.
    pub note: &'static str,
}

const OPEN_DRAIN: &str =
    "Port 0 is open-drain in I/O mode with no internal pull-up; needs an external pull-up \
     (10k typical) to drive a logic high.";
const WEAK_PULLUP: &str = "Has a weak internal pull-up; sinks ~10 mA but sources only ~60 uA.";

/// All forty pins, indexed 0-based by `pin - 1`.
pub const DIP40: [PinInfo; 40] = [
    PinInfo {
        pin: 1,
        name: "P1.0",
        port: Some(1),
        bit: Some(0),
        kind: PinKind::Io,
        alt: &["T2"],
        note: "Timer/Counter 2 external count input. The reference demo LED lives here, wired \
               active-low: +5V -> 330R -> LED -> P1.0.",
    },
    PinInfo {
        pin: 2,
        name: "P1.1",
        port: Some(1),
        bit: Some(1),
        kind: PinKind::Io,
        alt: &["T2EX"],
        note: "Timer/Counter 2 capture/reload trigger.",
    },
    PinInfo {
        pin: 3,
        name: "P1.2",
        port: Some(1),
        bit: Some(2),
        kind: PinKind::Io,
        alt: &[],
        note: WEAK_PULLUP,
    },
    PinInfo {
        pin: 4,
        name: "P1.3",
        port: Some(1),
        bit: Some(3),
        kind: PinKind::Io,
        alt: &[],
        note: WEAK_PULLUP,
    },
    PinInfo {
        pin: 5,
        name: "P1.4",
        port: Some(1),
        bit: Some(4),
        kind: PinKind::Io,
        alt: &[],
        note: WEAK_PULLUP,
    },
    PinInfo {
        pin: 6,
        name: "P1.5",
        port: Some(1),
        bit: Some(5),
        kind: PinKind::Io,
        alt: &["MOSI"],
        note: "SPI in-system-programming data in. A hardware ISP programmer drives this pin.",
    },
    PinInfo {
        pin: 7,
        name: "P1.6",
        port: Some(1),
        bit: Some(6),
        kind: PinKind::Io,
        alt: &["MISO"],
        note: "SPI in-system-programming data out. A hardware ISP programmer reads this pin.",
    },
    PinInfo {
        pin: 8,
        name: "P1.7",
        port: Some(1),
        bit: Some(7),
        kind: PinKind::Io,
        alt: &["SCK"],
        note: "SPI in-system-programming clock. A hardware ISP programmer drives this pin.",
    },
    PinInfo {
        pin: 9,
        name: "RST",
        port: None,
        bit: None,
        kind: PinKind::Control,
        alt: &[],
        note: "Active-high reset: hold high for two machine cycles while the oscillator runs. \
               Also pulled by an SPI ISP programmer to enter programming mode.",
    },
    PinInfo {
        pin: 10,
        name: "P3.0",
        port: Some(3),
        bit: Some(0),
        kind: PinKind::Io,
        alt: &["RXD"],
        note: "UART receive. This is half of the only link the server has to the board; driving \
               it as GPIO strands the session until a power cycle.",
    },
    PinInfo {
        pin: 11,
        name: "P3.1",
        port: Some(3),
        bit: Some(1),
        kind: PinKind::Io,
        alt: &["TXD"],
        note: "UART transmit. This is half of the only link the server has to the board; driving \
               it as GPIO strands the session until a power cycle.",
    },
    PinInfo {
        pin: 12,
        name: "P3.2",
        port: Some(3),
        bit: Some(2),
        kind: PinKind::Io,
        alt: &["INT0"],
        note: "External interrupt 0.",
    },
    PinInfo {
        pin: 13,
        name: "P3.3",
        port: Some(3),
        bit: Some(3),
        kind: PinKind::Io,
        alt: &["INT1"],
        note: "External interrupt 1.",
    },
    PinInfo {
        pin: 14,
        name: "P3.4",
        port: Some(3),
        bit: Some(4),
        kind: PinKind::Io,
        alt: &["T0"],
        note: "Timer/Counter 0 external input.",
    },
    PinInfo {
        pin: 15,
        name: "P3.5",
        port: Some(3),
        bit: Some(5),
        kind: PinKind::Io,
        alt: &["T1"],
        note: "Timer/Counter 1 external input. Timer 1 also generates the UART baud clock.",
    },
    PinInfo {
        pin: 16,
        name: "P3.6",
        port: Some(3),
        bit: Some(6),
        kind: PinKind::Io,
        alt: &["WR"],
        note: "External data memory write strobe.",
    },
    PinInfo {
        pin: 17,
        name: "P3.7",
        port: Some(3),
        bit: Some(7),
        kind: PinKind::Io,
        alt: &["RD"],
        note: "External data memory read strobe.",
    },
    PinInfo {
        pin: 18,
        name: "XTAL2",
        port: None,
        bit: None,
        kind: PinKind::Clock,
        alt: &[],
        note: "Inverting oscillator output. With XTAL1 it carries the 11.0592 MHz crystal, the \
               frequency that makes 9600 baud exact.",
    },
    PinInfo {
        pin: 19,
        name: "XTAL1",
        port: None,
        bit: None,
        kind: PinKind::Clock,
        alt: &[],
        note: "Inverting oscillator input and internal clock input. Drive here when using an \
               external clock source.",
    },
    PinInfo {
        pin: 20,
        name: "GND",
        port: None,
        bit: None,
        kind: PinKind::Power,
        alt: &["VSS"],
        note: "Ground. Must share a common ground with the USB-TTL adapter or the UART will \
               produce garbage.",
    },
    PinInfo {
        pin: 21,
        name: "P2.0",
        port: Some(2),
        bit: Some(0),
        kind: PinKind::Io,
        alt: &["A8"],
        note: "External memory address bit 8.",
    },
    PinInfo {
        pin: 22,
        name: "P2.1",
        port: Some(2),
        bit: Some(1),
        kind: PinKind::Io,
        alt: &["A9"],
        note: "External memory address bit 9.",
    },
    PinInfo {
        pin: 23,
        name: "P2.2",
        port: Some(2),
        bit: Some(2),
        kind: PinKind::Io,
        alt: &["A10"],
        note: "External memory address bit 10.",
    },
    PinInfo {
        pin: 24,
        name: "P2.3",
        port: Some(2),
        bit: Some(3),
        kind: PinKind::Io,
        alt: &["A11"],
        note: "External memory address bit 11.",
    },
    PinInfo {
        pin: 25,
        name: "P2.4",
        port: Some(2),
        bit: Some(4),
        kind: PinKind::Io,
        alt: &["A12"],
        note: "External memory address bit 12.",
    },
    PinInfo {
        pin: 26,
        name: "P2.5",
        port: Some(2),
        bit: Some(5),
        kind: PinKind::Io,
        alt: &["A13"],
        note: "External memory address bit 13.",
    },
    PinInfo {
        pin: 27,
        name: "P2.6",
        port: Some(2),
        bit: Some(6),
        kind: PinKind::Io,
        alt: &["A14"],
        note: "External memory address bit 14.",
    },
    PinInfo {
        pin: 28,
        name: "P2.7",
        port: Some(2),
        bit: Some(7),
        kind: PinKind::Io,
        alt: &["A15"],
        note: "External memory address bit 15.",
    },
    PinInfo {
        pin: 29,
        name: "PSEN",
        port: None,
        bit: None,
        kind: PinKind::Control,
        alt: &[],
        note: "Program Store Enable: the read strobe for external program memory. Leave \
               unconnected when running from internal flash.",
    },
    PinInfo {
        pin: 30,
        name: "ALE",
        port: None,
        bit: None,
        kind: PinKind::Control,
        alt: &["PROG"],
        note: "Address Latch Enable, which latches AD0-AD7 off Port 0 during external memory \
               access; doubles as the program pulse input during flash programming.",
    },
    PinInfo {
        pin: 31,
        name: "EA",
        port: None,
        bit: None,
        kind: PinKind::Control,
        alt: &["VPP"],
        note: "External Access: tie HIGH to VCC to run from internal flash, LOW to force all \
               fetches from external program memory. Leaving it floating is a classic \
               'board does nothing' bug.",
    },
    PinInfo {
        pin: 32,
        name: "P0.7",
        port: Some(0),
        bit: Some(7),
        kind: PinKind::Io,
        alt: &["AD7"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 33,
        name: "P0.6",
        port: Some(0),
        bit: Some(6),
        kind: PinKind::Io,
        alt: &["AD6"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 34,
        name: "P0.5",
        port: Some(0),
        bit: Some(5),
        kind: PinKind::Io,
        alt: &["AD5"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 35,
        name: "P0.4",
        port: Some(0),
        bit: Some(4),
        kind: PinKind::Io,
        alt: &["AD4"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 36,
        name: "P0.3",
        port: Some(0),
        bit: Some(3),
        kind: PinKind::Io,
        alt: &["AD3"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 37,
        name: "P0.2",
        port: Some(0),
        bit: Some(2),
        kind: PinKind::Io,
        alt: &["AD2"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 38,
        name: "P0.1",
        port: Some(0),
        bit: Some(1),
        kind: PinKind::Io,
        alt: &["AD1"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 39,
        name: "P0.0",
        port: Some(0),
        bit: Some(0),
        kind: PinKind::Io,
        alt: &["AD0"],
        note: OPEN_DRAIN,
    },
    PinInfo {
        pin: 40,
        name: "VCC",
        port: None,
        bit: None,
        kind: PinKind::Power,
        alt: &[],
        note: "Supply, 4.0-5.5 V for the AT89S52.",
    },
];

/// Look up a physical pin number, 1-40.
pub fn by_number(pin: u8) -> Option<&'static PinInfo> {
    if pin == 0 || pin as usize > DIP40.len() {
        return None;
    }
    Some(&DIP40[pin as usize - 1])
}

/// Look up the physical pin carrying `Pport.bit`.
pub fn by_port_bit(port: u8, bit: u8) -> Option<&'static PinInfo> {
    DIP40
        .iter()
        .find(|p| p.port == Some(port) && p.bit == Some(bit))
}

/// Every pin belonging to one 8051 port, in bit order.
pub fn port_pins(port: u8) -> Vec<&'static PinInfo> {
    let mut v: Vec<_> = DIP40.iter().filter(|p| p.port == Some(port)).collect();
    v.sort_by_key(|p| p.bit);
    v
}
