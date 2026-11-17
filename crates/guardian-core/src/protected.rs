//! Processes that are never killed, whatever the profile says.
//!
//! The Windows list mirrors `isSystemProcess` from the 2.x Go agent, minus the
//! 2.x binaries, plus the 3.0 binaries.

use crate::enforce::Os;

/// Protected Windows process names (lowercase).
pub const WINDOWS: &[&str] = &[
    // kernel & session
    "system",
    "system idle process",
    "secure system",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    // core services
    "svchost.exe",
    "dwm.exe",
    "fontdrvhost.exe",
    "conhost.exe",
    "wudfhost.exe",
    "wmiprvse.exe",
    "dllhost.exe",
    "spoolsv.exe",
    "audiodg.exe",
    "dashost.exe",
    "searchindexer.exe",
    "searchprotocolhost.exe",
    "searchfilterhost.exe",
    // shell & UWP
    "explorer.exe",
    "sihost.exe",
    "taskhostw.exe",
    "runtimebroker.exe",
    "applicationframehost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchui.exe",
    "searchapp.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "widgets.exe",
    "widgetservice.exe",
    "comppkgsrv.exe",
    // logon
    "userinit.exe",
    "logonui.exe",
    // security
    "securityhealthservice.exe",
    "securityhealthsystray.exe",
    "securityhealthhost.exe",
    "smartscreen.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "mpcmdrun.exe",
    // input & accessibility
    "ctfmon.exe",
    "tabletinputservice.exe",
    // networking
    "lsm.exe",
    "networkservice.exe",
    "localservice.exe",
    // tools & shells
    "tasklist.exe",
    "taskmgr.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "windowsterminal.exe",
    "wt.exe",
    "openssh.exe",
    "sshd.exe",
    // Guardian 3.0
    "guardian-agent.exe",
    "guardian-worker.exe",
    "guardian-setup.exe",
    "guardian-console.exe",
];

/// Protected Linux process names (lowercase).
pub const LINUX: &[&str] = &[
    "init",
    "systemd",
    "systemd-journald",
    "systemd-logind",
    "systemd-udevd",
    "systemd-resolved",
    "systemd-networkd",
    "systemd-timesyncd",
    "dbus-daemon",
    "dbus-broker",
    "polkitd",
    "networkmanager",
    "wpa_supplicant",
    "sshd",
    "cron",
    "crond",
    "login",
    "agetty",
    "sh",
    "bash",
    "zsh",
    "xorg",
    "xwayland",
    "gnome-shell",
    "gnome-session-binary",
    "kwin_wayland",
    "kwin_x11",
    "plasmashell",
    "sddm",
    "gdm",
    "gdm3",
    "lightdm",
    "pipewire",
    "wireplumber",
    "pulseaudio",
    "guardian-agent",
    "guardian-worker",
    "guardian-setup",
];

/// Exact match of an already-lowercased `name` against the protected list of `os`.
pub fn is_protected(os: Os, name: &str) -> bool {
    let list = match os {
        Os::Windows => WINDOWS,
        Os::Linux => LINUX,
    };
    list.contains(&name)
}
