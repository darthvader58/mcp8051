//! Electrical and timing limits, straight from the AT89S52 datasheet.
//!
//! These numbers are the reason `safety_preflight` exists. An 8051 port pin
//! sinks two orders of magnitude more current than it sources, which is why
//! every LED on a classic 8051 board is wired active-low.

/// Max `IOL` for one port pin, in mA. Above this the part is out of spec.
pub const PIN_SINK_MAX_MA: f64 = 10.0;

/// Where a per-pin sink stops being comfortable, in mA.
pub const PIN_SINK_WARN_MA: f64 = 8.0;

/// `IOH` at which `VOH` is specified, in µA. This is the whole story on sourcing.
pub const PIN_SOURCE_SPEC_UA: f64 = 60.0;

/// Any source load above this is treated as "this pin cannot do that", in mA.
/// The pin is specified at 60 µA; 0.1 mA is already generous.
pub const PIN_SOURCE_LIMIT_MA: f64 = 0.1;

/// Aggregate `IOL` budget for Port 0, in mA.
pub const PORT0_AGGREGATE_MA: f64 = 26.0;

/// Aggregate `IOL` budget for each of Ports 1, 2 and 3, in mA.
pub const PORT123_AGGREGATE_MA: f64 = 15.0;

/// Aggregate `IOL` budget across every output pin at once, in mA.
pub const ALL_PINS_AGGREGATE_MA: f64 = 71.0;

/// Absolute maximum DC output current for any one pin, in mA.
pub const ABS_MAX_DC_OUTPUT_MA: f64 = 15.0;

/// Input voltage absolute maximum range on any pin, in volts.
pub const VIN_MIN_V: f64 = -1.0;
pub const VIN_MAX_V: f64 = 7.0;

/// Reference crystal on the board this server targets, in Hz.
pub const CRYSTAL_HZ: u32 = 11_059_200;

/// UART baud the reference firmware uses.
pub const DEFAULT_BAUD: u32 = 9600;

/// Timer 1 reload for 9600 baud at 11.0592 MHz with SMOD = 0.
pub const TH1_9600: u8 = 0xFD;

/// The aggregate budget for a given 8051 port number.
pub const fn port_aggregate_ma(port: u8) -> f64 {
    if port == 0 {
        PORT0_AGGREGATE_MA
    } else {
        PORT123_AGGREGATE_MA
    }
}

/// Typical LED resistor for a 5 V rail driving an active-low LED, in ohms.
pub const DEMO_LED_RESISTOR_OHMS: u32 = 330;

/// Sink current of the reference demo circuit, in mA:
/// `+5V -> 330R -> LED(~2.0V Vf) -> P1.0`.
pub const DEMO_LED_SINK_MA: f64 = 7.7;
