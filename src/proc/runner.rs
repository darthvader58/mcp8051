//! The one subprocess runner.
//!
//! Design constraints, each of which is a bug that would otherwise be latent:
//!
//! - **Direct argv, never `sh -c`.** A firmware path is untrusted input; going
//!   through a shell turns a filename into a script.
//! - **`timeout` is a required field, not an `Option`.** There is deliberately
//!   no way to spell "run forever" — the type system forbids the untimed path.
//! - **Never `wait_with_output()`.** That buffers without bound. The pipes are
//!   drained concurrently with the wait, through a cap that discards the middle
//!   while still reading, so the child never blocks on a full pipe.
//! - **Timeout escalates and then reaps.** SIGTERM, a grace window (long enough
//!   for pyserial to put the tty's termios back), then SIGKILL, then `wait()`.
//!   Skipping the final `wait()` leaves a zombie holding the serial port.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::Instant;

use crate::envelope::Tail;
use crate::errors::AppError;
use crate::proc::{capture, display, which};

/// How long to let a child clean up after SIGTERM before SIGKILL.
pub const DEFAULT_GRACE: Duration = Duration::from_millis(500);

/// Grace for `stcgal`: pyserial needs a moment to restore the tty's termios, and
/// a hard kill mid-restore leaves the port in a state that needs a re-plug.
pub const STCGAL_GRACE: Duration = Duration::from_secs(3);

/// Everything one child run needs.
pub struct RunSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Required. Not an `Option` on purpose.
    pub timeout: Duration,
    pub grace: Duration,
    /// Bytes retained per stream.
    pub capture_cap: usize,
}

impl RunSpec {
    pub fn new(program: impl Into<String>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            timeout,
            grace: DEFAULT_GRACE,
            capture_cap: 64 * 1024,
        }
    }

    #[must_use]
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(it.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    #[must_use]
    pub fn grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    #[must_use]
    pub fn capture_cap(mut self, cap: usize) -> Self {
        self.capture_cap = cap;
        self
    }

    /// The command line, quoted for a human to read. Never re-executed.
    pub fn display(&self) -> String {
        display::render(&self.program, &self.args)
    }
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Shell-quoted argv, display only.
    pub command: String,
    /// The child's pid. Kept so a caller (or a test) can prove it was reaped.
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    /// Signal that killed the child, if any.
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub stdout: Tail,
    pub stderr: Tail,
    pub duration: Duration,
}

impl RunOutcome {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }

    /// stderr if the child said anything there, else stdout. Compilers use
    /// stderr; some tools report failures on stdout.
    pub fn diagnostics(&self) -> &str {
        if !self.stderr.text.trim().is_empty() {
            &self.stderr.text
        } else {
            &self.stdout.text
        }
    }
}

/// Spawn, drain, wait, and — if it overruns — terminate and reap.
pub async fn run(spec: RunSpec) -> Result<RunOutcome, AppError> {
    let command = spec.display();
    let resolved = which::find(&spec.program).ok_or_else(|| AppError::ToolNotFound {
        tool: spec.program.clone(),
    })?;

    let mut cmd = Command::new(&resolved);
    cmd.args(&spec.args)
        // Closed stdin. packihx reads stdin when given no readable file and
        // would otherwise block forever on an inherited tty.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // If this future is dropped (a cancelled MCP request), the child dies
        // with it rather than lingering with the serial port open.
        .kill_on_drop(true);
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }

    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|source| AppError::Spawn {
        program: spec.program.clone(),
        source,
    })?;

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Drains get a little longer than the child does, so a child killed at the
    // deadline still gets its final bytes collected; the extra slack also caps
    // the pathological case of a grandchild inheriting the pipe.
    let drain_deadline = started + spec.timeout + spec.grace + Duration::from_secs(2);
    let cap = spec.capture_cap;

    let out_fut = async {
        match stdout {
            Some(r) => capture::drain(r, cap, drain_deadline).await,
            None => Tail::default(),
        }
    };
    let err_fut = async {
        match stderr {
            Some(r) => capture::drain(r, cap, drain_deadline).await,
            None => Tail::default(),
        }
    };
    let wait_fut = wait_with_deadline(&mut child, pid, spec.timeout, spec.grace);

    let (stdout, stderr, waited) = tokio::join!(out_fut, err_fut, wait_fut);
    let (status, timed_out) = waited?;

    let signal = signal_of(&status);
    Ok(RunOutcome {
        command,
        pid,
        exit_code: status.code(),
        signal,
        timed_out,
        stdout,
        stderr,
        duration: started.elapsed(),
    })
}

#[cfg(unix)]
fn signal_of(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn signal_of(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

/// Wait for the child, escalating on overrun. Always returns a reaped status.
async fn wait_with_deadline(
    child: &mut tokio::process::Child,
    pid: Option<u32>,
    timeout: Duration,
    grace: Duration,
) -> Result<(std::process::ExitStatus, bool), AppError> {
    let wait_err = |e: std::io::Error| AppError::io("waiting for child process", e);

    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => Ok((status.map_err(wait_err)?, false)),
        Err(_) => {
            // Polite first: SIGTERM lets the child restore terminal state.
            #[cfg(unix)]
            if let Some(pid) = pid {
                // SAFETY: `pid` came from a child we spawned and have not yet
                // reaped, so it cannot have been recycled for another process.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }
            #[cfg(not(unix))]
            let _ = pid;

            if let Ok(status) = tokio::time::timeout(grace, child.wait()).await {
                return Ok((status.map_err(wait_err)?, true));
            }

            // Still there. SIGKILL, then reap — without this wait the child
            // becomes a zombie still holding whatever fds it had open.
            child.start_kill().map_err(wait_err)?;
            let status = child.wait().await.map_err(wait_err)?;
            Ok((status, true))
        }
    }
}
