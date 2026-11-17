//! `GetNamedPipeClientProcessId` (the only `unsafe` for pipes).
#![allow(unsafe_code)]

use std::os::windows::io::RawHandle;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;

/// PID of the client connected to the server end `h`.
pub fn client_pid(h: RawHandle) -> std::io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: `h` is a live named-pipe server handle owned by the caller; `pid` is
    // a valid out-pointer.
    unsafe { GetNamedPipeClientProcessId(HANDLE(h), &mut pid) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(pid)
}
