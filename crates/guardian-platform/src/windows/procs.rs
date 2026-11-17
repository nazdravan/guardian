//! Windows processes: Toolhelp listing, and killing through a handle whose
//! identity is checked first.

use std::path::Path;

use guardian_core::enforce::ProcessInfo;

use super::ffi_procs::{self, E_ACCESS_DENIED, E_INVALID_PARAMETER};
use crate::PlatformError;

fn map(e: &windows::core::Error) -> PlatformError {
    match e.code().0 {
        E_ACCESS_DENIED => PlatformError::AccessDenied,
        E_INVALID_PARAMETER => PlatformError::NotFound,
        _ => PlatformError::Os(e.to_string()),
    }
}

fn describe(pid: u32, name: String) -> ProcessInfo {
    let (exe_path, start_marker) = match ffi_procs::open(pid, false) {
        Ok(h) => (h.image_path(), h.creation_time().unwrap_or(0)),
        Err(_) => (None, 0),
    };
    // Prefer the executable's file name; Toolhelp's name is the same in practice.
    let name = exe_path
        .as_deref()
        .and_then(Path::file_name)
        .map_or(name, |n| n.to_string_lossy().into_owned());
    ProcessInfo {
        pid,
        name,
        exe_path,
        start_marker,
    }
}

/// Every process.
pub fn list() -> Result<Vec<ProcessInfo>, PlatformError> {
    let snap = ffi_procs::snapshot().map_err(|e| map(&e))?;
    Ok(snap
        .into_iter()
        .map(|(pid, name)| describe(pid, name))
        .collect())
}

/// One process.
pub fn info(pid: u32) -> Result<ProcessInfo, PlatformError> {
    let snap = ffi_procs::snapshot().map_err(|e| map(&e))?;
    snap.into_iter()
        .find(|(p, _)| *p == pid)
        .map(|(p, name)| describe(p, name))
        .ok_or(PlatformError::NotFound)
}

/// Terminates `p` through one handle, after checking on that same handle that
/// the executable and creation time are still the ones judged.
pub fn kill_checked(p: &ProcessInfo) -> Result<(), PlatformError> {
    let h = ffi_procs::open(p.pid, true).map_err(|e| map(&e))?;
    let path = h.image_path();
    let created = h.creation_time().unwrap_or(0);
    let name = path
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    if path != p.exe_path
        || created != p.start_marker
        || name.as_deref().is_some_and(|n| n != p.name)
    {
        return Err(PlatformError::Changed);
    }
    h.terminate().map_err(|e| map(&e))
}
