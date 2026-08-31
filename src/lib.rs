//! `mcs51-mcp` — an MCP stdio server wrapping the 8051 (MCS-51) development loop.
//!
//! The server runs on the *host*. The 8051 itself only runs reference firmware and
//! answers a newline-terminated UART line protocol; nothing in this crate executes
//! on the microcontroller.
//!
//! Layout:
//! - [`envelope`] — the single response shape every tool returns.
//! - [`errors`] — closed [`errors::ErrorCode`] set plus [`errors::AppError`].
//! - [`config`] — environment-derived knobs, resolved once at startup.
//! - [`paths`] — `FIRMWARE_ROOT` confinement.
//! - [`proc`] — the one and only subprocess runner.
//! - [`serial`] — session registry over blocking `serialport` handles.
//! - [`hw`] — datasheet constants and the DIP-40 pin table.
//! - [`tools`] — the twelve tool bodies.
//! - [`server`] — rmcp wiring: state plus twelve thin shims.

pub mod config;
pub mod envelope;
pub mod errors;
pub mod hw;
pub mod paths;
pub mod proc;
pub mod serial;
pub mod server;
pub mod tools;

/// Canonical tool names. The wire names are part of the contract, so they live in
/// one place rather than being retyped at each `#[tool]` attribute.
pub mod names {
    pub const DOCTOR: &str = "doctor";
    pub const LIST_SERIAL_PORTS: &str = "list_serial_ports";
    pub const COMPILE: &str = "compile";
    pub const FLASH: &str = "flash";
    pub const SERIAL_OPEN: &str = "serial_open";
    pub const SERIAL_WRITE: &str = "serial_write";
    pub const SERIAL_READ: &str = "serial_read";
    pub const SERIAL_EXPECT: &str = "serial_expect";
    pub const SERIAL_CLOSE: &str = "serial_close";
    pub const SERIAL_LIST_SESSIONS: &str = "serial_list_sessions";
    pub const SAFETY_PREFLIGHT: &str = "safety_preflight";
    pub const PINOUT: &str = "pinout";

    /// Every tool this server exposes, in registration order.
    pub const ALL: [&str; 12] = [
        DOCTOR,
        LIST_SERIAL_PORTS,
        COMPILE,
        FLASH,
        SERIAL_OPEN,
        SERIAL_WRITE,
        SERIAL_READ,
        SERIAL_EXPECT,
        SERIAL_CLOSE,
        SERIAL_LIST_SESSIONS,
        SAFETY_PREFLIGHT,
        PINOUT,
    ];
}
