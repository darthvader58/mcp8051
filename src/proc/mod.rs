//! The subprocess layer. Every child this server spawns goes through
//! [`runner::run`]; nothing else calls `Command`.

pub mod capture;
pub mod display;
pub mod runner;
pub mod which;

pub use runner::{run, RunOutcome, RunSpec};
