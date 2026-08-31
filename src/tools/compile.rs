//! `compile` — `sdcc -mmcs51`, then `packihx`, then *verify the result*.
//!
//! Two things here are not the obvious implementation:
//!
//! 1. `packihx` writes the packed hex to **stdout**. The documented recipe is
//!    `packihx foo.ihx > foo.hex`, but running that through a shell would make
//!    a firmware path into shell syntax. Instead stdout is captured and written
//!    with `tokio::fs::write`, which is the same result with no shell involved.
//! 2. `packihx` **always exits 0**, even when it could not open its input. Its
//!    exit code carries no information, so success is decided by inspecting the
//!    bytes: non-empty, starts with a record mark, and ends with the Intel-HEX
//!    EOF record `:00000001FF`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::envelope::{Envelope, NextAction};
use crate::errors::{AppError, ErrorCode};
use crate::names;
use crate::proc::{self, RunSpec};
use crate::server::Server;

/// Intel-HEX end-of-file record. Its absence means a truncated image.
const IHEX_EOF: &str = ":00000001FF";

/// packihx output is a whole firmware image, so it gets a much larger cap than
/// diagnostics do. 8 MiB is far past the 64 KB an 8051 can address.
const HEX_CAPTURE_CAP: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompileArgs {
    /// Path to the C source to compile.
    pub source: String,
    /// Where to write the packed .hex. Defaults to the source's name with a
    /// `.hex` extension, beside the source.
    pub out: Option<String>,
}

pub async fn run(server: &Server, args: CompileArgs) -> Result<Envelope, AppError> {
    let started = std::time::Instant::now();
    let cfg = server.config();

    let source = server.paths.resolve_input_file(&args.source)?;
    let dir = source
        .parent()
        .ok_or_else(|| AppError::invalid("source has no parent directory"))?
        .to_path_buf();
    let stem = source
        .file_stem()
        .ok_or_else(|| AppError::invalid("source has no file name"))?
        .to_string_lossy()
        .into_owned();
    let file_name = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    // ---- sdcc ---------------------------------------------------------------
    // Run in the source's directory: sdcc scatters .asm/.rel/.lst/.sym beside
    // its input, and this keeps that mess out of the server's cwd.
    let sdcc = proc::run(
        RunSpec::new("sdcc", cfg.compile_timeout)
            .arg("-mmcs51")
            .arg(&file_name)
            .cwd(&dir)
            .capture_cap(cfg.capture_cap),
    )
    .await?;

    if sdcc.timed_out {
        return Err(AppError::ProcessTimeout {
            program: "sdcc".into(),
            timeout_ms: cfg.compile_timeout.as_millis() as u64,
        });
    }
    if !sdcc.success() {
        // sdcc's diagnostics are the whole value of this failure: pass them
        // through untouched rather than summarizing them.
        return Ok(Envelope::new(names::COMPILE)
            .error(
                ErrorCode::CompileFailed,
                format!(
                    "sdcc exited {:?}. Its diagnostics are in `stderr`, verbatim.",
                    sdcc.exit_code
                ),
            )
            .remedy(
                "Fix the reported errors and compile again. sdcc reports the first genuine \
                 error first; later ones are often cascades.",
            )
            .command(sdcc.command)
            .exit_code(sdcc.exit_code)
            .stdout(sdcc.stdout)
            .stderr(sdcc.stderr)
            .data(json!({ "source": source.display().to_string() }))
            .duration(started.elapsed()));
    }

    let ihx_name = format!("{stem}.ihx");
    let ihx_path = dir.join(&ihx_name);
    if !ihx_path.is_file() {
        return Err(AppError::HexInvalid {
            message: format!(
                "sdcc reported success but produced no `{ihx_name}`. Nothing was written."
            ),
            path: ihx_path.display().to_string(),
        });
    }

    // ---- packihx ------------------------------------------------------------
    let packed = proc::run(
        RunSpec::new("packihx", cfg.compile_timeout)
            .arg(&ihx_name)
            .cwd(&dir)
            .capture_cap(HEX_CAPTURE_CAP),
    )
    .await?;

    if packed.timed_out {
        return Err(AppError::ProcessTimeout {
            program: "packihx".into(),
            timeout_ms: cfg.compile_timeout.as_millis() as u64,
        });
    }
    if packed.stdout.truncated {
        return Err(AppError::HexInvalid {
            message: format!(
                "packihx produced {} bytes, more than this server will buffer for one image. \
                 The .hex was not written rather than written truncated.",
                packed.stdout.total_bytes
            ),
            path: ihx_path.display().to_string(),
        });
    }

    let hex_text = packed.stdout.text.clone();
    let hex_path = match &args.out {
        Some(out) => server.paths.resolve_output(out)?,
        None => dir.join(format!("{stem}.hex")),
    };

    // ---- validate before writing -------------------------------------------
    // packihx exits 0 unconditionally, so this is the only real check there is.
    validate_hex(&hex_text, &packed.stderr.text, &ihx_path)?;

    tokio::fs::write(&hex_path, hex_text.as_bytes())
        .await
        .map_err(|e| AppError::io(format!("writing {}", hex_path.display()), e))?;

    let records = hex_text.lines().filter(|l| l.starts_with(':')).count();
    let command = format!(
        "{}  &&  {}   # stdout captured and written to {}, not shell-redirected",
        sdcc.command,
        packed.command,
        hex_path.display()
    );

    Ok(Envelope::new(names::COMPILE)
        .command(command)
        .exit_code(Some(0))
        .stdout(sdcc.stdout)
        .stderr(sdcc.stderr)
        .data(json!({
            "source": source.display().to_string(),
            "ihx": ihx_path.display().to_string(),
            "hex": hex_path.display().to_string(),
            "hex_bytes": hex_text.len(),
            "hex_records": records,
            "validated": {
                "non_empty": true,
                "starts_with_record_mark": true,
                "has_eof_record": true,
                "why": "packihx always exits 0, so the image is checked by content."
            },
        }))
        .next_action(NextAction::call(
            names::FLASH,
            "Write it to an STC89C52 over its serial bootloader.",
            json!({ "chip": "stc", "hex": hex_path.display().to_string(), "port": "/dev/cu.usbserial-XXXX" }),
        ))
        .duration(started.elapsed()))
}

/// Decide whether packihx actually produced an Intel-HEX image.
fn validate_hex(text: &str, stderr: &str, ihx: &std::path::Path) -> Result<(), AppError> {
    let path = ihx.display().to_string();
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return Err(AppError::HexInvalid {
            message: format!(
                "packihx produced no output (it exits 0 even when it cannot open its input; \
                 its stderr said: {}).",
                first_line(stderr)
            ),
            path,
        });
    }
    if !trimmed.starts_with(':') {
        return Err(AppError::HexInvalid {
            message: format!(
                "packihx output does not begin with an Intel-HEX record mark ':' — it begins \
                 with {:?}. This is not a hex image.",
                &trimmed.chars().take(24).collect::<String>()
            ),
            path,
        });
    }
    if !trimmed.contains(IHEX_EOF) {
        return Err(AppError::HexInvalid {
            message: format!(
                "packihx output has no Intel-HEX EOF record ({IHEX_EOF}), so the image is \
                 truncated. Flashing it would write a partial program."
            ),
            path,
        });
    }
    Ok(())
}

fn first_line(s: &str) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("(nothing)");
    line.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn check(text: &str) -> Result<(), AppError> {
        validate_hex(text, "", Path::new("/tmp/x.ihx"))
    }

    #[test]
    fn accepts_a_real_image() {
        assert!(check(":03000000020003F8\n:00000001FF\n").is_ok());
    }

    #[test]
    fn rejects_the_three_ways_packihx_lies() {
        // Exits 0 having printed nothing.
        assert!(check("").is_err());
        // Exits 0 having printed an error message.
        assert!(check("packihx: cannot open foo.ihx").is_err());
        // Exits 0 with a truncated image.
        assert!(check(":03000000020003F8\n").is_err());
    }
}
