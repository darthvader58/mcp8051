//! Port enumeration, with macOS's duplicate device nodes untangled.
//!
//! macOS exposes every serial device twice: `/dev/cu.*` (callout) and
//! `/dev/tty.*` (dial-in). They are the same hardware. Opening the `tty.*` node
//! blocks until DCD is asserted, which a bare USB-TTL adapter never does, so a
//! caller that picks the `tty.*` path just hangs. Ranking `cu.*` first and
//! saying why is the difference between a working session and a mystery.

use serde::Serialize;
use serialport::SerialPortType;

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct UsbInfo {
    pub vid: String,
    pub pid: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    /// Best guess at the bridge chip, from the USB IDs.
    pub likely_chip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortEntry {
    /// The path to actually open.
    pub path: String,
    /// `callout` (`/dev/cu.*`), `dial_in` (`/dev/tty.*`) or `other`.
    pub node: &'static str,
    /// The matching `/dev/tty.*` node for the same device, if present.
    pub paired_with: Option<String>,
    /// `usb` / `bluetooth` / `pci` / `unknown`.
    pub transport: &'static str,
    pub usb: Option<UsbInfo>,
    /// True when this is the node a caller should pass to `serial_open`.
    pub recommended: bool,
    pub note: Option<String>,
}

/// Recognize the common USB-TTL bridges by VID:PID.
fn likely_chip(vid: u16, pid: u16) -> Option<String> {
    Some(
        match (vid, pid) {
            (0x10c4, 0xea60) => "Silicon Labs CP2102 (driverless on Apple Silicon)",
            (0x1a86, 0x7523) => "WCH CH340 (needs the WCH DriverKit driver on Apple Silicon)",
            (0x1a86, 0x55d4) => "WCH CH9102 (needs the WCH DriverKit driver on Apple Silicon)",
            (0x0403, 0x6001) => "FTDI FT232R (driverless on Apple Silicon)",
            (0x0403, 0x6015) => "FTDI FT231X (driverless on Apple Silicon)",
            (0x067b, 0x2303) => "Prolific PL2303",
            _ => return None,
        }
        .to_string(),
    )
}

fn strip_node(path: &str) -> Option<(&'static str, &str)> {
    if let Some(rest) = path.strip_prefix("/dev/cu.") {
        Some(("callout", rest))
    } else if let Some(rest) = path.strip_prefix("/dev/tty.") {
        Some(("dial_in", rest))
    } else {
        None
    }
}

/// List every serial port, callout nodes first.
pub fn list() -> Result<Vec<PortEntry>, AppError> {
    let ports = serialport::available_ports()
        .map_err(|e| AppError::io("enumerating serial ports", std::io::Error::from(e)))?;

    let all: Vec<String> = ports.iter().map(|p| p.port_name.clone()).collect();

    let mut entries: Vec<PortEntry> = ports
        .into_iter()
        .map(|p| {
            let (node, stem) = strip_node(&p.port_name).unwrap_or(("other", ""));

            // Pair the two nodes that describe one physical device.
            let paired_with = if stem.is_empty() {
                None
            } else {
                let other = match node {
                    "callout" => format!("/dev/tty.{stem}"),
                    "dial_in" => format!("/dev/cu.{stem}"),
                    _ => String::new(),
                };
                all.iter().find(|c| **c == other).cloned()
            };

            let (transport, usb) = match p.port_type {
                SerialPortType::UsbPort(info) => (
                    "usb",
                    // NOTE: `location` and `interface` sit behind non-default
                    // serialport features and are deliberately not referenced.
                    Some(UsbInfo {
                        vid: format!("{:04x}", info.vid),
                        pid: format!("{:04x}", info.pid),
                        likely_chip: likely_chip(info.vid, info.pid),
                        manufacturer: info.manufacturer,
                        product: info.product,
                        serial_number: info.serial_number,
                    }),
                ),
                SerialPortType::BluetoothPort => ("bluetooth", None),
                SerialPortType::PciPort => ("pci", None),
                SerialPortType::Unknown => ("unknown", None),
            };

            let note = match node {
                "dial_in" => Some(
                    "Dial-in node: open() blocks until DCD is asserted, which a USB-TTL adapter \
                     never does. Use the paired /dev/cu.* path instead."
                        .to_string(),
                ),
                "callout" if transport == "bluetooth" => {
                    Some("Bluetooth serial profile, not a wired UART adapter.".to_string())
                }
                _ => None,
            };

            PortEntry {
                recommended: node == "callout" && transport == "usb",
                path: p.port_name,
                node,
                paired_with,
                transport,
                usb,
                note,
            }
        })
        .collect();

    // Rank: recommended, then callout, then USB, then by path.
    entries.sort_by(|a, b| {
        let key = |e: &PortEntry| {
            (
                !e.recommended,
                e.node != "callout",
                e.transport != "usb",
                e.path.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(entries)
}
