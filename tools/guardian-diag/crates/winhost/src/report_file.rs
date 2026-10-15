//! Saving the report next to the exe (spec R2, R24).

use std::path::{Path, PathBuf};

use guardian_core::diagnose::diagnose;
use guardian_core::report::report;
use guardian_core::snapshot::Snapshot;

/// `<dir>\<hostname>-<YYYYMMDD-HHMM>.txt` and `.json` (spec R24). Returns the .txt path.
/// Never overwrites: if either file exists the stem gets `-1`, `-2`, ….
pub fn save_report_to(s: &Snapshot, dir: &Path) -> Result<PathBuf, String> {
    let r = report(s, &diagnose(s));
    let json = serde_json::to_string_pretty(&r.json).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let host = if s.machine.hostname.is_empty() { "computer" } else { s.machine.hostname.as_str() };
    let base = format!("{host}-{}", s.taken_at.with_timezone(&chrono::Local).format("%Y%m%d-%H%M"));
    let mut n = 0u32;
    loop {
        let stem = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        let (txt, js) = (dir.join(format!("{stem}.txt")), dir.join(format!("{stem}.json")));
        if txt.exists() || js.exists() {
            n += 1;
            continue;
        }
        // create_new closes the race with another instance writing the same stem.
        match write_new(&txt, r.text.as_bytes()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                n += 1;
                continue;
            }
            Err(e) => return Err(e.to_string()),
        }
        write_new(&js, json.as_bytes()).map_err(|e| e.to_string())?;
        return Ok(txt);
    }
}

fn write_new(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(p)?.write_all(bytes)
}

/// Next to the exe, in GuardianReports\ (spec R2, R24).
pub fn save_report(s: &Snapshot) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("the exe has no folder")?.join("GuardianReports");
    save_report_to(s, &dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        guardian_core::testkit::snapshot_from_dir(&guardian_core::testkit::fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }

    #[test]
    fn writes_txt_and_json_without_secrets() {
        let s = snap();
        let d = tempfile::tempdir().unwrap();
        let p = save_report_to(&s, d.path()).unwrap();
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("LAB-A-05-2026101"));
        let txt = std::fs::read_to_string(&p).unwrap();
        assert!(!txt.contains("agt_secret_value_123"));
        assert!(p.with_extension("json").exists());
    }

    #[test]
    fn never_overwrites_an_existing_report() {
        let s = snap();
        let d = tempfile::tempdir().unwrap();
        let a = save_report_to(&s, d.path()).unwrap();
        let b = save_report_to(&s, d.path()).unwrap();
        let c = save_report_to(&s, d.path()).unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert!(b.file_stem().unwrap().to_string_lossy().ends_with("-1"));
        assert!(c.file_stem().unwrap().to_string_lossy().ends_with("-2"));
        assert!(b.with_extension("json").exists() && c.with_extension("json").exists());
        // A stray .json alone also forces a new stem.
        let d2 = tempfile::tempdir().unwrap();
        let first = save_report_to(&s, d2.path()).unwrap();
        std::fs::remove_file(&first).unwrap();
        let again = save_report_to(&s, d2.path()).unwrap();
        assert_ne!(first, again);
    }
}
