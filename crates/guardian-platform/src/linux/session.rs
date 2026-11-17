//! The active graphical session through `loginctl`, and starting the worker in it.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{PlatformError, UserSession};

fn loginctl(args: &[&str]) -> Result<String, PlatformError> {
    let out = Command::new("loginctl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(PlatformError::Io)?;
    if !out.status.success() {
        return Err(PlatformError::Os(format!(
            "loginctl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The session described by `loginctl show-session <id> -p …` output, if it is
/// an active, local, graphical user session.
pub fn parse_show_session(id: &str, text: &str) -> Option<UserSession> {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
            .map(str::trim)
    };
    if get("Active") != Some("yes") || get("Remote") == Some("yes") || get("Class") != Some("user")
    {
        return None;
    }
    let ty = get("Type")?;
    if ty != "x11" && ty != "wayland" {
        return None;
    }
    let uid: u32 = get("User")?.parse().ok()?;
    let name = get("Name").unwrap_or_default().to_string();
    let run = format!("/run/user/{uid}");
    let mut env = vec![
        ("XDG_RUNTIME_DIR".to_string(), run.clone()),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            format!("unix:path={run}/bus"),
        ),
        ("GUARDIAN_SESSION".to_string(), id.to_string()),
        ("USER".to_string(), name.clone()),
        ("HOME".to_string(), format!("/home/{name}")),
        (
            "PATH".to_string(),
            "/usr/local/bin:/usr/bin:/bin".to_string(),
        ),
    ];
    if let Some(d) = get("Display").filter(|d| !d.is_empty()) {
        env.push(("DISPLAY".to_string(), d.to_string()));
    }
    if ty == "wayland" {
        env.push(("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()));
    }
    Some(UserSession {
        id: id.to_string(),
        user: name,
        uid: Some(uid),
        env,
    })
}

/// The first active local graphical session.
pub fn active() -> Result<Option<UserSession>, PlatformError> {
    let list = loginctl(&["list-sessions", "--no-legend"])?;
    for id in list.lines().filter_map(|l| l.split_whitespace().next()) {
        let Ok(text) = loginctl(&[
            "show-session",
            id,
            "-p",
            "Active",
            "-p",
            "Remote",
            "-p",
            "Type",
            "-p",
            "Class",
            "-p",
            "Name",
            "-p",
            "User",
            "-p",
            "Display",
        ]) else {
            continue;
        };
        if let Some(s) = parse_show_session(id, &text) {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// Starts `exe args` as the session's user (uid and primary gid only).
pub fn spawn(s: &UserSession, exe: &Path, args: &[String]) -> Result<u32, PlatformError> {
    let uid = s
        .uid
        .ok_or_else(|| PlatformError::Os("session without uid".into()))?;
    let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .map_err(|e| PlatformError::Os(e.to_string()))?
        .ok_or(PlatformError::NotFound)?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .env_clear()
        .envs(s.env.iter().map(|(k, v)| (k, v)))
        .env("HOME", &user.dir)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .gid(user.gid.as_raw())
        .uid(uid);
    let mut child = cmd.spawn().map_err(PlatformError::Io)?;
    let pid = child.id();
    // Reap it when it exits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wayland_session() {
        let text =
            "Active=yes\nRemote=no\nType=wayland\nClass=user\nName=elev\nUser=1000\nDisplay=\n";
        let s = parse_show_session("3", text).unwrap();
        assert_eq!(
            (s.id.as_str(), s.user.as_str(), s.uid),
            ("3", "elev", Some(1000))
        );
        assert!(
            s.env
                .contains(&("WAYLAND_DISPLAY".into(), "wayland-0".into()))
        );
        assert!(
            s.env
                .contains(&("XDG_RUNTIME_DIR".into(), "/run/user/1000".into()))
        );
        assert!(!s.env.iter().any(|(k, _)| k == "DISPLAY"));
    }

    #[test]
    fn ignores_inactive_remote_and_tty_sessions() {
        let base = "Active=yes\nRemote=no\nType=x11\nClass=user\nName=e\nUser=1000\nDisplay=:0\n";
        assert!(parse_show_session("1", base).is_some());
        assert!(parse_show_session("1", &base.replace("Active=yes", "Active=no")).is_none());
        assert!(parse_show_session("1", &base.replace("Remote=no", "Remote=yes")).is_none());
        assert!(parse_show_session("1", &base.replace("Type=x11", "Type=tty")).is_none());
        assert!(parse_show_session("1", &base.replace("Class=user", "Class=greeter")).is_none());
    }
}
