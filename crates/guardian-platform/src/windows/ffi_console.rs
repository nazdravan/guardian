//! Attaching a GUI-subsystem program to the console it was started from (the
//! only `unsafe` for it).
#![allow(unsafe_code)]

use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

/// Attaches to the parent's console (cmd, PowerShell); `false` when there is none.
pub fn attach_parent_console() -> bool {
    // SAFETY: no pointers; fails harmlessly when there is no parent console.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok()
}
