//! Datasheet facts for the AT89S52 / STC89C52, kept as constants.
//!
//! [`pins`] is the single source of truth for the DIP-40 package and is read by
//! both the `pinout` tool and `safety_preflight`, so the reference the model
//! reads and the rules the server enforces cannot drift apart.

pub mod limits;
pub mod pins;
