//! The only `unsafe` for Windows processes: thin wrappers over Win32 calls.
#![allow(unsafe_code)]

use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
    TerminateProcess,
};
use windows::core::PWSTR;

/// HRESULT for `ERROR_ACCESS_DENIED`.
pub const E_ACCESS_DENIED: i32 = 0x8007_0005_u32 as i32;
/// HRESULT for `ERROR_INVALID_PARAMETER` (no such PID).
pub const E_INVALID_PARAMETER: i32 = 0x8007_0057_u32 as i32;

/// A handle closed on drop.
pub struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Win32 call and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

// SAFETY: a process handle may be used from any thread.
unsafe impl Send for OwnedHandle {}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// `(pid, image name)` of every process.
pub fn snapshot() -> windows::core::Result<Vec<(u32, String)>> {
    // SAFETY: plain call; the returned handle is owned below.
    let snap = OwnedHandle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? });
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0),
        ..Default::default()
    };
    let mut out = Vec::new();
    // SAFETY: `entry` is a properly sized PROCESSENTRY32W owned by this frame.
    let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) };
    while ok.is_ok() {
        out.push((entry.th32ProcessID, wide_to_string(&entry.szExeFile)));
        // SAFETY: as above.
        ok = unsafe { Process32NextW(snap.0, &mut entry) };
    }
    Ok(out)
}

/// Opens a process for querying, or for querying and terminating.
pub fn open(pid: u32, terminate: bool) -> windows::core::Result<OwnedHandle> {
    let mut access: PROCESS_ACCESS_RIGHTS = PROCESS_QUERY_LIMITED_INFORMATION;
    if terminate {
        access |= PROCESS_TERMINATE;
    }
    // SAFETY: plain call; the returned handle is owned by `OwnedHandle`.
    Ok(OwnedHandle(unsafe { OpenProcess(access, false, pid)? }))
}

impl OwnedHandle {
    /// Full Win32 path of the executable.
    pub fn image_path(&self) -> Option<PathBuf> {
        let mut buf = vec![0u16; 32_768];
        let mut len = u32::try_from(buf.len()).unwrap_or(0);
        // SAFETY: `buf` has `len` writable u16s; the call writes at most that many.
        unsafe {
            QueryFullProcessImageNameW(
                self.0,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
        }
        .ok()?;
        let len = usize::try_from(len).ok()?;
        Some(PathBuf::from(std::ffi::OsString::from_wide(
            buf.get(..len)?,
        )))
    }

    /// Creation time as a FILETIME in one u64.
    pub fn creation_time(&self) -> Option<u64> {
        let (mut c, mut e, mut k, mut u) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: four valid FILETIME out-pointers owned by this frame.
        unsafe { GetProcessTimes(self.0, &mut c, &mut e, &mut k, &mut u) }.ok()?;
        Some((u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime))
    }

    /// `TerminateProcess(h, 1)`.
    pub fn terminate(&self) -> windows::core::Result<()> {
        // SAFETY: the handle was opened with PROCESS_TERMINATE.
        unsafe { TerminateProcess(self.0, 1) }
    }
}
