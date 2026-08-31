//! Serial sessions over blocking `serialport` handles.
//!
//! [`registry`] holds the state machine; [`ops`] holds the bounded blocking
//! operations it runs; [`enumerate`] lists what is plugged in.

pub mod enumerate;
pub mod ops;
pub mod registry;
pub mod session;

pub use registry::{SessionRegistry, BUSY_REAP_AFTER};
pub use session::{IoStats, PortSlot, SerialLink, Session, SessionInfo};
