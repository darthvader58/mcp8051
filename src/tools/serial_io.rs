//! `serial_write`, `serial_read`, `serial_expect`.

use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::envelope::{Envelope, NextAction};
use crate::errors::AppError;
use crate::names;
use crate::serial::ops;
use crate::server::Server;

/// Default read/expect window when the caller does not say.
const DEFAULT_TIMEOUT_MS: u64 = 1000;

/// Render bytes for JSON: valid UTF-8 wins, otherwise show the escapes rather
/// than silently mangling a byte the caller may care about.
fn render(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), true),
        Err(_) => (
            bytes
                .iter()
                .map(|b| {
                    if b.is_ascii_graphic() || *b == b' ' {
                        (*b as char).to_string()
                    } else {
                        format!("\\x{b:02x}")
                    }
                })
                .collect(),
            false,
        ),
    }
}

fn lines_of(text: &str) -> Vec<String> {
    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

// -------------------------------------------------------------- serial_write

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteArgs {
    /// Id of an open session.
    pub session: String,
    /// Text to send. A trailing newline is appended if absent — the firmware's
    /// line protocol only acts on a complete line.
    pub data: String,
}

pub async fn write(server: &Server, args: WriteArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();

    // The firmware parses on newline. Sending "PING" without one leaves the
    // board waiting and the caller staring at an empty read.
    let appended = !args.data.ends_with('\n');
    let mut payload = args.data.clone();
    if appended {
        payload.push('\n');
    }
    let bytes = payload.into_bytes();
    let len = bytes.len();

    let written = server
        .sessions
        .with_port(&args.session, ops::write(args.session.clone(), bytes))
        .await?;

    Ok(Envelope::new(names::SERIAL_WRITE)
        .data(json!({
            "session": args.session,
            "sent": args.data,
            "bytes_written": written,
            "newline_appended": appended,
            "total_bytes": len,
        }))
        .duration(started.elapsed())
        .next_action(NextAction::call(
            names::SERIAL_EXPECT,
            "Wait for the reply rather than guessing how long it takes.",
            json!({ "session": args.session, "pattern": "OK", "timeout_ms": DEFAULT_TIMEOUT_MS }),
        )))
}

// --------------------------------------------------------------- serial_read

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    /// Id of an open session.
    pub session: String,
    /// How long to wait for data, in milliseconds. Default 1000.
    pub timeout_ms: Option<u64>,
}

pub async fn read(server: &Server, args: ReadArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let timeout_ms = args.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let cap = server.config().serial_read_cap;

    let out = server
        .sessions
        .with_port(
            &args.session,
            ops::read(args.session.clone(), Duration::from_millis(timeout_ms), cap),
        )
        .await?;

    let (text, is_utf8) = render(&out.bytes);
    let mut env = Envelope::new(names::SERIAL_READ)
        .data(json!({
            "session": args.session,
            "text": text,
            "lines": lines_of(&text),
            "bytes": out.bytes.len(),
            "valid_utf8": is_utf8,
            "truncated": out.truncated,
            "from_buffer": out.used_buffer,
            "timeout_ms": timeout_ms,
            "note": format!(
                "Returns as soon as {} ms pass with no new bytes, so a fast reply does not \
                 cost the whole window.",
                ops::IDLE_GAP.as_millis()
            ),
        }))
        .duration(started.elapsed());

    if out.bytes.is_empty() {
        env = env.warn().remedy(
            "Nothing arrived. Check the board is powered and running the reference firmware, \
             that TX/RX are crossed, that grounds are common, and that the baud matches (9600 \
             for an 11.0592 MHz crystal).",
        );
    }
    Ok(env)
}

// ------------------------------------------------------------- serial_expect

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExpectArgs {
    /// Id of an open session.
    pub session: String,
    /// Literal substring to wait for — not a regular expression.
    pub pattern: String,
    /// How long to wait, in milliseconds. Default 1000.
    pub timeout_ms: Option<u64>,
}

pub async fn expect(server: &Server, args: ExpectArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let timeout_ms = args.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let cap = server.config().serial_read_cap;

    if args.pattern.is_empty() {
        return Err(AppError::invalid(
            "pattern must not be empty; it is matched as a literal substring",
        ));
    }

    let out = server
        .sessions
        .with_port(
            &args.session,
            ops::expect(
                args.session.clone(),
                args.pattern.clone(),
                Duration::from_millis(timeout_ms),
                cap,
            ),
        )
        .await?;

    let (text, is_utf8) = render(&out.bytes);
    Ok(Envelope::new(names::SERIAL_EXPECT)
        .data(json!({
            "session": args.session,
            "pattern": args.pattern,
            "matched": true,
            "match_offset": out.at,
            "waited_ms": out.waited.as_millis() as u64,
            "timeout_ms": timeout_ms,
            "text": text,
            "lines": lines_of(&text),
            "valid_utf8": is_utf8,
            "note": "Returns the instant the pattern completes; any bytes after the match stay \
                     buffered for the next serial_read or serial_expect.",
        }))
        .duration(started.elapsed()))
}
