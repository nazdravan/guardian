//! Outcome of reading one source, and helpers for secrets and hashes.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// Tabs and the report distinguish "no file", "no access" and "broken" honestly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read<T> {
    Ok(T),
    Missing,
    Denied,
    Failed(String),
}

impl<T> Read<T> {
    pub fn as_ok(&self) -> Option<&T> {
        match self {
            Read::Ok(v) => Some(v),
            _ => None,
        }
    }
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Read<U> {
        match self {
            Read::Ok(v) => Read::Ok(f(v)),
            Read::Missing => Read::Missing,
            Read::Denied => Read::Denied,
            Read::Failed(e) => Read::Failed(e),
        }
    }
    /// "absent" / "no access" / the error, for display.
    pub fn problem(&self) -> Option<String> {
        match self {
            Read::Ok(_) => None,
            Read::Missing => Some("absent".into()),
            Read::Denied => Some("no access".into()),
            Read::Failed(e) => Some(e.clone()),
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The last 4 hex chars of a secret's SHA-256: enough to compare two files, useless to an attacker.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SecretTail(pub String);

impl SecretTail {
    pub fn of(secret: &str) -> Self {
        let h = sha256_hex(secret.as_bytes());
        SecretTail(h[h.len() - 4..].to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Crlf,
    Lf,
}

pub fn detect_encoding(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Encoding::Utf8Bom
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        Encoding::Utf16
    } else if std::str::from_utf8(bytes).is_ok() {
        Encoding::Utf8
    } else {
        Encoding::Invalid
    }
}

/// Predominant line ending; CRLF for a file with none (what the installer writes).
pub fn detect_line_ending(text: &str) -> LineEnding {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if lf > crlf { LineEnding::Lf } else { LineEnding::Crlf }
}

/// A file as read from disk: raw bytes (for rendering an edit), their hash (to detect a
/// change before writing), mtime, and the parsed view. `Debug` never prints the bytes:
/// `.env` can hold a binding token.
#[derive(Clone, PartialEq)]
pub struct SourceFile<T> {
    pub bytes: Vec<u8>,
    pub hash: String,
    pub mtime: Option<DateTime<Utc>>,
    pub parsed: T,
}

impl<T> SourceFile<T> {
    pub fn new(bytes: Vec<u8>, mtime: Option<DateTime<Utc>>, parse: impl FnOnce(&[u8]) -> T) -> Self {
        let parsed = parse(&bytes);
        let hash = sha256_hex(&bytes);
        SourceFile { bytes, hash, mtime, parsed }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for SourceFile<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceFile")
            .field("len", &self.bytes.len())
            .field("hash", &self.hash)
            .field("mtime", &self.mtime)
            .field("parsed", &self.parsed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_tail_is_four_hex_chars() {
        let t = SecretTail::of("abc");
        assert_eq!(t.0, "15ad"); // sha256("abc") ends with …f20015ad
    }

    #[test]
    fn source_file_debug_hides_bytes() {
        let f = SourceFile::new(b"BINDING_TOKEN=supersecret".to_vec(), None, |_| ());
        assert!(!format!("{f:?}").contains("supersecret"));
    }

    #[test]
    fn encodings_and_endings() {
        assert_eq!(detect_encoding(b"\xEF\xBB\xBFa=b"), Encoding::Utf8Bom);
        assert_eq!(detect_encoding(b"\xFF\xFEa\0"), Encoding::Utf16);
        assert_eq!(detect_encoding(b"\xC3"), Encoding::Invalid);
        assert_eq!(detect_line_ending("a\r\nb\r\nc\n"), LineEnding::Crlf);
        assert_eq!(detect_line_ending("a\nb\n"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a"), LineEnding::Crlf);
    }
}
