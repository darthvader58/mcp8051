//! `FIRMWARE_ROOT` confinement.
//!
//! The rule is deliberately blunt: resolve the path all the way through
//! symlinks with `fs::canonicalize`, *then* test containment. Checking a
//! textual prefix before resolving is the classic hole — a symlink inside the
//! root pointing at `/etc` passes a string test and fails this one.

use std::path::{Path, PathBuf};

use crate::errors::AppError;

#[derive(Debug, Clone)]
pub struct PathResolver {
    root: Option<PathBuf>,
}

impl PathResolver {
    /// `root` must already be canonical — [`crate::config::Config::from_env`]
    /// canonicalizes it and refuses to start if it cannot.
    pub fn new(root: Option<PathBuf>) -> Self {
        Self { root }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn confinement(&self) -> &'static str {
        if self.root.is_some() {
            "on"
        } else {
            "off"
        }
    }

    /// Base for relative paths: the sandbox root when confined, else the
    /// server's working directory.
    fn base(&self) -> PathBuf {
        match &self.root {
            Some(r) => r.clone(),
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    fn join(&self, raw: &str) -> PathBuf {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base().join(p)
        }
    }

    fn confine(&self, raw: &str, resolved: &Path) -> Result<(), AppError> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        if resolved.starts_with(root) {
            Ok(())
        } else {
            Err(AppError::PathEscapesRoot {
                path: raw.to_string(),
                resolved: resolved.display().to_string(),
                root: root.display().to_string(),
            })
        }
    }

    /// Resolve a path that must already exist. Fully canonicalized, so `..`
    /// segments and symlinks are gone before the containment test runs.
    pub fn resolve_input(&self, raw: &str) -> Result<PathBuf, AppError> {
        if raw.trim().is_empty() {
            return Err(AppError::invalid("path must not be empty"));
        }
        let joined = self.join(raw);
        let resolved = std::fs::canonicalize(&joined).map_err(|_| AppError::PathNotFound {
            path: joined.display().to_string(),
        })?;
        self.confine(raw, &resolved)?;
        Ok(resolved)
    }

    /// Like [`Self::resolve_input`], but also insists it is a regular file.
    pub fn resolve_input_file(&self, raw: &str) -> Result<PathBuf, AppError> {
        let path = self.resolve_input(raw)?;
        if !path.is_file() {
            return Err(AppError::NotAFile {
                path: path.display().to_string(),
            });
        }
        Ok(path)
    }

    /// Resolve a path that may not exist yet.
    ///
    /// The parent directory must exist and is canonicalized; the file name is
    /// then appended. When the target already exists it is canonicalized too,
    /// and while confinement is active a symlink in the target position is
    /// refused outright, so a pre-placed link cannot be written through.
    pub fn resolve_output(&self, raw: &str) -> Result<PathBuf, AppError> {
        if raw.trim().is_empty() {
            return Err(AppError::invalid("path must not be empty"));
        }
        let joined = self.join(raw);
        let name = joined
            .file_name()
            .ok_or_else(|| AppError::invalid(format!("`{raw}` does not name a file")))?
            .to_owned();
        let parent = joined.parent().unwrap_or_else(|| Path::new("/"));
        let parent = std::fs::canonicalize(parent).map_err(|_| AppError::PathNotFound {
            path: parent.display().to_string(),
        })?;

        let candidate = parent.join(&name);
        // `exists()` follows symlinks, so a link inside the root pointing at a
        // path that does not exist *yet* reports false — the check below would
        // be skipped and the write would follow the link straight out of the
        // root. `symlink_metadata` stats the link itself, so it is always seen.
        match std::fs::symlink_metadata(&candidate) {
            Ok(md) if md.file_type().is_symlink() && self.root.is_some() => {
                // Working out where a dangling link *would* land means
                // re-implementing the kernel's path walk, which is where
                // bypasses come from. Nothing legitimately writes a build
                // artifact through a symlink, so refuse it instead.
                let root = self.root.as_ref().expect("checked by the guard above");
                return Err(AppError::PathEscapesRoot {
                    path: raw.to_string(),
                    resolved: format!("{} (a symlink)", candidate.display()),
                    root: root.display().to_string(),
                });
            }
            // A real file already there may still be reachable only via a
            // symlinked parent, so canonicalize and re-check where it lands.
            Ok(_) => {
                if let Ok(real) = std::fs::canonicalize(&candidate) {
                    self.confine(raw, &real)?;
                }
            }
            Err(_) => {}
        }
        self.confine(raw, &candidate)?;
        Ok(candidate)
    }
}
