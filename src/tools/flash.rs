//! `flash` — write a hex image to the target.
//!
//! Two chips, two completely different stories:
//!
//! - **`stc`** (STC89C52, the default): a serial bootloader, so one USB-TTL
//!   adapter both flashes and talks. The catch is that the bootloader only
//!   listens for a few milliseconds after power-up, so `stcgal` starts by
//!   waiting — **the board must be power-cycled after this call starts**. Every
//!   response says so, because a caller who does not know this just sees a hang.
//! - **`at89s`**: no serial bootloader at all. It is programmed over SPI ISP,
//!   which needs hardware this server cannot drive. That is a documented stub
//!   with real alternatives, not a silent failure.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::envelope::{Envelope, NextAction};
use crate::errors::{AppError, ErrorCode};
use crate::names;
use crate::proc::{self, runner::STCGAL_GRACE};
use crate::server::Server;

/// The sentence that has to appear in every `stc` response.
const POWER_CYCLE: &str =
    "The STC bootloader only listens during the first few milliseconds after power-up. \
     stcgal is now waiting for that window: POWER-CYCLE THE BOARD (unplug and replug its \
     power, or press the on-board reset that cuts power) to enter the bootloader. \
     Nothing will happen until you do.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FlashArgs {
    /// `stc` for an STC89C52 over its serial bootloader, or `at89s` for an
    /// AT89S51/52 (which needs an external SPI ISP programmer).
    pub chip: String,
    /// Path to the Intel-HEX image to write.
    pub hex: String,
    /// Serial device path, e.g. `/dev/cu.usbserial-10`. Use a `/dev/cu.*` node.
    pub port: String,
}

pub async fn run(server: &Server, args: FlashArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();

    match args.chip.trim().to_ascii_lowercase().as_str() {
        "stc" | "stc89" | "stc89c52" => match flash_stc(server, args, started).await {
            Ok(env) => Ok(env),
            // Even when the call never reaches stcgal — a missing hex, a port we
            // already hold — say what flashing an STC part will require, so the
            // caller learns it before their second attempt rather than after it.
            Err(err) => Ok(err
                .into_envelope(names::FLASH)
                .next_action(NextAction::advice(POWER_CYCLE))),
        },
        "at89s" | "at89s51" | "at89s52" => Ok(at89s_stub(args, started)),
        other => Err(AppError::UnsupportedChip {
            chip: other.to_string(),
        }),
    }
}

async fn flash_stc(
    server: &Server,
    args: FlashArgs,
    started: std::time::Instant,
) -> Result<Envelope, AppError> {
    let cfg = server.config();

    // A serial port is exclusive. Handing stcgal a port we already hold would
    // either fail obscurely or, worse, half-work.
    if let Some(holder) = server.sessions.holder_of(&args.port) {
        return Err(AppError::PortHeldBySession {
            port: args.port.clone(),
            session: holder,
        });
    }

    let hex = server.paths.resolve_input_file(&args.hex)?;
    let invocation = server.stcgal_invocation().await?;

    let spec = invocation
        .spec(cfg.flash_timeout)
        .arg("-p")
        .arg(&args.port)
        .arg(hex.display().to_string())
        // pyserial restores the tty's termios on the way out; a hard kill in
        // the middle of that leaves the port needing a re-plug.
        .grace(STCGAL_GRACE)
        .capture_cap(cfg.capture_cap);

    let command = spec.display();
    let outcome = proc::run(spec).await?;

    let data = json!({
        "chip": "stc",
        "part": "STC89C52 (8052 core)",
        "hex": hex.display().to_string(),
        "port": args.port,
        "power_cycle_required": true,
        "instructions": POWER_CYCLE,
        "after": "Once flashed, the same port carries the firmware's 9600 8N1 line protocol: \
                  open a session with serial_open and send PING to get PONG.",
    });

    let mut env = Envelope::new(names::FLASH)
        .command(command)
        .exit_code(outcome.exit_code)
        .stdout(outcome.stdout.clone())
        .stderr(outcome.stderr.clone())
        .data(data)
        .duration(started.elapsed());

    if outcome.timed_out {
        return Ok(env
            .error(
                ErrorCode::ProcessTimeout,
                format!(
                    "stcgal did not finish within {} ms and was terminated (SIGTERM, then \
                     SIGKILL, then reaped). {POWER_CYCLE}",
                    cfg.flash_timeout.as_millis()
                ),
            )
            .remedy(
                "Almost always this means the board was never power-cycled, so the bootloader \
                 window never opened. Start the flash, then cut and restore the board's power.",
            ));
    }

    if !outcome.success() {
        return Ok(env
            .error(
                ErrorCode::FlashFailed,
                format!("stcgal exited {:?}. {POWER_CYCLE}", outcome.exit_code),
            )
            .remedy(
                "Check that the board was power-cycled after the call started, that TX/RX are \
                 crossed (adapter TX to P3.0/RXD, adapter RX to P3.1/TXD), and that grounds are \
                 common. Then retry.",
            ));
    }

    env = env
        .next_action(NextAction::call(
            names::SERIAL_OPEN,
            "Talk to the firmware you just flashed.",
            json!({ "port": args.port, "baud": crate::hw::limits::DEFAULT_BAUD, "session": "board" }),
        ))
        .next_action(NextAction::advice(POWER_CYCLE));
    Ok(env)
}

/// The AT89S path: honest about what it cannot do, and useful anyway.
fn at89s_stub(args: FlashArgs, started: std::time::Instant) -> Envelope {
    let data = json!({
        "chip": "at89s",
        "supported": false,
        "why": "The AT89S51/52 has no serial bootloader. It is programmed over SPI in-system \
                programming: MOSI on P1.5 (pin 6), MISO on P1.6 (pin 7), SCK on P1.7 (pin 8), \
                with RST (pin 9) held high for the whole session. A USB-TTL adapter speaks \
                asynchronous UART and cannot produce those clocked SPI transactions, so no \
                amount of software on this side can flash one over the port you passed.",
        "isp_pins": {
            "MOSI": "P1.5 / pin 6",
            "MISO": "P1.6 / pin 7",
            "SCK":  "P1.7 / pin 8",
            "RST":  "pin 9, held high throughout",
            "GND":  "pin 20",
            "VCC":  "pin 40",
        },
        "alternatives": [
            {
                "hardware": "minipro with a TL866II+ or T48 universal programmer",
                "how": "Seat the DIP-40 in the ZIF socket and run: minipro -p AT89S52 -w firmware.hex",
                "note": "Off-board programming; nothing needs to be wired. The most reliable option."
            },
            {
                "hardware": "USBasp (or any USB SPI programmer) with avrdude",
                "how": "Wire MOSI/MISO/SCK/RST/VCC/GND to the pins above, then: \
                        avrdude -c usbasp -p 89s52 -U flash:w:firmware.hex:i",
                "note": "avrdude speaks the AT89S ISP protocol despite the AVR name. In-circuit."
            },
            {
                "hardware": "An Arduino running ArduinoISP (Arduino-as-ISP)",
                "how": "Load the ArduinoISP sketch, wire the same six pins, then use avrdude \
                        with -c stk500v1 and the Arduino's port.",
                "note": "Cheapest route if an Arduino is already on the bench."
            }
        ],
        "recommended_instead": "This server targets the STC89C52, which has a serial bootloader: \
                                one USB-TTL adapter both flashes it and talks to it. \
                                Call flash with chip=\"stc\".",
        "requested": { "hex": args.hex, "port": args.port },
    });

    Envelope::new(names::FLASH)
        .error(
            ErrorCode::At89sNotSupported,
            "AT89S parts are programmed over SPI ISP, not over a serial port. This server \
             cannot flash one; see `data.alternatives` for three ways that work.",
        )
        .remedy(
            "Use a hardware programmer — minipro with a TL866II+/T48, a USBasp with avrdude, or \
             an Arduino running ArduinoISP — or switch to an STC89C52 and call \
             flash(chip=\"stc\"), which needs no extra hardware.",
        )
        .data(data)
        .next_action(NextAction::call(
            names::FLASH,
            "The STC89C52 flashes over the serial port you already have.",
            json!({ "chip": "stc" }),
        ))
        .duration(started.elapsed())
}
