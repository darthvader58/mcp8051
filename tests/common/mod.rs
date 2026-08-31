//! Shared test helpers. No hardware, no network, no `tempfile` dependency.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch directory that deletes itself on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("mcs51-mcp-test-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        // Resolve /var -> /private/var on macOS so comparisons against
        // canonicalized paths line up.
        let path = std::fs::canonicalize(&path).expect("canonicalize temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn write(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.child(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&p, contents).expect("write file");
        p
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        let p = self.child(name);
        std::fs::create_dir_all(&p).expect("create dir");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Is this pid present in the process table at all — including as a zombie?
///
/// `ps -o stat=` prints the state; a reaped child has no row, an unreaped one
/// shows `Z`. Distinguishing the two is the point: killing without waiting
/// leaves a zombie, which is still a leak.
pub fn process_state(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("run ps");
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
