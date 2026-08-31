//! Closed error-code vocabulary and the crate's error type.
//!
//! [`ErrorCode`] is deliberately a closed enum: it lands in `structuredContent`
//! and in the declared output schema, so a model caller can branch on it without
//! string-matching prose that might be reworded later.

use std::io;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::envelope::{Envelope, NextAction};

/// Every machine-readable failure this server can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // --- paths / confinement -------------------------------------------------
    PathEscapesFirmwareRoot,
    PathNotFound,
    NotAFile,
    // --- toolchain / subprocess ---------------------------------------------
    ToolNotFound,
    ProcessSpawnFailed,
    ProcessTimeout,
    CompileFailed,
    HexValidationFailed,
    FlashFailed,
    UnsupportedChip,
    At89sNotSupported,
    // --- serial --------------------------------------------------------------
    PortHeldBySession,
    SerialOpenFailed,
    SerialSessionNotFound,
    SerialSessionExists,
    SerialSessionBusy,
    SerialSessionPoisoned,
    SerialIoError,
    PatternNotFound,
    TooManySessions,
    // --- validation / safety -------------------------------------------------
    InvalidArgument,
    SafetyBlocked,
    // --- catch-alls ----------------------------------------------------------
    IoError,
    InternalError,
}

impl ErrorCode {
    /// The exact wire spelling, for log lines and prose.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PathEscapesFirmwareRoot => "PATH_ESCAPES_FIRMWARE_ROOT",
            Self::PathNotFound => "PATH_NOT_FOUND",
            Self::NotAFile => "NOT_A_FILE",
            Self::ToolNotFound => "TOOL_NOT_FOUND",
            Self::ProcessSpawnFailed => "PROCESS_SPAWN_FAILED",
            Self::ProcessTimeout => "PROCESS_TIMEOUT",
            Self::CompileFailed => "COMPILE_FAILED",
            Self::HexValidationFailed => "HEX_VALIDATION_FAILED",
            Self::FlashFailed => "FLASH_FAILED",
            Self::UnsupportedChip => "UNSUPPORTED_CHIP",
            Self::At89sNotSupported => "AT89S_NOT_SUPPORTED",
            Self::PortHeldBySession => "PORT_HELD_BY_SESSION",
            Self::SerialOpenFailed => "SERIAL_OPEN_FAILED",
            Self::SerialSessionNotFound => "SERIAL_SESSION_NOT_FOUND",
            Self::SerialSessionExists => "SERIAL_SESSION_EXISTS",
            Self::SerialSessionBusy => "SERIAL_SESSION_BUSY",
            Self::SerialSessionPoisoned => "SERIAL_SESSION_POISONED",
            Self::SerialIoError => "SERIAL_IO_ERROR",
            Self::PatternNotFound => "PATTERN_NOT_FOUND",
            Self::TooManySessions => "TOO_MANY_SESSIONS",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::SafetyBlocked => "SAFETY_BLOCKED",
            Self::IoError => "IO_ERROR",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The crate's error type. Every variant carries enough context to rebuild a
/// self-contained [`Envelope`] without the call site restating anything.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("path `{path}` resolves to `{resolved}`, which is outside FIRMWARE_ROOT `{root}`")]
    PathEscapesRoot {
        path: String,
        resolved: String,
        root: String,
    },

    #[error("no such path: `{path}`")]
    PathNotFound { path: String },

    #[error("not a regular file: `{path}`")]
    NotAFile { path: String },

    #[error("`{tool}` was not found on PATH")]
    ToolNotFound { tool: String },

    #[error("could not spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },

    #[error("`{program}` exceeded its {timeout_ms} ms budget and was terminated")]
    ProcessTimeout { program: String, timeout_ms: u64 },

    #[error("sdcc failed with exit code {exit_code:?}")]
    CompileFailed { exit_code: Option<i32> },

    #[error("{message}")]
    HexInvalid { message: String, path: String },

    #[error("stcgal failed with exit code {exit_code:?}")]
    FlashFailed { exit_code: Option<i32> },

    #[error("unsupported chip `{chip}`; known chips are `stc` and `at89s`")]
    UnsupportedChip { chip: String },

    #[error("serial port `{port}` is held by this server's session `{session}`")]
    PortHeldBySession { port: String, session: String },

    #[error("could not open `{port}` at {baud} baud: {source}")]
    SerialOpen {
        port: String,
        baud: u32,
        #[source]
        source: io::Error,
    },

    #[error("no serial session named `{session}`")]
    SessionNotFound { session: String },

    #[error("a serial session named `{session}` is already open on `{port}`")]
    SessionExists { session: String, port: String },

    #[error("serial session `{session}` is already running another operation")]
    SessionBusy { session: String },

    #[error("serial session `{session}` is poisoned: {reason}")]
    SessionPoisoned { session: String, reason: String },

    #[error("{open} serial sessions are already open (max {max})")]
    TooManySessions { open: usize, max: usize },

    #[error("serial I/O failed on session `{session}`: {source}")]
    SerialIo {
        session: String,
        fatal: bool,
        #[source]
        source: io::Error,
    },

    #[error("pattern {pattern:?} did not appear within {timeout_ms} ms")]
    PatternNotFound {
        pattern: String,
        timeout_ms: u64,
        seen: String,
    },

    #[error("{message}")]
    InvalidArgument { message: String },

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },

    #[error("internal error: {message}")]
    Internal { message: String },
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }

    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Map an `io::ErrorKind` seen mid-session onto "the device is gone".
    ///
    /// A timeout is ordinary — the peer simply had nothing to say. A broken pipe
    /// or a vanished device node means the USB adapter was unplugged and the fd
    /// will never work again, so the slot must be poisoned rather than reused.
    pub fn is_fatal_serial_kind(kind: io::ErrorKind, raw: Option<i32>) -> bool {
        use io::ErrorKind as K;
        if matches!(
            kind,
            K::BrokenPipe | K::NotConnected | K::UnexpectedEof | K::NotFound | K::PermissionDenied
        ) {
            return true;
        }
        // ENXIO / ENODEV / EIO / EBADF — the device node went away underneath us.
        matches!(
            raw,
            Some(libc::ENXIO | libc::ENODEV | libc::EIO | libc::EBADF)
        )
    }

    pub fn serial_io(session: impl Into<String>, source: io::Error) -> Self {
        let fatal = Self::is_fatal_serial_kind(source.kind(), source.raw_os_error());
        Self::SerialIo {
            session: session.into(),
            fatal,
            source,
        }
    }

    /// If this error means the port is unusable from now on, the reason to record.
    pub fn poison_reason(&self) -> Option<String> {
        match self {
            Self::SerialIo {
                fatal: true,
                source,
                ..
            } => Some(format!("device error: {source}")),
            _ => None,
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::PathEscapesRoot { .. } => ErrorCode::PathEscapesFirmwareRoot,
            Self::PathNotFound { .. } => ErrorCode::PathNotFound,
            Self::NotAFile { .. } => ErrorCode::NotAFile,
            Self::ToolNotFound { .. } => ErrorCode::ToolNotFound,
            Self::Spawn { .. } => ErrorCode::ProcessSpawnFailed,
            Self::ProcessTimeout { .. } => ErrorCode::ProcessTimeout,
            Self::CompileFailed { .. } => ErrorCode::CompileFailed,
            Self::HexInvalid { .. } => ErrorCode::HexValidationFailed,
            Self::FlashFailed { .. } => ErrorCode::FlashFailed,
            Self::UnsupportedChip { .. } => ErrorCode::UnsupportedChip,
            Self::PortHeldBySession { .. } => ErrorCode::PortHeldBySession,
            Self::SerialOpen { .. } => ErrorCode::SerialOpenFailed,
            Self::SessionNotFound { .. } => ErrorCode::SerialSessionNotFound,
            Self::SessionExists { .. } => ErrorCode::SerialSessionExists,
            Self::SessionBusy { .. } => ErrorCode::SerialSessionBusy,
            Self::SessionPoisoned { .. } => ErrorCode::SerialSessionPoisoned,
            Self::TooManySessions { .. } => ErrorCode::TooManySessions,
            Self::SerialIo { .. } => ErrorCode::SerialIoError,
            Self::PatternNotFound { .. } => ErrorCode::PatternNotFound,
            Self::InvalidArgument { .. } => ErrorCode::InvalidArgument,
            Self::Io { .. } => ErrorCode::IoError,
            Self::Internal { .. } => ErrorCode::InternalError,
        }
    }

    /// A concrete next step for the caller. Never a restatement of the message.
    pub fn remedy(&self) -> Option<String> {
        Some(match self {
            Self::PathEscapesRoot { root, .. } => format!(
                "Pass a path inside `{root}`, or unset FIRMWARE_ROOT to lift confinement. \
                 Symlinks are resolved before the check, so a link out of the root is refused too."
            ),
            Self::PathNotFound { .. } => {
                "Check the spelling, or pass an absolute path. Relative paths resolve against \
                 FIRMWARE_ROOT when it is set, otherwise against the server's working directory."
                    .into()
            }
            Self::NotAFile { .. } => "Point at a file, not a directory.".into(),
            Self::ToolNotFound { tool } => match tool.as_str() {
                "sdcc" | "packihx" => {
                    "Install SDCC (`brew install sdcc`); it ships both `sdcc` and `packihx`.".into()
                }
                "stcgal" => {
                    "Install stcgal (`pipx install stcgal` or `pip3 install --user stcgal`) \
                             and make sure ~/.local/bin is on PATH."
                        .into()
                }
                other => format!("Install `{other}` and put it on PATH."),
            },
            Self::Spawn { program, .. } => {
                format!("Confirm `{program}` is executable and on PATH; run `doctor` to check.")
            }
            Self::ProcessTimeout { .. } => {
                "The child was sent SIGTERM, then SIGKILL, and reaped. Retry, or raise the \
                 matching MCS51_MCP_*_TIMEOUT_MS environment variable."
                    .into()
            }
            Self::CompileFailed { .. } => {
                "Read the sdcc diagnostics in `stderr` — they are verbatim and point at the \
                 offending line."
                    .into()
            }
            Self::HexInvalid { .. } => {
                "packihx always exits 0, so the .hex was validated by content and failed. \
                 Re-run `compile`; if it fails again the .ihx from sdcc is likely truncated."
                    .into()
            }
            Self::FlashFailed { .. } => {
                "STC parts only enter the bootloader during the first moments after power-up: \
                 start the flash, then power-cycle the board."
                    .into()
            }
            Self::UnsupportedChip { .. } => {
                "Pass chip=\"stc\" (STC89C52) or chip=\"at89s\".".into()
            }
            Self::PortHeldBySession { session, .. } => format!(
                "Close the session first: serial_close(session=\"{session}\"). \
                 A serial port cannot be flashed and talked to at the same time."
            ),
            Self::SerialOpen { .. } => {
                "Run `list_serial_ports` and use a /dev/cu.* path. /dev/tty.* blocks on DCD and \
                 will hang. Check the adapter is plugged in and no other program holds it."
                    .into()
            }
            Self::SessionNotFound { .. } => {
                "Call `serial_list_sessions` to see live ids, or `serial_open` to create one."
                    .into()
            }
            Self::SessionExists { .. } => {
                "Reuse that id, or close it first with `serial_close`.".into()
            }
            Self::SessionBusy { .. } => {
                "One operation per session at a time. Await the in-flight call before issuing \
                 another; the server refuses rather than queueing so it can never deadlock."
                    .into()
            }
            Self::SessionPoisoned { .. } => {
                "The port is unusable. Call `serial_close` on the session, re-seat the USB \
                 adapter, then `serial_open` again."
                    .into()
            }
            Self::TooManySessions { .. } => {
                "Close a session with `serial_close`, or raise MCS51_MCP_MAX_SESSIONS.".into()
            }
            Self::SerialIo { fatal: true, .. } => {
                "The adapter appears to have been unplugged. Re-seat it, `serial_close`, then \
                 `serial_open` again."
                    .into()
            }
            Self::SerialIo { .. } => "Retry; if it persists, close and reopen the session.".into(),
            Self::PatternNotFound { .. } => {
                "Check the firmware replies exactly what you expect (the line protocol answers \
                 PONG / OK / ERR / a value), or raise timeout_ms."
                    .into()
            }
            Self::InvalidArgument { .. } => return None,
            Self::Io { .. } => return None,
            Self::Internal { .. } => {
                "This is a bug in mcs51-mcp; the server's stderr log has the details.".into()
            }
        })
    }

    /// Structured detail that would otherwise only exist inside the message.
    pub fn data(&self) -> Value {
        match self {
            Self::PathEscapesRoot {
                path,
                resolved,
                root,
            } => json!({ "path": path, "resolved": resolved, "firmware_root": root }),
            Self::PathNotFound { path } | Self::NotAFile { path } => json!({ "path": path }),
            Self::ToolNotFound { tool } => json!({ "tool": tool }),
            Self::ProcessTimeout {
                program,
                timeout_ms,
            } => json!({ "program": program, "timeout_ms": timeout_ms }),
            Self::HexInvalid { path, .. } => json!({ "hex": path }),
            Self::UnsupportedChip { chip } => {
                json!({ "chip": chip, "supported": ["stc", "at89s"] })
            }
            Self::PortHeldBySession { port, session } => {
                json!({ "port": port, "session": session })
            }
            Self::SerialOpen { port, baud, .. } => json!({ "port": port, "baud": baud }),
            Self::SessionNotFound { session } | Self::SessionBusy { session } => {
                json!({ "session": session })
            }
            Self::SessionExists { session, port } => json!({ "session": session, "port": port }),
            Self::SessionPoisoned { session, reason } => {
                json!({ "session": session, "reason": reason })
            }
            Self::TooManySessions { open, max } => json!({ "open": open, "max": max }),
            Self::SerialIo { session, fatal, .. } => json!({ "session": session, "fatal": fatal }),
            Self::PatternNotFound {
                pattern,
                timeout_ms,
                seen,
            } => json!({ "pattern": pattern, "timeout_ms": timeout_ms, "received": seen }),
            _ => Value::Null,
        }
    }

    /// Tool calls that would plausibly unstick the caller.
    pub fn next_actions(&self) -> Vec<NextAction> {
        match self {
            Self::PortHeldBySession { session, .. } => vec![NextAction::call(
                crate::names::SERIAL_CLOSE,
                "Release the port before flashing it.",
                json!({ "session": session }),
            )],
            Self::SessionNotFound { .. } | Self::SessionBusy { .. } => vec![NextAction::call(
                crate::names::SERIAL_LIST_SESSIONS,
                "See which sessions exist and what state they are in.",
                json!({}),
            )],
            Self::SessionPoisoned { session, .. } | Self::SerialIo { session, .. } => {
                vec![NextAction::call(
                    crate::names::SERIAL_CLOSE,
                    "Discard the dead handle so the id can be reused.",
                    json!({ "session": session }),
                )]
            }
            Self::ToolNotFound { .. } | Self::Spawn { .. } => vec![NextAction::call(
                crate::names::DOCTOR,
                "Check which parts of the toolchain are actually installed.",
                json!({}),
            )],
            Self::SerialOpen { .. } => vec![NextAction::call(
                crate::names::LIST_SERIAL_PORTS,
                "List the ports that really exist, ranked with /dev/cu.* first.",
                json!({}),
            )],
            _ => Vec::new(),
        }
    }

    /// Render as a complete failure envelope for the named tool.
    pub fn into_envelope(self, tool: &str) -> Envelope {
        let code = self.code();
        let message = self.to_string();
        let mut env = Envelope::new(tool).error(code, message).data(self.data());
        if let Some(remedy) = self.remedy() {
            env = env.remedy(remedy);
        }
        for action in self.next_actions() {
            env = env.next_action(action);
        }
        env
    }
}
