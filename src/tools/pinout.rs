//! `pinout` — DIP-40 reference, read from [`crate::hw::pins`].

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::envelope::Envelope;
use crate::errors::AppError;
use crate::hw::{limits, pins};
use crate::names;
use crate::server::Server;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PinoutArgs {
    /// Physical DIP-40 pin number, 1-40. Omit for the whole package.
    pub pin: Option<u8>,
}

pub async fn run(_server: &Server, args: PinoutArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();

    let data = match args.pin {
        Some(n) => {
            let info = pins::by_number(n).ok_or_else(|| {
                AppError::invalid(format!(
                    "pin {n} is not on a DIP-40 package; valid pins are 1-40"
                ))
            })?;
            json!({
                "package": "DIP-40",
                "pin": info,
                "electrical": electrical_note(info),
            })
        }
        None => json!({
            "package": "DIP-40",
            "device": "AT89S52 / STC89C52 (8052 core)",
            "pins": pins::DIP40.as_slice(),
            "ports": {
                "P0": "pins 39..32, and note the order descends: pin 39 is P0.0, pin 32 is P0.7. \
                       Open-drain in I/O mode, so an external pull-up is required to drive high. \
                       Doubles as the multiplexed AD0-AD7 bus.",
                "P1": "pins 1..8, plain bidirectional I/O with weak internal pull-ups. \
                       P1.5/P1.6/P1.7 are MOSI/MISO/SCK for SPI in-system programming.",
                "P2": "pins 21..28, bidirectional I/O; doubles as address lines A8-A15.",
                "P3": "pins 10..17, bidirectional I/O with alternate functions: \
                       RXD, TXD, INT0, INT1, T0, T1, WR, RD.",
            },
            "current_budget_ma": {
                "per_pin_sink_max": limits::PIN_SINK_MAX_MA,
                "port0_total": limits::PORT0_AGGREGATE_MA,
                "port1_2_3_total_each": limits::PORT123_AGGREGATE_MA,
                "all_pins_total": limits::ALL_PINS_AGGREGATE_MA,
                "per_pin_source_spec_ua": limits::PIN_SOURCE_SPEC_UA,
            },
            "clock": {
                "crystal_hz": limits::CRYSTAL_HZ,
                "why": "11.0592 MHz divides to exactly 9600 baud: 11059200 / (12 * 32 * (256 - 0xFD)).",
                "uart": format!("9600 8N1, Timer 1 mode 2 auto-reload, TH1 = 0x{:02X}", limits::TH1_9600),
            },
        }),
    };

    Ok(Envelope::new(names::PINOUT)
        .data(data)
        .duration(started.elapsed()))
}

fn electrical_note(info: &pins::PinInfo) -> String {
    match info.port {
        Some(0) => format!(
            "Open-drain, no internal pull-up. Sinks up to {} mA; sources essentially nothing \
             ({} uA spec) without an external pull-up. Port 0 aggregate budget {} mA.",
            limits::PIN_SINK_MAX_MA,
            limits::PIN_SOURCE_SPEC_UA,
            limits::PORT0_AGGREGATE_MA
        ),
        Some(_) => format!(
            "Weak internal pull-up. Sinks up to {} mA; VOH is specified at only {} uA of source \
             current, so drive loads active-low. Port aggregate budget {} mA.",
            limits::PIN_SINK_MAX_MA,
            limits::PIN_SOURCE_SPEC_UA,
            limits::PORT123_AGGREGATE_MA
        ),
        None => format!(
            "Not a general-purpose I/O pin. VIN absolute maximum {} V to {} V.",
            limits::VIN_MIN_V,
            limits::VIN_MAX_V
        ),
    }
}
