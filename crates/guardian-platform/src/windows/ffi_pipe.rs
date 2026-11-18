//! Named-pipe FFI: the client's PID and pipe instances with an explicit
//! security descriptor.
#![allow(unsafe_code)]

use std::os::windows::io::RawHandle;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows::core::PCWSTR;

/// Network logons denied; SYSTEM and Administrators full access; interactive
/// users (the worker runs as the logged-in student) read and write but not
/// `FILE_CREATE_PIPE_INSTANCE` (0x4), so they cannot add server instances.
/// `0x12019b` = `FILE_GENERIC_READ | FILE_GENERIC_WRITE` without that bit.
const PIPE_SDDL: &str = "D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12019b;;;IU)";

/// PID of the client connected to the server end `h`.
pub fn client_pid(h: RawHandle) -> std::io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: `h` is a live named-pipe server handle owned by the caller; `pid` is
    // a valid out-pointer.
    unsafe { GetNamedPipeClientProcessId(HANDLE(h), &mut pid) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(pid)
}

/// Frees a descriptor allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
struct LocalSd(PSECURITY_DESCRIPTOR);

impl Drop for LocalSd {
    fn drop(&mut self) {
        // SAFETY: the pointer came from LocalAlloc (inside the conversion call)
        // and is freed exactly once, here.
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

/// A new server instance of pipe `name` with [`PIPE_SDDL`].
pub fn create_server(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sddl` is NUL-terminated UTF-16 that outlives the call; `sd` is a
    // valid out-pointer; the size out-parameter is optional.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| std::io::Error::other(format!("pipe security descriptor: {e}")))?;
    let sd = LocalSd(sd);
    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| std::io::Error::other("SECURITY_ATTRIBUTES size"))?,
        lpSecurityDescriptor: sd.0.0,
        bInheritHandle: false.into(),
    };
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);
    // SAFETY: `attrs` and the descriptor it points to (kept alive by `sd`) are
    // valid for the whole call; the pipe copies the descriptor.
    unsafe {
        opts.create_with_security_attributes_raw(
            name,
            (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
        )
    }
}
