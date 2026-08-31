//! Session state and the port abstraction.
//!
//! The registry talks to [`SerialLink`] rather than to `serialport` directly.
//! That is one virtual call per I/O op and it buys hardware-free tests: the
//! whole checkout / check-in / poison state machine can be exercised against a
//! scripted fake, including failure modes (a panicking op, a device that
//! reports `ENXIO` mid-read) that are impossible to stage with real hardware.

use std::io;
use std::time::{Duration, Instant};

use serde::Serialize;

/// The minimum a serial port has to do for this server.
pub trait SerialLink: Send {
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()>;
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

impl SerialLink for Box<dyn serialport::SerialPort> {
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        serialport::SerialPort::set_timeout(self.as_mut(), timeout).map_err(io::Error::from)
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(self, buf)
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        io::Write::write_all(self, buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(self)
    }
}

/// Bytes moved by one operation, folded into the session's totals on check-in.
#[derive(Debug, Default, Clone, Copy)]
pub struct IoStats {
    pub read: u64,
    pub written: u64,
}

/// Where the port handle is right now.
///
/// The handle is *moved out* of the map for the duration of an operation.
/// That is what makes "one op at a time" a property of the data structure
/// instead of a convention: a second caller finds `Busy` and cannot get a
/// second `&mut` to the same fd even in principle.
pub enum PortSlot {
    /// Available. The caller may take it.
    Idle(Box<dyn SerialLink>),
    /// Checked out since this instant.
    Busy { since: Instant },
    /// Permanently unusable; the handle has already been dropped.
    Poisoned { reason: String, at: Instant },
}

impl PortSlot {
    pub fn state_name(&self) -> &'static str {
        match self {
            Self::Idle(_) => "idle",
            Self::Busy { .. } => "busy",
            Self::Poisoned { .. } => "poisoned",
        }
    }
}

impl std::fmt::Debug for PortSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle(_) => f.write_str("Idle(<port>)"),
            Self::Busy { since } => write!(f, "Busy {{ for {:?} }}", since.elapsed()),
            Self::Poisoned { reason, .. } => write!(f, "Poisoned({reason})"),
        }
    }
}

/// One open serial connection, keyed by a caller-chosen id.
pub struct Session {
    pub id: String,
    /// Distinguishes this session from a *different* one that later reuses the
    /// same id. An in-flight operation captures it at checkout and compares on
    /// check-in; without it, a slow op returning after its session was closed
    /// and the id reopened would overwrite the new session's live port with its
    /// own stale handle.
    pub generation: u64,
    pub port: String,
    pub baud: u32,
    pub opened_at: Instant,
    pub bytes_read: u64,
    pub bytes_written: u64,
    /// Set by `serial_close` while an op is in flight. The check-in honours it
    /// and drops the handle instead of returning it to the map.
    pub close_requested: bool,
    pub slot: PortSlot,
    /// Bytes received but not yet consumed — for instance whatever followed the
    /// match in a `serial_expect`. Kept so the next read does not lose them.
    pub pending: Vec<u8>,
}

impl Session {
    pub fn new(
        id: String,
        generation: u64,
        port: String,
        baud: u32,
        link: Box<dyn SerialLink>,
    ) -> Self {
        Self {
            id,
            generation,
            port,
            baud,
            opened_at: Instant::now(),
            bytes_read: 0,
            bytes_written: 0,
            close_requested: false,
            slot: PortSlot::Idle(link),
            pending: Vec::new(),
        }
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            session: self.id.clone(),
            port: self.port.clone(),
            baud: self.baud,
            age_ms: self.opened_at.elapsed().as_millis() as u64,
            bytes_read: self.bytes_read,
            bytes_written: self.bytes_written,
            buffered_bytes: self.pending.len() as u64,
            state: self.slot.state_name(),
            detail: match &self.slot {
                PortSlot::Busy { since } => {
                    Some(format!("busy for {} ms", since.elapsed().as_millis()))
                }
                PortSlot::Poisoned { reason, at } => Some(format!(
                    "poisoned {} ms ago: {reason}",
                    at.elapsed().as_millis()
                )),
                PortSlot::Idle(_) => None,
            },
            close_requested: self.close_requested,
        }
    }
}

/// Serializable snapshot for `serial_list_sessions`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session: String,
    pub port: String,
    pub baud: u32,
    pub age_ms: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub buffered_bytes: u64,
    /// `idle` / `busy` / `poisoned`.
    pub state: &'static str,
    pub detail: Option<String>,
    pub close_requested: bool,
}
