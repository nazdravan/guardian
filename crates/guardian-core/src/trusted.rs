//! Directories whose executables may keep a protected system name.
//!
//! A protected name (`svchost.exe`, `bash`, …) protects a process only when its
//! executable lives in one of these places; a copy anywhere else is judged like any
//! other program, so renaming a game does not get it past Exam mode.

use std::path::{Component, Path, PathBuf};

use crate::enforce::Os;

/// Trusted locations for one OS.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedRoots {
    /// Any file below these directories.
    pub recursive: Vec<PathBuf>,
    /// Only files directly inside these directories (e.g. `C:\Windows\explorer.exe`).
    pub direct_files: Vec<PathBuf>,
    /// Subdirectories (recursively) taken out of trust: places ordinary users can write.
    pub excluded: Vec<PathBuf>,
}

/// Path components as comparable strings; `None` if the path climbs with `..`.
fn parts(os: Os, p: &Path) -> Option<Vec<String>> {
    match os {
        Os::Windows => {
            let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
            let mut out = Vec::new();
            for c in s.split('\\').filter(|c| !c.is_empty() && *c != ".") {
                if c == ".." {
                    return None;
                }
                out.push(c.to_string());
            }
            Some(out)
        }
        Os::Linux => {
            let mut out = Vec::new();
            for c in p.components() {
                match c {
                    Component::ParentDir => return None,
                    Component::Normal(n) => out.push(n.to_string_lossy().into_owned()),
                    Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
                }
            }
            Some(out)
        }
    }
}

fn starts_with(path: &[String], dir: &[String]) -> bool {
    path.len() > dir.len() && path[..dir.len()] == *dir
}

impl TrustedRoots {
    /// Nothing is trusted.
    pub fn none() -> TrustedRoots {
        TrustedRoots::default()
    }

    /// Whether the executable at `exe` is in a trusted location.
    pub fn contains(&self, os: Os, exe: &Path) -> bool {
        let Some(path) = parts(os, exe) else {
            return false;
        };
        let dirs =
            |v: &[PathBuf]| -> Vec<Vec<String>> { v.iter().filter_map(|d| parts(os, d)).collect() };
        if dirs(&self.excluded).iter().any(|d| starts_with(&path, d)) {
            return false;
        }
        dirs(&self.recursive).iter().any(|d| starts_with(&path, d))
            || dirs(&self.direct_files)
                .iter()
                .any(|d| path.len() == d.len() + 1 && starts_with(&path, d))
    }
}
