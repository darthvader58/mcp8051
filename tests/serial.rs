//! The serial session state machine, exercised against a fake port.
//!
//! The port sits behind a trait precisely so these cases can be staged: a
//! panicking operation, a device that vanishes mid-read, a close that arrives
//! while an operation is in flight, and — the one that motivates the whole
//! design — a caller whose future is dropped part-way through.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mcs51_mcp::errors::ErrorCode;
use mcs51_mcp::serial::{SerialLink, SessionRegistry};

/// A scripted serial port.
#[derive(Default)]
struct FakeState {
    /// Bytes handed out by successive reads.
    to_read: Vec<Vec<u8>>,
    read_idx: usize,
    written: Vec<u8>,
    /// Fail every read with this kind once `fail_after` reads have happened.
    fail_with: Option<io::ErrorKind>,
    fail_after: usize,
    reads: usize,
    panic_on_read: bool,
    /// Sleep this long inside each read, to stage overlapping operations.
    delay: Duration,
    /// Set on drop, so a test can prove the fd was released.
    dropped: Option<Arc<AtomicUsize>>,
}

struct FakePort(Arc<Mutex<FakeState>>);

impl Drop for FakePort {
    fn drop(&mut self) {
        let st = self.0.lock().unwrap();
        if let Some(flag) = &st.dropped {
            flag.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl SerialLink for FakePort {
    fn set_timeout(&mut self, _t: Duration) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (delay, panic_now, fail, chunk) = {
            let mut st = self.0.lock().unwrap();
            st.reads += 1;
            let fail = if st.reads > st.fail_after {
                st.fail_with
            } else {
                None
            };
            let chunk = if st.read_idx < st.to_read.len() {
                let c = st.to_read[st.read_idx].clone();
                st.read_idx += 1;
                Some(c)
            } else {
                None
            };
            (st.delay, st.panic_on_read, fail, chunk)
        };

        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if panic_now {
            panic!("staged panic inside a blocking serial op");
        }
        if let Some(kind) = fail {
            return Err(io::Error::from(kind));
        }
        match chunk {
            Some(c) => {
                let n = c.len().min(buf.len());
                buf[..n].copy_from_slice(&c[..n]);
                Ok(n)
            }
            // Nothing scripted: behave like a quiet line.
            None => Err(io::Error::from(io::ErrorKind::TimedOut)),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.lock().unwrap().written.extend_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Build a registry whose ports are all clones of one scripted state.
fn registry_with(state: Arc<Mutex<FakeState>>) -> SessionRegistry {
    SessionRegistry::with_opener(
        8,
        Arc::new(move |_path: &str, _baud: u32, _t: Duration| {
            Ok(Box::new(FakePort(Arc::clone(&state))) as Box<dyn SerialLink>)
        }),
    )
}

fn open(reg: &SessionRegistry, id: &str) {
    reg.open(id, "/dev/cu.fake", 9600, Duration::from_millis(50))
        .expect("fake open should succeed");
}

fn state_of(reg: &SessionRegistry, id: &str) -> String {
    reg.list()
        .into_iter()
        .find(|s| s.session == id)
        .map(|s| s.state.to_string())
        .unwrap_or_else(|| "<gone>".to_string())
}

#[tokio::test]
async fn a_successful_operation_returns_the_port_to_idle() {
    let st = Arc::new(Mutex::new(FakeState {
        to_read: vec![b"PONG\r\n".to_vec()],
        ..Default::default()
    }));
    let reg = registry_with(Arc::clone(&st));
    open(&reg, "board");

    let out = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::expect(
                "board".into(),
                "PONG".into(),
                Duration::from_millis(500),
                4096,
            ),
        )
        .await
        .expect("expect should match");

    assert_eq!(out.at, 0);
    assert_eq!(state_of(&reg, "board"), "idle");

    let info = reg.list().into_iter().next().unwrap();
    assert_eq!(info.bytes_read, 6);
    assert_eq!(info.port, "/dev/cu.fake");
    assert_eq!(info.baud, 9600);
}

#[tokio::test]
async fn writes_are_accounted_and_reach_the_port() {
    let st = Arc::new(Mutex::new(FakeState::default()));
    let reg = registry_with(Arc::clone(&st));
    open(&reg, "board");

    reg.with_port(
        "board",
        mcs51_mcp::serial::ops::write("board".into(), b"PING\n".to_vec()),
    )
    .await
    .unwrap();

    assert_eq!(st.lock().unwrap().written, b"PING\n");
    assert_eq!(reg.list()[0].bytes_written, 5);
}

#[tokio::test]
async fn bytes_after_a_match_stay_buffered_for_the_next_call() {
    let st = Arc::new(Mutex::new(FakeState {
        to_read: vec![b"OK\r\nLEFTOVER".to_vec()],
        ..Default::default()
    }));
    let reg = registry_with(Arc::clone(&st));
    open(&reg, "board");

    reg.with_port(
        "board",
        mcs51_mcp::serial::ops::expect("board".into(), "OK".into(), Duration::from_secs(1), 4096),
    )
    .await
    .unwrap();

    // The tail of the line is not thrown away.
    assert_eq!(reg.list()[0].buffered_bytes, 10);

    let out = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(50), 4096),
        )
        .await
        .unwrap();
    assert_eq!(out.bytes, b"\r\nLEFTOVER");
    assert!(out.used_buffer);
}

#[tokio::test]
async fn expect_returns_the_instant_it_matches_not_at_the_deadline() {
    let st = Arc::new(Mutex::new(FakeState {
        to_read: vec![b"PONG\n".to_vec()],
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    let started = std::time::Instant::now();
    reg.with_port(
        "board",
        mcs51_mcp::serial::ops::expect(
            "board".into(),
            "PONG".into(),
            Duration::from_secs(10),
            4096,
        ),
    )
    .await
    .unwrap();

    assert!(
        started.elapsed() < Duration::from_millis(500),
        "expect burned {:?} of a 10s window on a match that arrived immediately",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_missed_pattern_reports_what_did_arrive() {
    let st = Arc::new(Mutex::new(FakeState {
        to_read: vec![b"ERR\n".to_vec()],
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    let err = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::expect(
                "board".into(),
                "PONG".into(),
                Duration::from_millis(200),
                4096,
            ),
        )
        .await
        .expect_err("PONG never arrives");

    assert_eq!(err.code(), ErrorCode::PatternNotFound);
    assert!(err.to_string().contains("PONG"));
    assert_eq!(err.data()["received"], "ERR\n");
    // A miss is not a poisoning: the port is still usable.
    assert_eq!(state_of(&reg, "board"), "idle");
}

#[tokio::test]
async fn a_second_operation_on_a_busy_session_is_refused_immediately() {
    let st = Arc::new(Mutex::new(FakeState {
        delay: Duration::from_millis(400),
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    let slow = {
        let reg = reg.clone();
        tokio::spawn(async move {
            reg.with_port(
                "board",
                mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(800), 4096),
            )
            .await
        })
    };

    // Let the first operation take the port.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state_of(&reg, "board"), "busy");

    let started = std::time::Instant::now();
    let err = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(50), 4096),
        )
        .await
        .expect_err("a concurrent op must be refused");

    assert_eq!(err.code(), ErrorCode::SerialSessionBusy);
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "the refusal must be immediate, not queued behind the running op"
    );

    let _ = slow.await.unwrap();
    assert_eq!(state_of(&reg, "board"), "idle");
}

#[tokio::test]
async fn a_cancelled_caller_does_not_brick_the_session() {
    // The reason the check-in lives inside a detached `tokio::spawn`. Here the
    // caller's future is dropped at its await point — exactly what a cancelled
    // MCP request does. The blocking op cannot be aborted, so if the put-back
    // rode on the caller's future the slot would stay `Busy` forever and one
    // Ctrl-C would kill the session permanently.
    let st = Arc::new(Mutex::new(FakeState {
        delay: Duration::from_millis(300),
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    {
        let fut = reg.with_port(
            "board",
            mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(500), 4096),
        );
        // Give it long enough to check the port out, then drop it.
        let cancelled = tokio::time::timeout(Duration::from_millis(80), fut).await;
        assert!(cancelled.is_err(), "the caller should have been cancelled");
    }
    assert_eq!(state_of(&reg, "board"), "busy");

    // The detached task finishes on its own and puts the port back.
    for _ in 0..100 {
        if state_of(&reg, "board") == "idle" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        state_of(&reg, "board"),
        "idle",
        "the port was never checked back in after the caller was cancelled"
    );

    // And it is genuinely usable again, not merely labelled idle.
    reg.with_port(
        "board",
        mcs51_mcp::serial::ops::write("board".into(), b"PING\n".to_vec()),
    )
    .await
    .expect("the session should work after a cancelled call");
}

#[tokio::test]
async fn a_panicking_operation_poisons_the_session_rather_than_wedging_it() {
    let st = Arc::new(Mutex::new(FakeState {
        panic_on_read: true,
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    let err = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(200), 4096),
        )
        .await
        .expect_err("a panic must surface as an error, not unwind the server");

    assert_eq!(err.code(), ErrorCode::SerialSessionPoisoned);
    assert_eq!(state_of(&reg, "board"), "poisoned");

    // Poisoned is terminal: further ops are refused, not retried against a
    // handle that was dropped by the unwind.
    let err = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::write("board".into(), b"x\n".to_vec()),
        )
        .await
        .expect_err("a poisoned session must stay refused");
    assert_eq!(err.code(), ErrorCode::SerialSessionPoisoned);

    // But it can still be closed and the id reused.
    reg.close("board")
        .expect("a poisoned session can be closed");
    assert_eq!(state_of(&reg, "board"), "<gone>");
}

#[tokio::test]
async fn an_unplugged_adapter_poisons_but_a_timeout_does_not() {
    // A timeout is ordinary: the board simply had nothing to say.
    let quiet = Arc::new(Mutex::new(FakeState {
        fail_with: Some(io::ErrorKind::TimedOut),
        ..Default::default()
    }));
    let reg = registry_with(quiet);
    open(&reg, "board");
    let out = reg
        .with_port(
            "board",
            mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(120), 4096),
        )
        .await
        .expect("a quiet line is not an error");
    assert!(out.bytes.is_empty());
    assert_eq!(state_of(&reg, "board"), "idle");

    // A vanished device node is not.
    for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::NotConnected] {
        let gone = Arc::new(Mutex::new(FakeState {
            fail_with: Some(kind),
            ..Default::default()
        }));
        let reg = registry_with(gone);
        open(&reg, "board");
        let err = reg
            .with_port(
                "board",
                mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(200), 4096),
            )
            .await
            .expect_err("an unplugged adapter must be an error");
        assert_eq!(err.code(), ErrorCode::SerialIoError, "for {kind:?}");
        assert_eq!(state_of(&reg, "board"), "poisoned", "for {kind:?}");
    }
}

#[tokio::test]
async fn closing_a_busy_session_is_deferred_and_then_really_happens() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let st = Arc::new(Mutex::new(FakeState {
        delay: Duration::from_millis(250),
        dropped: Some(Arc::clone(&dropped)),
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    let running = {
        let reg = reg.clone();
        tokio::spawn(async move {
            reg.with_port(
                "board",
                mcs51_mcp::serial::ops::read("board".into(), Duration::from_millis(600), 4096),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (info, deferred) = reg.close("board").expect("close should be accepted");
    assert!(deferred, "closing a busy session must be deferred");
    assert!(info.close_requested);
    // Still listed, because the port is not ours to drop yet.
    assert_eq!(state_of(&reg, "board"), "busy");

    let _ = running.await.unwrap();

    // The check-in honoured the close: entry gone, handle dropped, no orphan fd.
    assert_eq!(state_of(&reg, "board"), "<gone>");
    assert_eq!(reg.count(), 0);
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        1,
        "the port handle must be dropped, not leaked"
    );

    // And the id is free again.
    assert_eq!(
        reg.with_port(
            "board",
            mcs51_mcp::serial::ops::write("board".into(), b"x\n".to_vec())
        )
        .await
        .expect_err("the session is gone")
        .code(),
        ErrorCode::SerialSessionNotFound
    );
}

#[tokio::test]
async fn closing_an_idle_session_drops_the_handle_at_once() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let st = Arc::new(Mutex::new(FakeState {
        dropped: Some(Arc::clone(&dropped)),
        ..Default::default()
    }));
    let reg = registry_with(st);
    open(&reg, "board");

    let (_info, deferred) = reg.close("board").unwrap();
    assert!(!deferred);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(reg.count(), 0);
}

#[tokio::test]
async fn session_bookkeeping_is_enforced() {
    let st = Arc::new(Mutex::new(FakeState::default()));
    let reg = SessionRegistry::with_opener(
        2,
        Arc::new(move |_p: &str, _b: u32, _t: Duration| {
            Ok(Box::new(FakePort(Arc::clone(&st))) as Box<dyn SerialLink>)
        }),
    );

    open(&reg, "a");
    // A duplicate id is refused rather than silently replacing a live port.
    assert_eq!(
        reg.open("a", "/dev/cu.fake", 9600, Duration::from_millis(50))
            .unwrap_err()
            .code(),
        ErrorCode::SerialSessionExists
    );

    open(&reg, "b");
    assert_eq!(
        reg.open("c", "/dev/cu.fake", 9600, Duration::from_millis(50))
            .unwrap_err()
            .code(),
        ErrorCode::TooManySessions
    );

    // `flash` uses this to refuse a port we already hold.
    assert_eq!(reg.holder_of("/dev/cu.fake").as_deref(), Some("a"));
    assert_eq!(reg.holder_of("/dev/cu.other"), None);

    assert_eq!(
        reg.close("nope").unwrap_err().code(),
        ErrorCode::SerialSessionNotFound
    );
}

#[tokio::test]
async fn operations_on_an_unknown_session_are_a_clean_error() {
    let reg = registry_with(Arc::new(Mutex::new(FakeState::default())));
    let err = reg
        .with_port(
            "ghost",
            mcs51_mcp::serial::ops::read("ghost".into(), Duration::from_millis(50), 4096),
        )
        .await
        .expect_err("no such session");
    assert_eq!(err.code(), ErrorCode::SerialSessionNotFound);
}
