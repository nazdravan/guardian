//! Whether this process runs elevated (the only `unsafe` for it).
#![allow(unsafe_code)]

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// `true` when the process token is elevated (administrator rights in use).
pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo-handle of this process; `token` is written on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(0);
    // SAFETY: `elevation` is a TOKEN_ELEVATION of `size` bytes owned by this frame.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            size,
            &mut len,
        )
    }
    .is_ok();
    // SAFETY: a handle from the successful OpenProcessToken, closed once.
    let _ = unsafe { CloseHandle(token) };
    ok && elevation.TokenIsElevated != 0
}
