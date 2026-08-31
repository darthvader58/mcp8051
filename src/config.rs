//! Environment-derived configuration, resolved once at startup.

use std::path::PathBuf;
use std::time::Duration;

use crate::errors::AppError;

/// Env var naming the firmware sandbox root. Unset means confinement is off.
pub const ENV_FIRMWARE_ROOT: &str = "FIRMWARE_ROOT";

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env_u64(key, default as u64) as usize
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Canonicalized sandbox root, or `None` when confinement is off.
    pub firmware_root: Option<PathBuf>,
    /// Budget for one `sdcc` or `packihx` invocation.
    pub compile_timeout: Duration,
    /// Budget for one `stcgal` run. Generous: it waits for a power-cycle.
    pub flash_timeout: Duration,
    /// Budget for a `--version` probe.
    pub probe_timeout: Duration,
    /// Total bytes kept per captured stream (head + tail).
    pub capture_cap: usize,
    /// Bytes kept from one serial read.
    pub serial_read_cap: usize,
    /// Concurrent serial sessions allowed.
    pub max_sessions: usize,
    /// Baud used when `serial_open` omits one.
    pub default_baud: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            firmware_root: None,
            compile_timeout: Duration::from_millis(60_000),
            flash_timeout: Duration::from_millis(180_000),
            probe_timeout: Duration::from_millis(15_000),
            capture_cap: 64 * 1024,
            serial_read_cap: 64 * 1024,
            max_sessions: 8,
            default_baud: crate::hw::limits::DEFAULT_BAUD,
        }
    }
}

impl Config {
    /// Read the environment.
    ///
    /// If `FIRMWARE_ROOT` is set but missing or not a directory this **fails**
    /// rather than quietly running unconfined: silently downgrading a security
    /// boundary because of a typo is exactly how sandboxes stop being sandboxes.
    pub fn from_env() -> Result<Self, AppError> {
        let mut cfg = Self::default();

        if let Some(raw) = std::env::var_os(ENV_FIRMWARE_ROOT) {
            let raw = PathBuf::from(raw);
            if raw.as_os_str().is_empty() {
                return Err(AppError::invalid(format!(
                    "{ENV_FIRMWARE_ROOT} is set but empty. Unset it to run unconfined, or point \
                     it at a real directory."
                )));
            }
            let canonical = std::fs::canonicalize(&raw).map_err(|e| {
                AppError::invalid(format!(
                    "{ENV_FIRMWARE_ROOT}=`{}` could not be resolved ({e}). Refusing to start \
                     unconfined when confinement was explicitly requested.",
                    raw.display()
                ))
            })?;
            if !canonical.is_dir() {
                return Err(AppError::invalid(format!(
                    "{ENV_FIRMWARE_ROOT}=`{}` is not a directory. Refusing to start unconfined \
                     when confinement was explicitly requested.",
                    canonical.display()
                )));
            }
            cfg.firmware_root = Some(canonical);
        }

        cfg.compile_timeout =
            Duration::from_millis(env_u64("MCS51_MCP_COMPILE_TIMEOUT_MS", 60_000));
        cfg.flash_timeout = Duration::from_millis(env_u64("MCS51_MCP_FLASH_TIMEOUT_MS", 180_000));
        cfg.probe_timeout = Duration::from_millis(env_u64("MCS51_MCP_PROBE_TIMEOUT_MS", 15_000));
        cfg.capture_cap = env_usize("MCS51_MCP_CAPTURE_BYTES", 64 * 1024).max(512);
        cfg.serial_read_cap = env_usize("MCS51_MCP_SERIAL_READ_BYTES", 64 * 1024).max(512);
        cfg.max_sessions = env_usize("MCS51_MCP_MAX_SESSIONS", 8).clamp(1, 256);

        Ok(cfg)
    }

    /// `"on"` / `"off"` — surfaced by `doctor` so the boundary is never invisible.
    pub fn confinement(&self) -> &'static str {
        if self.firmware_root.is_some() {
            "on"
        } else {
            "off"
        }
    }
}
