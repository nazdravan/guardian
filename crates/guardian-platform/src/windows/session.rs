//! The console session and starting the worker in it.

use std::path::Path;

use super::ffi_session;
use crate::{PlatformError, UserSession};

/// Quotes one argument for a Windows command line (CommandLineToArgvW rules).
pub fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The console session with a logged-in user.
pub fn active() -> Result<Option<UserSession>, PlatformError> {
    let Some(id) = ffi_session::active_console_session() else {
        return Ok(None);
    };
    let Some(user) = ffi_session::session_user(id) else {
        return Ok(None);
    };
    Ok(Some(UserSession {
        id: id.to_string(),
        user: user.clone(),
        uid: None,
        env: vec![
            ("GUARDIAN_SESSION".into(), id.to_string()),
            ("USERNAME".into(), user),
        ],
    }))
}

/// Starts `exe args` in the session.
pub fn spawn(s: &UserSession, exe: &Path, args: &[String]) -> Result<u32, PlatformError> {
    let id: u32 =
        s.id.parse()
            .map_err(|_| PlatformError::Os(format!("bad session id {:?}", s.id)))?;
    let mut line = quote_arg(&exe.to_string_lossy());
    for a in args {
        line.push(' ');
        line.push_str(&quote_arg(a));
    }
    ffi_session::spawn_as_session_user(id, &line).map_err(|e| PlatformError::Os(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn quoting_follows_windows_rules() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(
            quote_arg(r"C:\Program Files\Guardian\w.exe"),
            r#""C:\Program Files\Guardian\w.exe""#
        );
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r"end\ "), r#""end\ ""#);
        assert_eq!(quote_arg(r"x y\"), r#""x y\\""#);
        assert_eq!(quote_arg(""), r#""""#);
    }
}
