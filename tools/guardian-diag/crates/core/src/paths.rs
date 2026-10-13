//! Path helpers ported from tools/whitelist-gui/paths.go.

use std::path::{Path, PathBuf};

/// A service ImagePath without quotes and arguments: `"C:\a b\x.exe" -run` → `C:\a b\x.exe`.
pub fn unquote_image_path(v: &str) -> String {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => rest[..end].to_string(),
            None => rest.to_string(),
        };
    }
    match v.to_lowercase().find(".exe") {
        Some(i) => v[..i + 4].to_string(),
        None => v.to_string(),
    }
}

/// The path this process can actually open. A 32-bit process on 64-bit Windows is
/// redirected from System32 to SysWOW64; `Sysnative` is the escape hatch. Everything
/// shown to the user stays canonical (System32); this is applied only at I/O time.
pub fn io_path(canonical: &Path, wow64: bool, windir: &str) -> PathBuf {
    if !wow64 {
        return canonical.to_path_buf();
    }
    let s = canonical.to_string_lossy();
    let sys32 = format!("{}\\System32", windir.trim_end_matches('\\'));
    if s.len() >= sys32.len() && s.is_char_boundary(sys32.len()) && s[..sys32.len()].eq_ignore_ascii_case(&sys32) {
        return PathBuf::from(format!("{}\\Sysnative{}", windir.trim_end_matches('\\'), &s[sys32.len()..]));
    }
    canonical.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquote_image_path_cases() {
        assert_eq!(
            unquote_image_path(r#""C:\Windows\System32\ProcSentinel\agent\procsentinel-agent64.exe""#),
            r"C:\Windows\System32\ProcSentinel\agent\procsentinel-agent64.exe"
        );
        assert_eq!(unquote_image_path(r"C:\x\agent.EXE -service"), r"C:\x\agent.EXE");
        assert_eq!(unquote_image_path(r#""C:\x\a.exe"#), r"C:\x\a.exe");
    }

    #[test]
    fn io_path_rewrites_system32_only_under_wow64() {
        let p = Path::new(r"C:\Windows\System32\ProcSentinel\agent\.env");
        assert_eq!(io_path(p, false, r"C:\Windows"), p);
        assert_eq!(io_path(p, true, r"C:\Windows"), PathBuf::from(r"C:\Windows\Sysnative\ProcSentinel\agent\.env"));
        assert_eq!(io_path(Path::new(r"c:\windows\system32\x"), true, r"C:\Windows"), PathBuf::from(r"C:\Windows\Sysnative\x"));
        assert_eq!(io_path(Path::new(r"D:\agent\.env"), true, r"C:\Windows"), PathBuf::from(r"D:\agent\.env"));
    }
}
