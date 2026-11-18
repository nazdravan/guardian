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

/// Adds the rule (replacing an old one). Without administrator rights, asks for
/// them with a UAC prompt (`Start-Process netsh -Verb RunAs`).
#[cfg(windows)]
pub fn open_firewall(port: u16) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let netsh = |args: &[String]| {
        std::process::Command::new("netsh")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    };
    let _ = netsh(&firewall_delete_args());
    match netsh(&firewall_args(port)) {
        Ok(o) if o.status.success() => return Ok(()),
        Ok(_) | Err(_) => {}
    }
    let quoted = |args: Vec<String>| {
        args.iter()
            .map(|a| format!("'\"{}\"'", a.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };
    let script = format!(
        "Start-Process netsh -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList {}; \
         Start-Process netsh -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList {}",
        quoted(firewall_delete_args()),
        quoted(firewall_args(port))
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
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
    std::process::Command::new("netsh")
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
