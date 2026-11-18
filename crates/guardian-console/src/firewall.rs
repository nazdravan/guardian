//! The Windows Firewall rule that lets agents on the local network reach the
//! embedded server.

/// Rule name.
pub const RULE: &str = "Guardian Console server";

/// `netsh` arguments that add the rule: TCP inbound on `port`, every network
/// profile (school networks are often "Public"), local subnet only.
pub fn firewall_args(port: u16) -> Vec<String> {
    [
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE}"),
        "dir=in",
        "action=allow",
        "protocol=TCP",
        &format!("localport={port}"),
        "profile=any",
        "remoteip=localsubnet",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `netsh` arguments that delete the rule.
pub fn firewall_delete_args() -> Vec<String> {
    [
        "advfirewall",
        "firewall",
        "delete",
        "rule",
        &format!("name={RULE}"),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `%SystemRoot%` (default `C:\Windows`).
pub fn system_root() -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .filter(|v| !v.is_empty())
        .map_or_else(
            || std::path::PathBuf::from(r"C:\Windows"),
            std::path::PathBuf::from,
        )
}

/// A system tool by absolute path (`<root>\System32\<name>`): a bare name would
/// also be found next to the console's own exe, e.g. in Downloads.
pub fn system_tool(system_root: &std::path::Path, name: &str) -> std::path::PathBuf {
    system_root.join("System32").join(name)
}

fn cmd_line(netsh: &std::path::Path, args: &[String]) -> String {
    let quoted: Vec<String> = args
        .iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect();
    format!("\"{}\" {}", netsh.display(), quoted.join(" "))
}

/// The PowerShell command that deletes and re-adds the rule in **one** elevated
/// process (one UAC prompt; the old rule is only deleted if the prompt is
/// accepted).
pub fn elevated_script(system_root: &std::path::Path, port: u16) -> String {
    let netsh = system_tool(system_root, "netsh.exe");
    let cmd = system_tool(system_root, "cmd.exe");
    let line = format!(
        "/c {} & {}",
        cmd_line(&netsh, &firewall_delete_args()),
        cmd_line(&netsh, &firewall_args(port))
    );
    format!(
        "Start-Process -FilePath '{}' -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '{}'",
        cmd.display(),
        line.replace('\'', "''")
    )
}

/// Adds the rule (replacing an old one). Without administrator rights, asks for
/// them with one UAC prompt.
#[cfg(windows)]
pub fn open_firewall(port: u16) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let root = system_root();
    let netsh = |args: &[String]| {
        std::process::Command::new(system_tool(&root, "netsh.exe"))
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    };
    let _ = netsh(&firewall_delete_args());
    if netsh(&firewall_args(port)).is_ok_and(|o| o.status.success()) {
        return Ok(());
    }
    let powershell = root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let out = std::process::Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &elevated_script(&root, port),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
    if out.status.success() && rule_exists() {
        Ok(())
    } else {
        Err("The firewall rule was not created (the administrator prompt was declined?).".into())
    }
}

/// Whether the rule is there.
#[cfg(windows)]
pub fn rule_exists() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new(system_tool(&system_root(), "netsh.exe"))
        .args([
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &format!("name={RULE}"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}
