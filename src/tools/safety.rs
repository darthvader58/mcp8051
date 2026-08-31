//! `safety_preflight` — check a pin plan against the datasheet before it is wired.
//!
//! Every rule here comes from [`crate::hw::limits`] and [`crate::hw::pins`], so
//! this tool and `pinout` cannot disagree about what a pin is.
//!
//! The one rule that could not be written as a static lookup is
//! [`FindingCode::UartPinConflict`], which consults the live session registry:
//! driving P3.0 is always a bad idea, but doing it *while the server is talking
//! over that UART* severs the only link to the board. Knowing the difference is
//! what makes this a tool rather than a paragraph of documentation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::envelope::{Envelope, NextAction};
use crate::errors::{AppError, ErrorCode};
use crate::hw::{limits, pins};
use crate::names;
use crate::server::Server;

/// What the pin is being asked to do. An enum rather than a raw 0/1 so the
/// schema documents itself and a caller cannot silently invert the logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Drive the pin high (source current).
    High,
    /// Drive the pin low (sink current). This is how an 8051 drives loads.
    Low,
    /// Leave it as an input / high-impedance.
    Input,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SafetyArgs {
    /// 8051 port number 0-3 (P0-P3), **not** a serial device path.
    pub mcu_port: u8,
    /// Bit within that port, 0-7.
    pub bit: u8,
    /// Intended drive: `high`, `low` or `input`.
    pub level: Level,
    /// Load current through the pin in milliamps, if known. Omitting it skips
    /// every current check, which is reported rather than assumed safe.
    pub load_ma: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingCode {
    PortOutOfRange,
    BitOutOfRange,
    UartPinConflict,
    P0NeedsExternalPullup,
    SourceCurrentInsufficient,
    SinkCurrentOverPinLimit,
    SinkCurrentNearPinLimit,
    PortAggregateAdvisory,
    IspPinConflict,
    InterruptPinRepurposed,
    ExternalMemoryPin,
    LoadNotSpecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub code: FindingCode,
    pub severity: Severity,
    pub message: String,
    pub remedy: String,
}

impl Finding {
    fn new(
        code: FindingCode,
        severity: Severity,
        message: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            remedy: remedy.into(),
        }
    }
}

/// Rolled-up verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    PassWithWarnings,
    Blocked,
}

pub async fn run(server: &Server, args: SafetyArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let SafetyArgs {
        mcu_port,
        bit,
        level,
        load_ma,
    } = args;

    let mut findings: Vec<Finding> = Vec::new();

    // --- range ---------------------------------------------------------------
    if mcu_port > 3 {
        findings.push(Finding::new(
            FindingCode::PortOutOfRange,
            Severity::Blocker,
            format!("mcu_port={mcu_port} does not exist; the 8051 has ports P0-P3."),
            "Pass 0, 1, 2 or 3. Note mcu_port is an 8051 port number, not a serial device path \
             — if you meant a /dev/cu.* device, that belongs to serial_open or flash.",
        ));
    }
    if bit > 7 {
        findings.push(Finding::new(
            FindingCode::BitOutOfRange,
            Severity::Blocker,
            format!("bit={bit} is out of range; each 8051 port is 8 bits wide."),
            "Pass 0-7.",
        ));
    }

    let pin = if mcu_port <= 3 && bit <= 7 {
        pins::by_port_bit(mcu_port, bit)
    } else {
        None
    };

    if let Some(info) = pin {
        collect_pin_findings(server, info, mcu_port, bit, level, load_ma, &mut findings);
    }

    // --- roll up -------------------------------------------------------------
    let worst = findings.iter().map(|f| f.severity).max();
    let verdict = match worst {
        Some(Severity::Blocker) => Verdict::Blocked,
        Some(Severity::Warning) => Verdict::PassWithWarnings,
        _ => Verdict::Pass,
    };

    let pin_label = pin.map(|p| {
        json!({
            "pin": p.pin,
            "name": p.name,
            "alt": p.alt,
            "note": p.note,
        })
    });

    let data = json!({
        "request": {
            "mcu_port": mcu_port,
            "bit": bit,
            "level": level,
            "load_ma": load_ma,
        },
        "pin": pin_label,
        "verdict": verdict,
        "findings": findings,
        "counts": {
            "blocker": count(&findings, Severity::Blocker),
            "warning": count(&findings, Severity::Warning),
            "info": count(&findings, Severity::Info),
        },
        "limits_ma": {
            "per_pin_sink_max": limits::PIN_SINK_MAX_MA,
            "per_pin_sink_warn": limits::PIN_SINK_WARN_MA,
            "per_pin_source_spec_ua": limits::PIN_SOURCE_SPEC_UA,
            "port_aggregate": limits::port_aggregate_ma(mcu_port.min(3)),
            "all_pins_aggregate": limits::ALL_PINS_AGGREGATE_MA,
        },
    });

    let mut env = Envelope::new(names::SAFETY_PREFLIGHT)
        .data(data)
        .duration(started.elapsed());

    env = match verdict {
        // `ok: false` is load-bearing. A caller that only skims `ok` must not
        // be able to read "blocked" as "fine".
        Verdict::Blocked => {
            let blockers: Vec<&str> = findings
                .iter()
                .filter(|f| f.severity == Severity::Blocker)
                .map(|f| f.message.as_str())
                .collect();
            let remedy = findings
                .iter()
                .find(|f| f.severity == Severity::Blocker)
                .map(|f| f.remedy.clone())
                .unwrap_or_default();
            env.error(ErrorCode::SafetyBlocked, blockers.join(" "))
                .remedy(remedy)
        }
        Verdict::PassWithWarnings => env.warn(),
        Verdict::Pass => env,
    };

    if pin.is_some() && verdict != Verdict::Blocked {
        env = env.next_action(NextAction::advice(
            "Wire it up, then drive it from firmware with the line protocol: \
             `SET <port> <bit> <value>` (0 drives low, 1 releases high).",
        ));
    }

    Ok(env)
}

fn count(findings: &[Finding], sev: Severity) -> usize {
    findings.iter().filter(|f| f.severity == sev).count()
}

#[allow(clippy::too_many_arguments)]
fn collect_pin_findings(
    server: &Server,
    info: &pins::PinInfo,
    mcu_port: u8,
    bit: u8,
    level: Level,
    load_ma: Option<f64>,
    findings: &mut Vec<Finding>,
) {
    let name = info.name;

    // --- the UART link -------------------------------------------------------
    if mcu_port == 3 && (bit == 0 || bit == 1) {
        let function = if bit == 0 { "RXD" } else { "TXD" };
        let live: Vec<_> = server.sessions.list();
        let message = if live.is_empty() {
            format!(
                "{name} is {function}, the UART the reference firmware uses to answer commands. \
                 Driving it as GPIO breaks the serial link and the board can only be recovered \
                 by a power cycle."
            )
        } else {
            let ports: Vec<String> = live
                .iter()
                .map(|s| format!("{} on {}", s.session, s.port))
                .collect();
            format!(
                "{name} is {function}, and this server currently has {} open serial session(s) \
                 ({}). Driving it now would sever the link mid-session: the server would stop \
                 receiving replies and the board would need a power cycle to recover.",
                live.len(),
                ports.join(", ")
            )
        };
        findings.push(Finding::new(
            FindingCode::UartPinConflict,
            Severity::Blocker,
            message,
            "Pick any pin outside P3.0/P3.1. The reference firmware also refuses `SET 3 0` and \
             `SET 3 1` with ERR for exactly this reason.",
        ));
    }

    // --- Port 0 is open-drain ------------------------------------------------
    if mcu_port == 0 {
        let driving_high_into_load = level == Level::High && load_ma.is_some_and(|ma| ma > 0.0);
        let severity = if driving_high_into_load {
            Severity::Blocker
        } else {
            Severity::Warning
        };
        let message = if driving_high_into_load {
            format!(
                "{name} is on Port 0, which is open-drain with no internal pull-up. Writing a 1 \
                 does not drive the pin high, it just releases it — with a load attached the pin \
                 floats rather than reaching VOH, so this circuit will not work as written."
            )
        } else {
            format!(
                "{name} is on Port 0, which is open-drain with no internal pull-up. It can pull \
                 low but cannot source a high on its own."
            )
        };
        findings.push(Finding::new(
            FindingCode::P0NeedsExternalPullup,
            severity,
            message,
            "Fit an external pull-up (10k to VCC is typical), or move the signal to P1/P2/P3, \
             which have weak internal pull-ups.",
        ));
    }

    // --- current -------------------------------------------------------------
    match load_ma {
        None => findings.push(Finding::new(
            FindingCode::LoadNotSpecified,
            Severity::Info,
            "load_ma was not given, so the source and sink current checks were skipped. \
             This is not a statement that the current is safe.",
            format!(
                "Pass load_ma to have the {} mA per-pin sink limit and the {} uA source \
                 specification checked. For an LED: (VCC - Vf) / R, e.g. (5 - 2.0) / 330 = 9.1 mA.",
                limits::PIN_SINK_MAX_MA,
                limits::PIN_SOURCE_SPEC_UA
            ),
        )),
        Some(ma) if ma < 0.0 => findings.push(Finding::new(
            FindingCode::LoadNotSpecified,
            Severity::Info,
            format!("load_ma={ma} is negative and was ignored; pass the magnitude."),
            "Give the current as a positive number of milliamps.",
        )),
        Some(ma) => match level {
            Level::High if ma > limits::PIN_SOURCE_LIMIT_MA => {
                findings.push(Finding::new(
                    FindingCode::SourceCurrentInsufficient,
                    Severity::Blocker,
                    format!(
                        "Driving {name} high into {ma} mA is not something this part can do. \
                         VOH is specified at IOH = -{} uA, i.e. {} mA — about {:.0}x less than \
                         requested. The pin will sag well below a logic high.",
                        limits::PIN_SOURCE_SPEC_UA,
                        limits::PIN_SOURCE_SPEC_UA / 1000.0,
                        ma / (limits::PIN_SOURCE_SPEC_UA / 1000.0)
                    ),
                    "Rewire the load active-low: supply -> resistor -> load -> pin, and drive \
                     the pin LOW to turn it on. The pin then sinks, which it does well. \
                     This is why 8051 boards wire LEDs to VCC rather than to ground.",
                ));
            }
            Level::Low if ma > limits::PIN_SINK_MAX_MA => {
                findings.push(Finding::new(
                    FindingCode::SinkCurrentOverPinLimit,
                    Severity::Blocker,
                    format!(
                        "Sinking {ma} mA through {name} exceeds the {} mA per-pin IOL maximum \
                         (absolute maximum DC output current is {} mA).",
                        limits::PIN_SINK_MAX_MA,
                        limits::ABS_MAX_DC_OUTPUT_MA
                    ),
                    format!(
                        "Raise the series resistor so the current lands under {} mA, or drive \
                         the load through a transistor or a ULN2003-style sink driver.",
                        limits::PIN_SINK_MAX_MA
                    ),
                ));
            }
            Level::Low if ma > limits::PIN_SINK_WARN_MA => {
                findings.push(Finding::new(
                    FindingCode::SinkCurrentNearPinLimit,
                    Severity::Warning,
                    format!(
                        "Sinking {ma} mA through {name} is within the {} mA per-pin limit but \
                         leaves little margin, and VOL rises with IOL.",
                        limits::PIN_SINK_MAX_MA
                    ),
                    "Fine for one pin. If several pins on this port sink at once, check the \
                     port aggregate below.",
                ));
            }
            _ => {}
        },
    }

    // --- aggregate budgets ---------------------------------------------------
    if level != Level::Input {
        let port_budget = limits::port_aggregate_ma(mcu_port);
        findings.push(Finding::new(
            FindingCode::PortAggregateAdvisory,
            Severity::Warning,
            format!(
                "Per-pin limits are not the only ceiling: Port {mcu_port} may sink {port_budget} mA \
                 in total ({} mA for Port 0, {} mA for each of Ports 1-3), and all pins together \
                 may sink {} mA. This tool sees one pin, so it cannot check the total for you.",
                limits::PORT0_AGGREGATE_MA,
                limits::PORT123_AGGREGATE_MA,
                limits::ALL_PINS_AGGREGATE_MA
            ),
            format!(
                "Add up every pin you drive at once and keep Port {mcu_port} under \
                 {port_budget} mA and the whole device under {} mA.",
                limits::ALL_PINS_AGGREGATE_MA
            ),
        ));
    }

    // --- alternate functions -------------------------------------------------
    if mcu_port == 1 && (5..=7).contains(&bit) {
        let func = info.alt.first().copied().unwrap_or("SPI");
        findings.push(Finding::new(
            FindingCode::IspPinConflict,
            Severity::Warning,
            format!(
                "{name} is {func}, part of the SPI in-system-programming interface \
                 (P1.5=MOSI, P1.6=MISO, P1.7=SCK). A hardware ISP programmer drives these pins \
                 together with RST."
            ),
            "Safe to use as GPIO while running, but unplug or isolate the ISP programmer before \
             driving it, and expect the pin to move during programming.",
        ));
    }

    if mcu_port == 3 && (bit == 2 || bit == 3) {
        let func = info.alt.first().copied().unwrap_or("INT");
        findings.push(Finding::new(
            FindingCode::InterruptPinRepurposed,
            Severity::Info,
            format!(
                "{name} is also {func}, an external interrupt input. Using it as GPIO is fine \
                 as long as that interrupt is not enabled."
            ),
            "Leave IE's EX0/EX1 bit clear, or move the signal if you need the interrupt.",
        ));
    }

    if mcu_port == 2 || mcu_port == 0 {
        let (role, detail) = if mcu_port == 2 {
            ("A8-A15", "the high address byte")
        } else {
            ("AD0-AD7", "the multiplexed low address / data bus")
        };
        findings.push(Finding::new(
            FindingCode::ExternalMemoryPin,
            Severity::Info,
            format!(
                "{name} doubles as {role}, {detail} for external memory access. \
                 Irrelevant when running from internal flash with EA tied high, which is the \
                 normal configuration for this board."
            ),
            "Keep EA (pin 31) tied to VCC and avoid MOVX so the port stays yours.",
        ));
    }
}
