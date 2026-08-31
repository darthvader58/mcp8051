//! The session registry: checkout, blocking I/O, check-in.
//!
//! `serialport` is blocking, so every read and write runs on a
//! `spawn_blocking` thread. The delicate part is not the blocking call, it is
//! where the *put-back* lives.
//!
//! ```text
//! caller future
//!   └─ awaits JoinHandle ──────────────┐   (may be dropped: cancelled request)
//!                                      │
//! tokio::spawn (detached) ─────────────┘
//!   ├─ spawn_blocking(op).await            <- the actual I/O
//!   └─ re-lock the map and check the port back in
//! ```
//!
//! The check-in sits inside the detached `tokio::spawn`, never in the caller's
//! future. `spawn_blocking` tasks cannot be aborted, so if the put-back were
//! inline, a cancelled MCP request — one Ctrl-C — would drop the caller's
//! future at its await point and leave the slot `Busy` forever. The session
//! would be bricked while the blocking thread happily finished its work and
//! dropped the port on the floor. Detaching costs one task per operation and
//! makes the slot's return unconditional.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::errors::AppError;
use crate::serial::session::{IoStats, PortSlot, SerialLink, Session, SessionInfo};

/// A slot checked out for longer than this is presumed lost — a panic that
/// somehow escaped, or a runtime shutdown mid-op — and is reaped to `Poisoned`
/// so the id stops being permanently unusable.
pub const BUSY_REAP_AFTER: Duration = Duration::from_secs(300);

/// Opens ports. Swapped for a fake in tests, which is the entire reason the
/// registry is testable without a USB adapter plugged in.
pub type Opener = Arc<dyn Fn(&str, u32, Duration) -> io::Result<Box<dyn SerialLink>> + Send + Sync>;

/// The real opener.
pub fn system_opener() -> Opener {
    Arc::new(|path: &str, baud: u32, timeout: Duration| {
        serialport::new(path, baud)
            .timeout(timeout)
            .open()
            .map(|p| Box::new(p) as Box<dyn SerialLink>)
            .map_err(io::Error::from)
    })
}

#[derive(Clone)]
pub struct SessionRegistry {
    inner: Arc<Mutex<HashMap<String, Session>>>,
    opener: Opener,
    max_sessions: usize,
    /// Stamped onto every session so a check-in can tell "the session I was
    /// working on" from "a different session that reused its id".
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl SessionRegistry {
    pub fn new(max_sessions: usize) -> Self {
        Self::with_opener(max_sessions, system_opener())
    }

    pub fn with_opener(max_sessions: usize, opener: Opener) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            opener,
            max_sessions,
            generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Session>> {
        // A poisoned map only means some *other* thread panicked while holding
        // it; the map itself is a plain HashMap and is still coherent.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a port and register it under `id`.
    pub fn open(
        &self,
        id: &str,
        port: &str,
        baud: u32,
        io_slice: Duration,
    ) -> Result<SessionInfo, AppError> {
        {
            let map = self.lock();
            if let Some(existing) = map.get(id) {
                return Err(AppError::SessionExists {
                    session: id.to_string(),
                    port: existing.port.clone(),
                });
            }
            if let Some(holder) = map.values().find(|s| s.port == port) {
                return Err(AppError::PortHeldBySession {
                    port: port.to_string(),
                    session: holder.id.clone(),
                });
            }
            if map.len() >= self.max_sessions {
                return Err(AppError::TooManySessions {
                    open: map.len(),
                    max: self.max_sessions,
                });
            }
        }

        let link = (self.opener)(port, baud, io_slice).map_err(|source| AppError::SerialOpen {
            port: port.to_string(),
            baud,
            source,
        })?;

        let mut map = self.lock();
        // Re-check under the lock: two concurrent opens could both have passed
        // the check above while the slow `open()` was in flight.
        if let Some(existing) = map.get(id) {
            return Err(AppError::SessionExists {
                session: id.to_string(),
                port: existing.port.clone(),
            });
        }
        if let Some(holder) = map.values().find(|s| s.port == port) {
            return Err(AppError::PortHeldBySession {
                port: port.to_string(),
                session: holder.id.clone(),
            });
        }
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let session = Session::new(id.to_string(), generation, port.to_string(), baud, link);
        let info = session.info();
        map.insert(id.to_string(), session);
        Ok(info)
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let map = self.lock();
        let mut v: Vec<_> = map.values().map(Session::info).collect();
        v.sort_by(|a, b| a.session.cmp(&b.session));
        v
    }

    pub fn count(&self) -> usize {
        self.lock().len()
    }

    /// Is `port` currently held by one of our sessions? Used by `flash`, which
    /// must not hand a port to stcgal while we have it open.
    ///
    /// [`Self::open`] refuses a second session on a port already held, so at
    /// most one session can match and the answer is well-defined. Without that
    /// invariant this would return an arbitrary one of several holders, and
    /// `flash` would name the wrong session to close.
    pub fn holder_of(&self, port: &str) -> Option<String> {
        let map = self.lock();
        map.values().find(|s| s.port == port).map(|s| s.id.clone())
    }

    /// Close a session.
    ///
    /// If the port is checked out, the handle is not ours to drop — flagging
    /// `close_requested` makes the in-flight check-in drop it and remove the
    /// entry instead of returning it to the map.
    pub fn close(&self, id: &str) -> Result<(SessionInfo, bool), AppError> {
        let mut map = self.lock();
        let session = map.get_mut(id).ok_or_else(|| AppError::SessionNotFound {
            session: id.to_string(),
        })?;

        let busy = matches!(session.slot, PortSlot::Busy { .. });
        if busy {
            session.close_requested = true;
            let mut info = session.info();
            info.close_requested = true;
            return Ok((info, true));
        }

        let session = map.remove(id).expect("checked present under the same lock");
        Ok((session.info(), false))
    }

    /// Run one blocking operation against a session's port.
    ///
    /// The closure receives the port, the session's buffered-but-unconsumed
    /// bytes (which it may consume and rewrite), and a counter to record what
    /// it moved.
    pub async fn with_port<T, F>(&self, id: &str, op: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut dyn SerialLink, &mut Vec<u8>, &mut IoStats) -> Result<T, AppError>
            + Send
            + 'static,
    {
        // ---- 1. checkout: take the handle out of the map, then drop the guard.
        let (mut link, pending, generation) = {
            let mut map = self.lock();
            let session = map.get_mut(id).ok_or_else(|| AppError::SessionNotFound {
                session: id.to_string(),
            })?;

            match &session.slot {
                PortSlot::Poisoned { reason, .. } => {
                    return Err(AppError::SessionPoisoned {
                        session: id.to_string(),
                        reason: reason.clone(),
                    });
                }
                PortSlot::Busy { since } => {
                    if since.elapsed() > BUSY_REAP_AFTER {
                        // Nothing legitimate holds a port this long. Whatever
                        // owned it is gone; stop pretending the id is alive.
                        let reason = format!(
                            "checked out for {:?} with no check-in; the owning task is gone",
                            since.elapsed()
                        );
                        session.slot = PortSlot::Poisoned {
                            reason: reason.clone(),
                            at: Instant::now(),
                        };
                        return Err(AppError::SessionPoisoned {
                            session: id.to_string(),
                            reason,
                        });
                    }
                    // Refuse rather than queue: a queue behind a blocking op
                    // that never finishes is a deadlock with extra steps.
                    return Err(AppError::SessionBusy {
                        session: id.to_string(),
                    });
                }
                PortSlot::Idle(_) => {}
            }

            let taken = std::mem::replace(
                &mut session.slot,
                PortSlot::Busy {
                    since: Instant::now(),
                },
            );
            let PortSlot::Idle(link) = taken else {
                unreachable!("slot was matched as Idle under this lock");
            };
            (
                link,
                std::mem::take(&mut session.pending),
                session.generation,
            )
        };

        // ---- 2. detached task owns the I/O *and* the check-in.
        let inner = Arc::clone(&self.inner);
        let owned_id = id.to_string();

        let handle = tokio::spawn(async move {
            let joined = tokio::task::spawn_blocking(move || {
                let mut pending = pending;
                let mut stats = IoStats::default();
                let outcome = op(link.as_mut(), &mut pending, &mut stats);
                (link, pending, stats, outcome)
            })
            .await;

            // ---- 3. check-in. Runs whether the op succeeded, failed, or panicked.
            let mut map = inner.lock().unwrap_or_else(|e| e.into_inner());

            match joined {
                Ok((link, pending, stats, outcome)) => {
                    let poison = outcome.as_ref().err().and_then(AppError::poison_reason);
                    let new_slot = match poison {
                        // Dropping `link` here closes the fd, which is the right
                        // thing for a device that has physically gone away.
                        Some(reason) => PortSlot::Poisoned {
                            reason,
                            at: Instant::now(),
                        },
                        None => PortSlot::Idle(link),
                    };

                    let mut drop_session = false;
                    // The generation check is what makes "same id" mean "same
                    // session". If ours was closed and the id reopened while we
                    // were blocked, the entry here belongs to a *different*
                    // port; writing our stale handle into it would drop the new
                    // session's live one and leave `holder_of` naming the wrong
                    // device. Matching only on the id would not catch that.
                    match map.get_mut(&owned_id) {
                        Some(session) if session.generation == generation => {
                            session.bytes_read += stats.read;
                            session.bytes_written += stats.written;
                            session.pending = pending;
                            session.slot = new_slot;
                            // A close arrived while we held the port.
                            drop_session = session.close_requested;
                        }
                        // Either the session was removed underneath us, or the
                        // id now belongs to someone else. Drop the handle rather
                        // than leak the fd; there is nowhere it belongs.
                        _ => drop(new_slot),
                    }
                    if drop_session {
                        map.remove(&owned_id);
                    }
                    outcome
                }
                Err(join_err) => {
                    // The closure panicked. Unwinding already dropped the port,
                    // so there is no handle to return — only a slot to mark.
                    let reason = format!("serial operation panicked: {join_err}");
                    if let Some(session) = map.get_mut(&owned_id) {
                        session.slot = PortSlot::Poisoned {
                            reason: reason.clone(),
                            at: Instant::now(),
                        };
                    }
                    Err(AppError::SessionPoisoned {
                        session: owned_id,
                        reason,
                    })
                }
            }
        });

        // Dropping this JoinHandle detaches the task; it does not abort it, so
        // the check-in above still runs even if the caller goes away.
        match handle.await {
            Ok(result) => result,
            Err(join_err) => Err(AppError::internal(format!(
                "serial task did not complete: {join_err}"
            ))),
        }
    }
}
