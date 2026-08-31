//! The blocking operations handed to [`crate::serial::registry::SessionRegistry::with_port`].
//!
//! Each one sets a short timeout slice on the port and loops to a deadline, so
//! the blocking work is bounded *by construction*. A `spawn_blocking` task
//! cannot be aborted; the only way to guarantee the thread comes back is for
//! the closure to be unable to block longer than its own budget.

use std::time::{Duration, Instant};

use crate::errors::AppError;
use crate::serial::session::{IoStats, SerialLink};

/// How long a single `read()` may block before the loop re-checks the deadline.
pub const IO_SLICE: Duration = Duration::from_millis(50);

/// Once some bytes have arrived, this much silence is treated as end-of-reply.
/// Without it every `serial_read` would burn its whole window even though the
/// board answered in 2 ms.
pub const IDLE_GAP: Duration = Duration::from_millis(120);

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
    )
}

/// Write `data`, then flush.
pub fn write(
    session: String,
    data: Vec<u8>,
) -> impl FnOnce(&mut dyn SerialLink, &mut Vec<u8>, &mut IoStats) -> Result<usize, AppError>
       + Send
       + 'static {
    move |link, _pending, stats| {
        link.set_timeout(IO_SLICE.max(Duration::from_millis(200)))
            .map_err(|e| AppError::serial_io(session.clone(), e))?;
        link.write_all(&data)
            .map_err(|e| AppError::serial_io(session.clone(), e))?;
        link.flush()
            .map_err(|e| AppError::serial_io(session.clone(), e))?;
        stats.written += data.len() as u64;
        Ok(data.len())
    }
}

/// Read whatever arrives before `timeout`, returning early after [`IDLE_GAP`]
/// of silence once something has been received.
pub fn read(
    session: String,
    timeout: Duration,
    cap: usize,
) -> impl FnOnce(&mut dyn SerialLink, &mut Vec<u8>, &mut IoStats) -> Result<ReadOut, AppError>
       + Send
       + 'static {
    move |link, pending, stats| {
        let deadline = Instant::now() + timeout;
        // Anything buffered from a previous expect is part of this read.
        let mut got = std::mem::take(pending);
        let had_buffered = !got.is_empty();
        let mut last_data = Instant::now();
        let mut chunk = [0u8; 4096];

        link.set_timeout(IO_SLICE)
            .map_err(|e| AppError::serial_io(session.clone(), e))?;

        loop {
            if got.len() >= cap {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            match link.read(&mut chunk) {
                Ok(0) => {}
                Ok(n) => {
                    got.extend_from_slice(&chunk[..n]);
                    stats.read += n as u64;
                    last_data = Instant::now();
                }
                Err(e) if is_timeout(&e) => {}
                Err(e) => {
                    *pending = got;
                    return Err(AppError::serial_io(session.clone(), e));
                }
            }
            if !got.is_empty() && last_data.elapsed() >= IDLE_GAP {
                break;
            }
        }

        let truncated = got.len() > cap;
        if truncated {
            // Keep the overflow buffered rather than discarding it.
            *pending = got.split_off(cap);
        }
        Ok(ReadOut {
            bytes: got,
            truncated,
            used_buffer: had_buffered,
        })
    }
}

#[derive(Debug)]
pub struct ReadOut {
    pub bytes: Vec<u8>,
    pub truncated: bool,
    /// True when part of the result came from bytes buffered by an earlier call.
    pub used_buffer: bool,
}

#[derive(Debug)]
pub struct ExpectOut {
    /// Everything received up to and including the match.
    pub bytes: Vec<u8>,
    /// Byte offset where `pattern` starts.
    pub at: usize,
    pub waited: Duration,
}

/// Wait for `pattern` to appear as a literal substring.
///
/// Returns **the instant the pattern completes**, not at the end of the window.
/// A `PING` round-trip at 9600 baud takes about 6 ms; burning a 1000 ms budget
/// on a success would make every serial interaction feel broken.
pub fn expect(
    session: String,
    pattern: String,
    timeout: Duration,
    cap: usize,
) -> impl FnOnce(&mut dyn SerialLink, &mut Vec<u8>, &mut IoStats) -> Result<ExpectOut, AppError>
       + Send
       + 'static {
    move |link, pending, stats| {
        let started = Instant::now();
        let deadline = started + timeout;
        let needle = pattern.as_bytes();
        let mut got = std::mem::take(pending);
        let mut chunk = [0u8; 4096];

        // The answer may already be sitting in the buffer from a previous call,
        // in which case this costs no I/O at all.
        let mut search_from = 0usize;
        if let Some(at) = find(&got, needle, 0) {
            let rest = got.split_off(at + needle.len());
            *pending = rest;
            return Ok(ExpectOut {
                bytes: got,
                at,
                waited: started.elapsed(),
            });
        }

        link.set_timeout(IO_SLICE)
            .map_err(|e| AppError::serial_io(session.clone(), e))?;

        loop {
            if Instant::now() >= deadline {
                break;
            }
            match link.read(&mut chunk) {
                Ok(0) => continue,
                Ok(n) => {
                    let before = got.len();
                    got.extend_from_slice(&chunk[..n]);
                    stats.read += n as u64;

                    // Only rescan from just before the new bytes: a match can
                    // straddle the chunk boundary by at most needle.len()-1.
                    let from = before.saturating_sub(needle.len().saturating_sub(1));
                    if let Some(at) = find(&got, needle, from.max(search_from)) {
                        let rest = got.split_off(at + needle.len());
                        *pending = rest;
                        return Ok(ExpectOut {
                            bytes: got,
                            at,
                            waited: started.elapsed(),
                        });
                    }
                    search_from = 0;
                    if got.len() > cap {
                        // Keep the tail: a pattern can only match against the
                        // most recent bytes anyway.
                        let drop_to = got.len() - cap;
                        got.drain(..drop_to);
                    }
                }
                Err(e) if is_timeout(&e) => continue,
                Err(e) => {
                    *pending = got;
                    return Err(AppError::serial_io(session.clone(), e));
                }
            }
        }

        let seen = String::from_utf8_lossy(&got).into_owned();
        // Leave what arrived buffered; a follow-up read should still see it.
        *pending = got;
        Err(AppError::PatternNotFound {
            pattern,
            timeout_ms: timeout.as_millis() as u64,
            seen,
        })
    }
}

/// Naive substring search. Patterns here are a handful of bytes (`OK`, `PONG`),
/// so anything cleverer would be slower.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let start = from.min(haystack.len().saturating_sub(needle.len()));
    haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + start)
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn substring_search_handles_offsets_and_misses() {
        assert_eq!(find(b"hello PONG\r\n", b"PONG", 0), Some(6));
        assert_eq!(find(b"hello PONG", b"PING", 0), None);
        assert_eq!(find(b"aaa", b"", 0), None);
        assert_eq!(find(b"ab", b"abc", 0), None);
        // A `from` past the last possible start still finds a trailing match.
        assert_eq!(find(b"xxOK", b"OK", 99), Some(2));
    }
}
