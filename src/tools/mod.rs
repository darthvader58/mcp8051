//! Tool bodies.
//!
//! `server.rs` holds the twelve `#[tool]` signatures because `#[tool_router]`
//! only collects attributes from its own impl block. Each of those is a thin
//! shim delegating here, where the real work can use `?` against
//! [`crate::errors::AppError`] instead of hand-rolling early returns.

pub mod compile;
pub mod doctor;
pub mod flash;
pub mod pinout;
pub mod safety;
pub mod serial_io;
pub mod serial_session;

/// How to invoke a Python-packaged tool that may or may not have a console
/// script on `PATH`.
///
/// `stcgal` is the case that matters: `pipx` puts a `stcgal` binary in
/// `~/.local/bin`, but a plain `pip install --user` may leave only the module.
/// `doctor` works out which form exists and caches it on the server so `flash`
/// does not have to re-probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub prefix: Vec<String>,
}

impl Invocation {
    pub fn direct(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            prefix: Vec::new(),
        }
    }

    pub fn module(python: impl Into<String>, module: impl Into<String>) -> Self {
        Self {
            program: python.into(),
            prefix: vec!["-m".to_string(), module.into()],
        }
    }

    /// Start a [`crate::proc::RunSpec`] for this invocation.
    pub fn spec(&self, timeout: std::time::Duration) -> crate::proc::RunSpec {
        crate::proc::RunSpec::new(self.program.clone(), timeout).args(self.prefix.clone())
    }

    pub fn display(&self) -> String {
        crate::proc::display::render(&self.program, &self.prefix)
    }
}
