//! Minimal-diff rendering (spec R15): every byte not being edited survives.

use crate::read::{Encoding, LineEnding, detect_encoding, detect_line_ending};

pub const MAX_BYTES: usize = 1_048_576;
pub const EMPTY_WHITELIST_COMMENT: &str = "# Local whitelist is empty. Do not delete this file: the agent regenerates it on start.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    Encoding(&'static str),
    TooLarge,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Encoding(e) => write!(f, "Cannot edit: the file is {e}"),
            RenderError::TooLarge => write!(f, "Cannot edit: the file is larger than 1 MB"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvChange {
    Set(&'static str, String),
    Remove(&'static str),
}

/// Refuses anything that isn't UTF-8. A UTF-8 BOM stays *inside* the text as U+FEFF,
/// exactly as the agent sees it: the first line's key is then `\u{feff}KEY`, which the
/// agent does not recognise either, so a `Set` appends a fresh, readable line and the
/// BOM survives untouched at the start of the file.
fn decode(original: &[u8]) -> Result<&str, RenderError> {
    if original.len() > MAX_BYTES {
        return Err(RenderError::TooLarge);
    }
    match detect_encoding(original) {
        Encoding::Utf16 => Err(RenderError::Encoding("UTF-16")),
        Encoding::Invalid => Err(RenderError::Encoding("not UTF-8")),
        Encoding::Utf8 | Encoding::Utf8Bom => std::str::from_utf8(original).map_err(|_| RenderError::Encoding("not UTF-8")),
    }
}

fn eol(text: &str) -> &'static str {
    match detect_line_ending(text) {
        LineEnding::Crlf => "\r\n",
        LineEnding::Lf => "\n",
    }
}

/// Key of an agent-visible `KEY=value` line, as readEnvFile would split it.
fn line_key(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    t.split_once('=').map(|(k, _)| k.trim())
}

fn line_ending_of(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn finish(lines: Vec<String>) -> Vec<u8> {
    lines.concat().into_bytes()
}

fn push_line(lines: &mut Vec<String>, text: &str, e: &str) {
    if let Some(last) = lines.last_mut()
        && !last.ends_with('\n')
    {
        last.push_str(e);
    }
    lines.push(format!("{text}{e}"));
}

pub fn render_env(original: &[u8], changes: &[EnvChange]) -> Result<Vec<u8>, RenderError> {
    let text = decode(original)?;
    let e = eol(text);
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    for ch in changes {
        match ch {
            EnvChange::Set(key, value) => {
                let last = lines.iter().rposition(|l| line_key(l) == Some(key));
                match last {
                    Some(i) => {
                        let ending = line_ending_of(&lines[i]).to_string();
                        lines[i] = format!("{key}={value}{ending}");
                    }
                    None => push_line(&mut lines, &format!("{key}={value}"), e),
                }
            }
            EnvChange::Remove(key) => lines.retain(|l| line_key(l) != Some(key)),
        }
    }
    Ok(finish(lines))
}

pub const SCRUBBED_VALUE: &str = "<removed by Guardian>";

/// Keys whose values must never be copied into a backup (R14.3, R4).
const SECRET_ENV_KEYS: &[&str] = &["BINDING_TOKEN"];

/// Agent-visible key of one raw line (bytes, any encoding): trimmed, `#` comments skipped,
/// split at the first `=` — the same rule as the agent's readEnvFile/removeEnvKey.
fn raw_line_key(line: &[u8]) -> Option<String> {
    let trimmed = match std::str::from_utf8(line) {
        Ok(s) => s.trim().as_bytes(),
        Err(_) => line.trim_ascii(),
    };
    if trimmed.is_empty() || trimmed.starts_with(b"#") {
        return None;
    }
    let eq = trimmed.iter().position(|&b| b == b'=')?;
    let key = &trimmed[..eq];
    let key = match std::str::from_utf8(key) {
        Ok(s) => s.trim().to_string(),
        Err(_) => String::from_utf8_lossy(key.trim_ascii()).into_owned(),
    };
    Some(key)
}

/// A copy of `.env` safe to leave on disk as a backup: the value of every agent-visible
/// `BINDING_TOKEN=` line is replaced by `<removed by Guardian>`; every other byte — the key,
/// its spacing, comments, line endings, an invalid encoding — is kept as it was.
pub fn scrub_env_secrets(original: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(original.len());
    for line in original.split_inclusive(|&b| b == b'\n') {
        let secret = raw_line_key(line).is_some_and(|k| SECRET_ENV_KEYS.contains(&k.as_str()));
        if !secret {
            out.extend_from_slice(line);
            continue;
        }
        let ending: &[u8] = if line.ends_with(b"\r\n") {
            b"\r\n"
        } else if line.ends_with(b"\n") {
            b"\n"
        } else {
            b""
        };
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            out.extend_from_slice(line);
            continue;
        };
        out.extend_from_slice(&line[..=eq]);
        out.extend_from_slice(SCRUBBED_VALUE.as_bytes());
        out.extend_from_slice(ending);
    }
    out
}

pub fn render_whitelist(original: &[u8], entries: &[String]) -> Result<Vec<u8>, RenderError> {
    let text = decode(original)?;
    let e = eol(text);
    let wanted: Vec<String> = entries.iter().map(|s| s.to_lowercase()).collect();
    let mut emitted: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for l in text.split_inclusive('\n') {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            lines.push(l.to_string());
            continue;
        }
        let lower = t.to_lowercase();
        if wanted.contains(&lower) && !emitted.contains(&lower) {
            emitted.push(lower);
            lines.push(l.to_string());
        }
    }
    for w in &wanted {
        if !emitted.contains(w) {
            emitted.push(w.clone());
            push_line(&mut lines, w, e);
        }
    }
    if lines.iter().all(|l| l.trim().is_empty()) {
        lines = vec![format!("{EMPTY_WHITELIST_COMMENT}{e}")];
    }
    Ok(finish(lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(b: Vec<u8>) -> String {
        String::from_utf8(b).unwrap()
    }

    #[test]
    fn render_env_changes_only_the_last_occurrence() {
        let src = b"# head\r\nSERVER_ADDRESS=https://a\r\nPROXY=x\r\nSERVER_ADDRESS = https://b\r\nCHECK_INTERVAL=30\r\n";
        let out = render_env(src, &[EnvChange::Set("SERVER_ADDRESS", "https://c".into())]).unwrap();
        assert_eq!(s(out), "# head\r\nSERVER_ADDRESS=https://a\r\nPROXY=x\r\nSERVER_ADDRESS=https://c\r\nCHECK_INTERVAL=30\r\n");
    }

    #[test]
    fn render_env_appends_missing_key_with_the_files_ending() {
        assert_eq!(s(render_env(b"A=1\nB=2", &[EnvChange::Set("CHECK_INTERVAL", "15".into())]).unwrap()), "A=1\nB=2\nCHECK_INTERVAL=15\n");
        assert_eq!(s(render_env(b"", &[EnvChange::Set("SERVER_ADDRESS", "https://a".into())]).unwrap()), "SERVER_ADDRESS=https://a\r\n");
    }

    #[test]
    fn render_env_remove_drops_every_occurrence() {
        let out =
            render_env(b"BINDING_TOKEN=a\r\nX=1\r\n  BINDING_TOKEN = b\r\n#BINDING_TOKEN=c\r\n", &[EnvChange::Remove("BINDING_TOKEN")]).unwrap();
        assert_eq!(s(out), "X=1\r\n#BINDING_TOKEN=c\r\n");
    }

    #[test]
    fn render_env_keeps_bom_and_refuses_utf16() {
        let out = render_env(b"\xEF\xBB\xBFX=1\r\n", &[EnvChange::Set("CHECK_INTERVAL", "9".into())]).unwrap();
        assert_eq!(out, b"\xEF\xBB\xBFX=1\r\nCHECK_INTERVAL=9\r\n");
        assert!(matches!(render_env(b"\xFF\xFEX\0", &[]), Err(RenderError::Encoding(_))));
        assert!(matches!(render_env(&vec![b'a'; 1_048_577], &[]), Err(RenderError::TooLarge)));
    }

    #[test]
    fn render_whitelist_keeps_comments_order_and_case() {
        let src = b"# local\r\nNotepad.exe\r\n\r\ngame.exe\r\ncalc.exe\r\n";
        let out = render_whitelist(src, &["notepad.exe".into(), "calc.exe".into(), "winword.exe".into()]).unwrap();
        assert_eq!(s(out), "# local\r\nNotepad.exe\r\n\r\ncalc.exe\r\nwinword.exe\r\n");
    }

    #[test]
    fn render_whitelist_never_empty() {
        assert_eq!(s(render_whitelist(b"a.exe\n", &[]).unwrap()), format!("{EMPTY_WHITELIST_COMMENT}\n"));
        assert_eq!(s(render_whitelist(b"# keep\na.exe\n", &[]).unwrap()), "# keep\n");
        assert_eq!(s(render_whitelist(b"", &[]).unwrap()), format!("{EMPTY_WHITELIST_COMMENT}\r\n"));
    }
    #[test]
    fn scrub_env_secrets_replaces_only_agent_visible_token_values() {
        let src =
            b"# BINDING_TOKEN=keep-comment\r\nSERVER_ADDRESS=https://a\r\n  BINDING_TOKEN = abc=def \r\nBINDING_TOKENX=keep\r\nBINDING_TOKEN=last";
        let out = scrub_env_secrets(src);
        assert_eq!(
            s(out),
            "# BINDING_TOKEN=keep-comment\r\nSERVER_ADDRESS=https://a\r\n  BINDING_TOKEN =<removed by Guardian>\r\nBINDING_TOKENX=keep\r\nBINDING_TOKEN=<removed by Guardian>"
        );
    }

    #[test]
    fn scrub_env_secrets_keeps_everything_else_byte_identical() {
        let src: &[u8] = b"\xEF\xBB\xBFX=1\nY=\xFF\xFE\n\n#c\nBINDING_TOKEN=t\n";
        assert_eq!(scrub_env_secrets(src), b"\xEF\xBB\xBFX=1\nY=\xFF\xFE\n\n#c\nBINDING_TOKEN=<removed by Guardian>\n");
        let clean: &[u8] = b"A=1\r\nB\r\n";
        assert_eq!(scrub_env_secrets(clean), clean);
        assert_eq!(scrub_env_secrets(b""), b"");
    }
}
