//! The subprocess runner's two hard guarantees:
//!
//! 1. An overrunning child is killed **and reaped** — no orphan, no zombie
//!    still holding a serial port.
//! 2. Capping output means discarding the middle while continuing to read.
//!    A capped reader that stops reading fills the pipe buffer and wedges the
//!    child; a capped reader that buffers everything runs the host out of
//!    memory. Neither is acceptable.

mod common;

use std::time::Duration;

use mcs51_mcp::proc::{run, RunSpec};

#[tokio::test]
async fn a_quick_child_reports_its_output_and_status() {
    let out = run(RunSpec::new("echo", Duration::from_secs(5)).arg("hello 8051"))
        .await
        .expect("echo should run");

    assert!(out.success());
    assert_eq!(out.exit_code, Some(0));
    assert!(!out.timed_out);
    assert_eq!(out.stdout.text.trim(), "hello 8051");
    assert!(!out.stdout.truncated);
    // The display string is quoted for a human, and is not what was executed.
    assert_eq!(out.command, "echo 'hello 8051'");
}

#[tokio::test]
async fn a_nonzero_exit_is_reported_not_raised() {
    let out = run(RunSpec::new("false", Duration::from_secs(5)))
        .await
        .expect("false should run");
    assert!(!out.success());
    assert_eq!(out.exit_code, Some(1));
}

#[tokio::test]
async fn a_missing_program_is_a_clean_error() {
    let err = run(RunSpec::new(
        "mcs51-mcp-no-such-program",
        Duration::from_secs(5),
    ))
    .await
    .expect_err("a missing program must not panic");
    assert_eq!(err.code(), mcs51_mcp::errors::ErrorCode::ToolNotFound);
}

#[tokio::test]
async fn arguments_are_passed_as_argv_never_through_a_shell() {
    // If this went through `sh -c`, the semicolon would run a second command
    // and `echo` would not print it literally.
    let out = run(RunSpec::new("echo", Duration::from_secs(5)).arg("a; touch /tmp/pwned; b"))
        .await
        .unwrap();
    assert_eq!(out.stdout.text.trim(), "a; touch /tmp/pwned; b");
}

#[tokio::test]
async fn stdin_is_closed_so_a_child_that_reads_it_does_not_hang() {
    // `cat` with no arguments reads stdin. With stdin inherited from a tty it
    // would block forever; with /dev/null it sees EOF at once. This is exactly
    // the packihx failure mode.
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        run(RunSpec::new("cat", Duration::from_secs(3))),
    )
    .await
    .expect("cat must not block on stdin")
    .expect("cat should run");

    assert!(out.success());
    assert!(!out.timed_out);
    assert_eq!(out.stdout.total_bytes, 0);
}

#[tokio::test]
async fn an_overrunning_child_is_killed_and_reaped_leaving_nothing_behind() {
    let started = std::time::Instant::now();
    let out = run(RunSpec::new("sleep", Duration::from_millis(600)).arg("30"))
        .await
        .expect("sleep should spawn");
    let elapsed = started.elapsed();

    assert!(out.timed_out, "a 30s sleep must trip a 600ms budget");
    assert!(
        elapsed < Duration::from_secs(5),
        "the call must return promptly, took {elapsed:?}"
    );
    // Terminated by a signal, not a graceful exit.
    assert_eq!(out.exit_code, None);
    assert!(out.signal.is_some(), "expected a terminating signal");

    // The important part: gone from the process table entirely. If it had been
    // killed without `wait()`, `ps` would still show it as a zombie ("Z"), and
    // on macOS a zombie still holds its file descriptors.
    let pid = out.pid.expect("runner should report the child's pid");
    let mut state = common::process_state(pid);
    for _ in 0..50 {
        if state.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        state = common::process_state(pid);
    }
    assert!(
        state.is_none(),
        "pid {pid} is still in the process table as {state:?} — it was not reaped"
    );
}

#[tokio::test]
async fn an_ignored_sigterm_still_ends_in_sigkill() {
    // A child that traps SIGTERM and keeps going must still be killed. This is
    // why the escalation is SIGTERM -> grace -> SIGKILL -> wait, not just a
    // polite signal and hope.
    let out = run(RunSpec::new("sh", Duration::from_millis(500))
        .args(["-c", "trap '' TERM; sleep 30"])
        .grace(Duration::from_millis(300)))
    .await
    .expect("sh should spawn");

    assert!(out.timed_out);
    assert_eq!(
        out.signal,
        Some(libc::SIGKILL),
        "a child ignoring SIGTERM must end up SIGKILLed"
    );

    let pid = out.pid.expect("pid");
    let mut state = common::process_state(pid);
    for _ in 0..50 {
        if state.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        state = common::process_state(pid);
    }
    assert!(state.is_none(), "pid {pid} left behind as {state:?}");
}

#[tokio::test]
async fn an_unbounded_writer_is_drained_not_buffered_and_not_stalled() {
    // `yes` writes forever as fast as it can. Two failure modes are being ruled
    // out at once:
    //   - buffering everything  -> memory grows without bound
    //   - reading only `cap` and then stopping -> the 64 KiB pipe buffer fills,
    //     `yes` blocks in write(), and total_bytes stalls near the pipe size.
    const CAP: usize = 4096;
    let out = run(RunSpec::new("yes", Duration::from_millis(700))
        .arg("mcs51")
        .capture_cap(CAP))
    .await
    .expect("yes should spawn");

    assert!(out.timed_out);

    // Retained text stays inside the cap (plus the elision marker).
    assert!(
        out.stdout.text.len() < CAP + 128,
        "retained {} bytes for a {CAP}-byte cap",
        out.stdout.text.len()
    );
    assert!(out.stdout.truncated);

    // But we kept reading the whole time. A stalled reader would sit around one
    // pipe buffer (64 KiB); this should be orders of magnitude past that.
    assert!(
        out.stdout.total_bytes > 1_000_000,
        "only drained {} bytes in 700ms — the reader stalled instead of discarding",
        out.stdout.total_bytes
    );
    // Once the buffer is full it retains exactly `cap` bytes, so every byte
    // read is accounted for as either retained or elided — nothing is lost
    // silently and nothing is double-counted.
    assert_eq!(
        out.stdout.elided_bytes,
        out.stdout.total_bytes - CAP as u64,
        "retained + elided must equal what was read"
    );

    // Head and tail are both real output, with the marker between them. The
    // tail is a byte window, so it may start mid-word — what matters is that it
    // is the *end* of the stream, not more of the beginning.
    assert!(out.stdout.text.starts_with("mcs51"));
    assert!(out.stdout.text.contains("bytes elided"));
    let after_marker = out
        .stdout
        .text
        .split_once("]...")
        .expect("elision marker")
        .1;
    assert!(
        after_marker.contains("mcs51"),
        "the tail half should hold real output, got {after_marker:?}"
    );
}

#[tokio::test]
async fn both_streams_are_captured_concurrently() {
    let out = run(RunSpec::new("sh", Duration::from_secs(10))
        .args(["-c", "echo to-stdout; echo to-stderr >&2; exit 3"]))
    .await
    .unwrap();

    assert_eq!(out.exit_code, Some(3));
    assert_eq!(out.stdout.text.trim(), "to-stdout");
    assert_eq!(out.stderr.text.trim(), "to-stderr");
    assert_eq!(out.diagnostics().trim(), "to-stderr");
}

#[tokio::test]
async fn the_cwd_is_honoured() {
    let dir = common::TempDir::new("cwd");
    let out = run(RunSpec::new("pwd", Duration::from_secs(5)).cwd(dir.path()))
        .await
        .unwrap();
    assert_eq!(out.stdout.text.trim(), dir.path().to_string_lossy());
}
