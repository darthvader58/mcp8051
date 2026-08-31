//! `list_serial_ports`, `serial_open`, `serial_close`, `serial_list_sessions`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::envelope::{Envelope, NextAction};
use crate::errors::AppError;
use crate::hw::limits;
use crate::names;
use crate::serial::ops::IO_SLICE;
use crate::server::Server;

// ---------------------------------------------------------------- list_ports

pub async fn list_ports(_server: &Server) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let ports = crate::serial::enumerate::list()?;
    let recommended: Vec<&str> = ports
        .iter()
        .filter(|p| p.recommended)
        .map(|p| p.path.as_str())
        .collect();

    let data = json!({
        "ports": ports,
        "recommended": recommended,
        "macos_note": "macOS exposes every serial device twice. /dev/cu.* is the callout node \
                       and is the one to use. /dev/tty.* is the dial-in node: open() on it blocks \
                       until DCD is asserted, which a bare USB-TTL adapter never asserts, so it \
                       hangs instead of failing. Entries are ranked with /dev/cu.* first.",
        "driver_note": "CP2102 and FTDI adapters work driverless on Apple Silicon. CH340/CH9102 \
                        adapters need the WCH DriverKit driver installed, or they never appear here.",
    });

    let mut env = Envelope::new(names::LIST_SERIAL_PORTS)
        .data(data)
        .duration(started.elapsed());

    if recommended.is_empty() {
        env = env.warn().remedy(
            "No USB serial adapter was found. Plug one in; if it is a CH340 clone, install the \
             WCH DriverKit driver first.",
        );
    } else {
        env = env.next_action(NextAction::call(
            names::SERIAL_OPEN,
            "Open a session on the recommended port.",
            json!({ "port": recommended[0], "baud": limits::DEFAULT_BAUD, "session": "board" }),
        ));
    }
    Ok(env)
}

// --------------------------------------------------------------- serial_open

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenArgs {
    /// Serial device path. Use a `/dev/cu.*` node — `/dev/tty.*` blocks on DCD.
    pub port: String,
    /// Baud rate. Defaults to 9600, which is what the reference firmware uses.
    pub baud: Option<u32>,
    /// Caller-chosen id used by every later `serial_*` call.
    pub session: String,
}

pub async fn open(server: &Server, args: OpenArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let baud = args.baud.unwrap_or(server.config().default_baud);

    if args.session.trim().is_empty() {
        return Err(AppError::invalid("session id must not be empty"));
    }
    if baud == 0 {
        return Err(AppError::invalid("baud must be greater than zero"));
    }

    let info = server
        .sessions
        .open(&args.session, &args.port, baud, IO_SLICE)?;

    let mut env = Envelope::new(names::SERIAL_OPEN)
        .data(json!({
            "session": info,
            "settings": {
                "baud": baud,
                "framing": "8N1",
                "io_slice_ms": IO_SLICE.as_millis() as u64,
            },
            "protocol": {
                "framing": "newline-terminated ASCII; serial_write appends \\n when missing",
                "commands": {
                    "PING": "PONG",
                    "SET p b v": "OK  (p=0..3, b=0..7, v=0|1)",
                    "GET p b": "0 or 1",
                    "WRP p hh": "OK  (write a whole port, hh = 2 hex digits)",
                    "RDP p": "hh  (read a whole port)",
                    "<anything else>": "ERR",
                },
                "note": "Writes to P3.0/P3.1 answer ERR: they are RXD/TXD, the link itself.",
            },
        }))
        .duration(started.elapsed());

    if args.port.starts_with("/dev/tty.") {
        env = env.warn().remedy(
            "This is a dial-in node. It opened, but reads can block on DCD; prefer the paired \
             /dev/cu.* path.",
        );
    }

    Ok(env.next_action(NextAction::call(
        names::SERIAL_WRITE,
        "Check the board is alive.",
        json!({ "session": args.session, "data": "PING" }),
    )))
}

// -------------------------------------------------------------- serial_close

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseArgs {
    /// Id of the session to close.
    pub session: String,
}

pub async fn close(server: &Server, args: CloseArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let (info, deferred) = server.sessions.close(&args.session)?;

    let mut env = Envelope::new(names::SERIAL_CLOSE)
        .data(json!({
            "session": info,
            "closed": !deferred,
            "deferred": deferred,
        }))
        .duration(started.elapsed());

    if deferred {
        env = env.warn().remedy(
            "An operation is still running on this port. The close was recorded: the port will \
             be dropped and the session removed the moment that operation checks back in. \
             The id is already unusable for new calls.",
        );
    }
    Ok(env)
}

// ------------------------------------------------------- serial_list_sessions

pub async fn list_sessions(server: &Server) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let sessions = server.sessions.list();
    let poisoned = sessions.iter().filter(|s| s.state == "poisoned").count();

    let mut env = Envelope::new(names::SERIAL_LIST_SESSIONS)
        .data(json!({
            "sessions": sessions,
            "count": sessions.len(),
            "max": server.config().max_sessions,
            "states": {
                "idle": "the port is available for the next call",
                "busy": "an operation holds the port; another call would return SERIAL_SESSION_BUSY",
                "poisoned": "the port is gone (unplugged, or an operation panicked); close it and reopen",
            },
        }))
        .duration(started.elapsed());

    if poisoned > 0 {
        env = env.warn().remedy(format!(
            "{poisoned} session(s) are poisoned and will never work again. Call serial_close on \
             each, re-seat the adapter, then serial_open."
        ));
    }
    Ok(env)
}
