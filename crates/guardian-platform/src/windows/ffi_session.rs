//! WTS session lookup and `CreateProcessAsUserW` (the only `unsafe` for sessions).
#![allow(unsafe_code)]

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TOKEN_ALL_ACCESS, TokenPrimary,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTSFreeMemory, WTSGetActiveConsoleSessionId,
    WTSQuerySessionInformationW, WTSQueryUserToken, WTSUserName,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, PROCESS_INFORMATION,
    STARTUPINFOW,
};
use windows::core::{PCWSTR, PWSTR};

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: a handle from a successful call, closed once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// The session at the physical console, if any.
pub fn active_console_session() -> Option<u32> {
    // SAFETY: no arguments.
    let id = unsafe { WTSGetActiveConsoleSessionId() };
    (id != u32::MAX).then_some(id)
}

/// The user logged in to `session`, if any.
pub fn session_user(session: u32) -> Option<String> {
    let mut buf = PWSTR::null();
    let mut bytes = 0u32;
    // SAFETY: out-pointers are valid; the buffer is freed with WTSFreeMemory below.
    unsafe {
        WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            session,
            WTSUserName,
            &mut buf,
            &mut bytes,
        )
    }
    .ok()?;
    if buf.is_null() {
        return None;
    }
    // SAFETY: on success `buf` is a NUL-terminated wide string owned by WTS.
    let name = unsafe { buf.to_string() }.ok();
    // SAFETY: frees the buffer allocated by WTSQuerySessionInformationW.
    unsafe { WTSFreeMemory(buf.0.cast()) };
    name.filter(|n| !n.is_empty())
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Starts `command_line` as the user of `session` on `winsta0\default`; its PID.
pub fn spawn_as_session_user(session: u32, command_line: &str) -> windows::core::Result<u32> {
    let mut user_token = HANDLE::default();
    // SAFETY: needs SeTcbPrivilege (LocalSystem); `user_token` is a valid out-pointer.
    unsafe { WTSQueryUserToken(session, &mut user_token)? };
    let user_token = Handle(user_token);
    let mut primary = HANDLE::default();
    // SAFETY: duplicates a live token into a primary token.
    unsafe {
        DuplicateTokenEx(
            user_token.0,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )?
    };
    let primary = Handle(primary);
    let mut env: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `env` receives a block freed with DestroyEnvironmentBlock below.
    unsafe { CreateEnvironmentBlock(&mut env, Some(primary.0), false)? };
    let mut desktop = wide(r"winsta0\default");
    let mut cmd = wide(command_line);
    let si = STARTUPINFOW {
        cb: u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0),
        lpDesktop: PWSTR(desktop.as_mut_ptr()),
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();
    // SAFETY: all buffers live until the call returns; `cmd` is mutable as required.
    let r = unsafe {
        CreateProcessAsUserW(
            Some(primary.0),
            PCWSTR::null(),
            Some(PWSTR(cmd.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            Some(env.cast_const()),
            PCWSTR::null(),
            &si,
            &mut pi,
        )
    };
    // SAFETY: the block came from CreateEnvironmentBlock.
    let _ = unsafe { DestroyEnvironmentBlock(env) };
    r?;
    let pid = pi.dwProcessId;
    drop(Handle(pi.hThread));
    drop(Handle(pi.hProcess));
    Ok(pid)
}
