//! Locating executables.
//!
//! An MCP server is usually launched by a GUI client, which hands it a much
//! thinner `PATH` than a login shell. `stcgal` in particular installs to
//! `~/.local/bin`, which is almost never on that `PATH`, so a few well-known
//! locations are searched after `PATH` proper rather than reporting the tool
//! missing when it is plainly installed.

use std::path::{Path, PathBuf};

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Directories searched after `PATH`.
pub fn fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.insert(0, PathBuf::from(&home).join(".local/bin"));
        dirs.push(PathBuf::from(&home).join("Library/Python/3.9/bin"));
    }
    dirs
}

/// Find `program`, returning its absolute path.
pub fn find(program: &str) -> Option<PathBuf> {
    // An explicit path is used as given.
    if program.contains('/') {
        let p = Path::new(program);
        return is_executable(p).then(|| p.to_path_buf());
    }

    let from_path = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();

    from_path
        .into_iter()
        .chain(fallback_dirs())
        .map(|dir| dir.join(program))
        .find(|c| is_executable(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_universally_present_binary() {
        assert!(find("sh").is_some());
    }

    #[test]
    fn reports_absent_binaries_as_absent() {
        assert!(find("mcs51-mcp-definitely-not-installed").is_none());
    }
}
