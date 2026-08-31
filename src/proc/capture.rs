//! Bounded output capture.
//!
//! Capping output means *keep reading and throw away the middle*, never *stop
//! reading*. A reader that stops fills the 64 KiB pipe buffer, at which point
//! the child blocks forever inside `write()` and the timeout has to kill a
//! process that was only trying to talk to us.
//!
//! Head and tail are both kept because they answer different questions: the
//! head holds the first compiler error (the one that caused all the others),
//! the tail holds a flasher's verdict.

use std::collections::VecDeque;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::Instant;

use crate::envelope::Tail;

/// A fixed-memory head+tail accumulator.
pub struct CapBuf {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    total: u64,
}

impl CapBuf {
    /// `cap` is the total number of bytes retained, split evenly.
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(64);
        let head_cap = cap / 2;
        Self {
            head: Vec::with_capacity(head_cap.min(8192)),
            tail: VecDeque::new(),
            head_cap,
            tail_cap: cap - head_cap,
            total: 0,
        }
    }

    pub fn push(&mut self, mut bytes: &[u8]) {
        self.total += bytes.len() as u64;

        if self.head.len() < self.head_cap {
            let take = (self.head_cap - self.head.len()).min(bytes.len());
            self.head.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
        if bytes.is_empty() || self.tail_cap == 0 {
            return;
        }
        // Only the last `tail_cap` bytes of the incoming chunk can survive.
        let start = bytes.len().saturating_sub(self.tail_cap);
        self.tail.extend(&bytes[start..]);
        while self.tail.len() > self.tail_cap {
            self.tail.pop_front();
        }
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    /// Materialize. Splitting on byte boundaries can cut a UTF-8 sequence, so
    /// the halves are decoded lossily rather than dropped.
    pub fn finish(self) -> Tail {
        let kept = self.head.len() as u64 + self.tail.len() as u64;
        let elided = self.total.saturating_sub(kept);
        let truncated = elided > 0;

        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if truncated {
            text.push_str(&format!("\n...[{elided} bytes elided]...\n"));
        }
        if !self.tail.is_empty() {
            let tail: Vec<u8> = self.tail.into_iter().collect();
            text.push_str(&String::from_utf8_lossy(&tail));
        }

        Tail {
            text,
            total_bytes: self.total,
            elided_bytes: elided,
            truncated,
        }
    }
}

/// Drain a pipe to EOF or `deadline`, keeping at most `cap` bytes.
///
/// Returns whatever was captured on either path, so a child killed by the
/// timeout still reports the diagnostics it managed to print. The deadline is
/// what stops a grandchild holding the write end open from hanging the call.
pub async fn drain<R>(mut reader: R, cap: usize, deadline: Instant) -> Tail
where
    R: AsyncRead + Unpin,
{
    let mut buf = CapBuf::new(cap);
    let mut chunk = vec![0u8; 8192];
    loop {
        match tokio::time::timeout_at(deadline, reader.read(&mut chunk)).await {
            // EOF.
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => buf.push(&chunk[..n]),
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    buf.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_is_kept_verbatim() {
        let mut b = CapBuf::new(1024);
        b.push(b"hello world");
        let t = b.finish();
        assert_eq!(t.text, "hello world");
        assert!(!t.truncated);
        assert_eq!(t.elided_bytes, 0);
    }

    #[test]
    fn long_output_keeps_both_ends_and_reports_the_gap() {
        let mut b = CapBuf::new(64);
        let data: Vec<u8> = (0..10_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        b.push(&data);
        let t = b.finish();
        assert!(t.truncated);
        assert_eq!(t.total_bytes, 10_000);
        assert_eq!(t.elided_bytes, 10_000 - 64);
        assert!(t.text.contains("bytes elided"));
        // Head is the true head, tail the true tail.
        assert!(t.text.starts_with("abcdefg"));
        let end = String::from_utf8_lossy(&data[data.len() - 8..]).into_owned();
        assert!(t.text.ends_with(&end));
        // Memory is bounded regardless of input size.
        assert!(t.text.len() < 200);
    }

    #[test]
    fn many_small_pushes_behave_like_one_big_one() {
        let mut b = CapBuf::new(32);
        for i in 0..1000u32 {
            b.push(format!("{i}").as_bytes());
        }
        let t = b.finish();
        assert!(t.truncated);
        assert!(t.text.starts_with('0'));
        assert!(t.text.ends_with("999"));
    }
}
