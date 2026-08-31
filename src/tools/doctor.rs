//! `doctor` — what is actually installed, and what state the server is in.
//!
//! Version probing is per-tool because the tools disagree about how to be
//! asked. `packihx` in particular has **no** version flag: it treats every
//! argument as a filename, prints `packihx: cannot open --version`, and exits
//! **0 anyway**. Anything that parses a version out of that is inventing one,
//! so this reports `version: null` with a note instead.

use serde_json::json;

use crate::config::Config;
use crate::envelope::{Envelope, NextAction};
use crate::errors::AppError;
use crate::names;
use crate::proc::{which, RunSpec};
use crate::server::Server;
use crate::tools::Invocation;

/// What `doctor` found out about one executable.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolReport {
    pub name: &'static str,
    pub present: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// How the version was obtained, or why it could not be.
    pub note: Option<String>,
    /// The exact command line that would be used to invoke it.
    pub invocation: Option<String>,
}

/// Pull the first `N.N[.N]` looking token out of a version banner.
fn parse_version(text: &str) -> Option<String> {
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')') {
        let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.');
        let mut parts = t.split('.');
        let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !a.is_empty()
            && a.chars().all(|c| c.is_ascii_digit())
            && !b.is_empty()
            && b.chars().all(|c| c.is_ascii_digit())
        {
            return Some(t.to_string());
        }
    }
    None
}

async fn probe_sdcc(cfg: &Config) -> ToolReport {
    let Some(path) = which::find("sdcc") else {
        return ToolReport {
            name: "sdcc",
            present: false,
            path: None,
            version: None,
            note: Some("Not on PATH. `brew install sdcc` installs both sdcc and packihx.".into()),
            invocation: None,
        };
    };

    let spec = RunSpec::new("sdcc", cfg.probe_timeout)
        .arg("--version")
        .capture_cap(8192);
    let invocation = Some(spec.display());
    let outcome = crate::proc::run(spec).await;

    // sdcc prints its banner on stdout, line 1.
    let (version, note) = match outcome {
        Ok(o) if o.success() => {
            let first = o.stdout.text.lines().next().unwrap_or_default().to_string();
            (
                parse_version(&first),
                Some(first.trim().to_string()).filter(|s| !s.is_empty()),
            )
        }
        Ok(o) => (
            None,
            Some(format!("`sdcc --version` exited {:?}", o.exit_code)),
        ),
        Err(e) => (None, Some(format!("could not run sdcc: {e}"))),
    };

    ToolReport {
        name: "sdcc",
        present: true,
        path: Some(path.display().to_string()),
        version,
        note,
        invocation,
    }
}

fn probe_packihx() -> ToolReport {
    match which::find("packihx") {
        Some(path) => ToolReport {
            name: "packihx",
            present: true,
            path: Some(path.display().to_string()),
            // Deliberately not probed: see the module docs.
            version: None,
            note: Some(
                "Ships with SDCC; has no --version flag. It treats every argument as a filename \
                 and exits 0 even on error, so its exit code is not evidence of anything — \
                 `compile` validates the produced .hex by content instead."
                    .into(),
            ),
            invocation: Some("packihx <file>.ihx".into()),
        },
        None => ToolReport {
            name: "packihx",
            present: false,
            path: None,
            version: None,
            note: Some("Not on PATH. It ships with SDCC: `brew install sdcc`.".into()),
            invocation: None,
        },
    }
}

/// Probe stcgal, trying the console script first and `python3 -m stcgal` after.
///
/// A `pip install --user stcgal` may leave the module importable with no script
/// on `PATH`; refusing to flash in that case would be a self-inflicted failure.
pub async fn probe_stcgal(cfg: &Config) -> (ToolReport, Option<Invocation>) {
    let mut candidates: Vec<Invocation> = Vec::new();
    if which::find("stcgal").is_some() {
        candidates.push(Invocation::direct("stcgal"));
    }
    for python in ["python3", "python"] {
        if which::find(python).is_some() {
            candidates.push(Invocation::module(python, "stcgal"));
        }
    }

    let mut last_note = None;
    for inv in candidates {
        let spec = inv
            .spec(cfg.probe_timeout)
            .arg("--version")
            .capture_cap(8192);
        let display = spec.display();
        match crate::proc::run(spec).await {
            Ok(o) if o.success() => {
                // stcgal prints `stcgal 1.10`; some builds use stderr.
                let text = if o.stdout.text.trim().is_empty() {
                    o.stderr.text.clone()
                } else {
                    o.stdout.text.clone()
                };
                let line = text.lines().next().unwrap_or_default().trim().to_string();
                let path = which::find(&inv.program).map(|p| p.display().to_string());
                return (
                    ToolReport {
                        name: "stcgal",
                        present: true,
                        path,
                        version: parse_version(&line),
                        note: Some(line),
                        invocation: Some(display),
                    },
                    Some(inv),
                );
            }
            Ok(o) => {
                last_note = Some(format!("`{display} --version` exited {:?}", o.exit_code));
            }
            Err(e) => last_note = Some(format!("`{display} --version` failed: {e}")),
        }
    }

    (
        ToolReport {
            name: "stcgal",
            present: false,
            path: None,
            version: None,
            note: Some(last_note.unwrap_or_else(|| {
                "Neither a `stcgal` executable nor an importable `stcgal` module was found."
                    .to_string()
            })),
            invocation: None,
        },
        None,
    )
}

pub async fn run(server: &Server) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let cfg = server.config();

    let sdcc = probe_sdcc(cfg).await;
    let packihx = probe_packihx();
    let (stcgal, stcgal_invocation) = probe_stcgal(cfg).await;

    // Cache the working form so `flash` does not repeat the probe.
    server.set_stcgal(stcgal_invocation.clone());

    let ports = crate::serial::enumerate::list().unwrap_or_default();
    let recommended = ports.iter().filter(|p| p.recommended).count();

    let can_compile = sdcc.present && packihx.present;
    let can_flash = stcgal.present;

    let data = json!({
        "tools": [sdcc, packihx, stcgal],
        "capabilities": {
            "compile": can_compile,
            "flash_stc": can_flash,
            "flash_at89s": false,
            "serial": true,
        },
        "firmware_root": {
            "value": cfg.firmware_root.as_ref().map(|p| p.display().to_string()),
            "confinement": cfg.confinement(),
            "meaning": if cfg.firmware_root.is_some() {
                "Relative paths resolve under FIRMWARE_ROOT, and every path is canonicalized \
                 and must land inside it — symlinks out of the root are refused."
            } else {
                "FIRMWARE_ROOT is unset, so paths are unrestricted. Set it to confine this \
                 server to one directory tree."
            },
        },
        "serial": {
            "ports_total": ports.len(),
            "ports_recommended": recommended,
            "open_sessions": server.sessions.count(),
            "max_sessions": cfg.max_sessions,
        },
        "host": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "server": env!("CARGO_PKG_NAME"),
            "server_version": env!("CARGO_PKG_VERSION"),
        },
        "timeouts_ms": {
            "compile": cfg.compile_timeout.as_millis() as u64,
            "flash": cfg.flash_timeout.as_millis() as u64,
            "probe": cfg.probe_timeout.as_millis() as u64,
        },
        "target": {
            "default_chip": "stc",
            "part": "STC89C52 (8052 core), flashed over its serial bootloader",
            "crystal_hz": crate::hw::limits::CRYSTAL_HZ,
            "uart": "9600 8N1",
        },
    });

    let mut env = Envelope::new(names::DOCTOR)
        .data(data)
        .duration(started.elapsed());

    let missing: Vec<&str> = [&sdcc, &packihx, &stcgal]
        .iter()
        .filter(|t| !t.present)
        .map(|t| t.name)
        .collect();

    if !missing.is_empty() {
        env = env
            .warn()
            .remedy(format!("Missing: {}. ", missing.join(", ")));
    }
    if recommended == 0 {
        env = env.warn().next_action(NextAction::call(
            names::LIST_SERIAL_PORTS,
            "No USB serial adapter is currently attached; plug one in before flashing or \
             opening a session.",
            json!({}),
        ));
    }

    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn pulls_versions_out_of_real_banners() {
        assert_eq!(
            parse_version("SDCC : mcs51/z80/... TD- 4.6.0 #16555 (Mac OS X ppc)"),
            Some("4.6.0".into())
        );
        assert_eq!(parse_version("stcgal 1.10"), Some("1.10".into()));
        assert_eq!(parse_version("no numbers here"), None);
    }
}
