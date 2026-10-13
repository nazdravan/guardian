use std::path::Path;

use chrono::{DateTime, Utc};
use guardian_core::read::{Read, SourceFile};

pub const MAX_FILE: u64 = 1_048_576;

pub fn read_capped(io: &Path) -> Read<(Vec<u8>, Option<DateTime<Utc>>)> {
    let meta = match std::fs::metadata(io) {
        Ok(m) => m,
        Err(e) => return from_io(e),
    };
    if meta.len() > MAX_FILE {
        return Read::Failed(format!("larger than 1 MB ({} bytes)", meta.len()));
    }
    match std::fs::read(io) {
        Ok(b) => Read::Ok((b, meta.modified().ok().map(DateTime::<Utc>::from))),
        Err(e) => from_io(e),
    }
}

pub fn from_io<T>(e: std::io::Error) -> Read<T> {
    match e.kind() {
        std::io::ErrorKind::NotFound => Read::Missing,
        std::io::ErrorKind::PermissionDenied => Read::Denied,
        _ => Read::Failed(e.to_string()),
    }
}

pub fn read_source<T>(io: &Path, parse: impl FnOnce(&[u8]) -> T) -> Read<SourceFile<T>> {
    read_capped(io).map(|(b, m)| SourceFile::new(b, m, parse))
}

/// For parsers that can fail (JSON).
pub fn read_source_fallible<T>(io: &Path, parse: impl FnOnce(&[u8]) -> Result<T, String>) -> Read<SourceFile<T>> {
    match read_capped(io) {
        Read::Ok((b, m)) => match parse(&b) {
            Ok(v) => Read::Ok(SourceFile::new(b, m, |_| v)),
            Err(e) => Read::Failed(e),
        },
        Read::Missing => Read::Missing,
        Read::Denied => Read::Denied,
        Read::Failed(e) => Read::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::read::Read;

    #[test]
    fn read_capped_cases() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read_capped(&d.path().join("nope")), Read::Missing);
        std::fs::write(d.path().join("a"), b"x").unwrap();
        assert!(matches!(read_capped(&d.path().join("a")), Read::Ok((b, Some(_))) if b == b"x"));
        std::fs::write(d.path().join("big"), vec![b'a'; (MAX_FILE + 1) as usize]).unwrap();
        assert!(matches!(read_capped(&d.path().join("big")), Read::Failed(m) if m.contains("1 MB")));
    }
}
