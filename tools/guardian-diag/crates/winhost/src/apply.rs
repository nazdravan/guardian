//! Writing a WritePlan: change check → backup → temp + sync → rename (with retries) → verify → rotate.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local};
use guardian_core::edit::plan::{Expected, FileKind, FileWrite, WritePlan};
use guardian_core::read::sha256_hex;

pub const KEEP_BACKUPS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Changed,
    Backup,
    Write,
    Replace,
    Verify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub kind: FileKind,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub written: Vec<Written>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    pub kind: FileKind,
    pub step: Step,
    pub backup: Option<PathBuf>,
    pub written_before: Vec<FileKind>,
    pub message: String,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = self.kind.file_name();
        match self.step {
            Step::Changed => write!(f, "{what} changed on disk since it was loaded")?,
            Step::Backup => write!(f, "Could not back up {what}: {}", self.message)?,
            Step::Write => write!(f, "Could not write {what}: {}", self.message)?,
            Step::Replace => write!(f, "Could not replace {what}: {}", self.message)?,
            Step::Verify => write!(f, "{what} was written but reads back differently: {}", self.message)?,
        }
        if let Some(b) = &self.backup {
            write!(f, ". The previous version is in {}", b.display())?;
        }
        if !self.written_before.is_empty() {
            let names: Vec<&str> = self.written_before.iter().map(|k| k.file_name()).collect();
            write!(f, ". Already saved: {}", names.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for ApplyError {}

fn current_hash(io: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read(io) {
        Ok(b) => Ok(Some(sha256_hex(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn backup_name(file: &str, now: DateTime<Local>, dir_io: &Path) -> String {
    let base = format!("{file}.bak-{}", now.format("%Y%m%d-%H%M%S"));
    if !dir_io.join(&base).exists() {
        return base;
    }
    (1..).map(|i| format!("{base}-{i}")).find(|n| !dir_io.join(n).exists()).unwrap_or(base)
}

fn rotate(dir_io: &Path, file: &str) {
    let prefix = format!("{file}.bak-");
    let Ok(rd) = std::fs::read_dir(dir_io) else { return };
    let mut baks: Vec<String> =
        rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&prefix)).collect();
    // Names sort chronologically (timestamp, then -N suffix).
    baks.sort_by(|a, b| b.cmp(a));
    for old in baks.iter().skip(KEEP_BACKUPS) {
        let _ = std::fs::remove_file(dir_io.join(old));
    }
}

fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..3 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("rename failed")))
}

fn apply_one(fw: &FileWrite, io: &dyn Fn(&Path) -> PathBuf, overwrite: bool, now: DateTime<Local>, done: &[FileKind]) -> Result<Written, ApplyError> {
    let fail = |step, backup: Option<PathBuf>, message: String| ApplyError { kind: fw.kind, step, backup, written_before: done.to_vec(), message };
    let target = io(&fw.path);
    let dir_canon = fw.path.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir_io = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = fw.kind.file_name();

    let cur = current_hash(&target).map_err(|e| fail(Step::Changed, None, e.to_string()))?;
    if !overwrite {
        let same = match (&fw.expected, &cur) {
            (Expected::Absent, None) => true,
            (Expected::Hash(h), Some(c)) => h == c,
            _ => false,
        };
        if !same {
            return Err(fail(Step::Changed, None, String::new()));
        }
    }

    let backup = if cur.is_some() {
        let bn = backup_name(name, now, &dir_io);
        std::fs::copy(&target, dir_io.join(&bn)).map_err(|e| fail(Step::Backup, None, e.to_string()))?;
        Some(dir_canon.join(&bn))
    } else {
        None
    };

    let tmp = dir_io.join(format!("{name}.guardian-tmp-{}", std::process::id()));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&fw.new_bytes)?;
        f.sync_all()
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(Step::Write, backup, e.to_string()));
    }
    if let Err(e) = rename_with_retry(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(Step::Replace, backup, e.to_string()));
    }
    match std::fs::read(&target) {
        Ok(b) if b == fw.new_bytes => {}
        Ok(_) => return Err(fail(Step::Verify, backup, "content differs".into())),
        Err(e) => return Err(fail(Step::Verify, backup, e.to_string())),
    }
    rotate(&dir_io, name);
    Ok(Written { kind: fw.kind, backup })
}

pub fn apply(plan: &WritePlan, io: &dyn Fn(&Path) -> PathBuf, overwrite: bool, now: DateTime<Local>) -> Result<Applied, ApplyError> {
    let mut written = Vec::new();
    let mut done = Vec::new();
    for fw in &plan.files {
        let w = apply_one(fw, io, overwrite, now, &done)?;
        done.push(w.kind);
        written.push(w);
    }
    Ok(Applied { written })
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::edit::plan::{Expected, FileKind, FileWrite, WritePlan};
    use guardian_core::read::sha256_hex;
    use std::path::{Path, PathBuf};

    fn id(p: &Path) -> PathBuf {
        p.to_path_buf()
    }
    fn at(h: u32, m: u32, s: u32) -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        chrono::Local.with_ymd_and_hms(2026, 10, 14, h, m, s).unwrap()
    }
    fn fw(dir: &Path, kind: FileKind, old: Option<&[u8]>, new: &[u8]) -> FileWrite {
        FileWrite {
            kind,
            path: dir.join(kind.file_name()),
            expected: old.map_or(Expected::Absent, |o| Expected::Hash(sha256_hex(o))),
            new_bytes: new.to_vec(),
            summary: vec![],
        }
    }

    #[test]
    fn apply_writes_backs_up_and_verifies() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".env"), b"A=1\r\n").unwrap();
        let plan = WritePlan { files: vec![fw(d.path(), FileKind::Env, Some(b"A=1\r\n"), b"A=2\r\n")] };
        let r = apply(&plan, &id, false, at(14, 12, 0)).unwrap();
        assert_eq!(std::fs::read(d.path().join(".env")).unwrap(), b"A=2\r\n");
        let b = r.written[0].backup.clone().unwrap();
        assert_eq!(b.file_name().unwrap(), ".env.bak-20261014-141200");
        assert_eq!(std::fs::read(&b).unwrap(), b"A=1\r\n");
        assert!(std::fs::read_dir(d.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains("guardian-tmp")));
    }

    #[test]
    fn apply_refuses_when_file_changed() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".env"), b"A=1\r\nBINDING_TOKEN=x\r\n").unwrap();
        // Snapshot saw the token; the agent has since removed it.
        std::fs::write(d.path().join(".env"), b"A=1\r\n").unwrap();
        let plan = WritePlan { files: vec![fw(d.path(), FileKind::Env, Some(b"A=1\r\nBINDING_TOKEN=x\r\n"), b"A=2\r\n")] };
        let e = apply(&plan, &id, false, at(14, 12, 0)).unwrap_err();
        assert_eq!(e.step, Step::Changed);
        assert_eq!(std::fs::read(d.path().join(".env")).unwrap(), b"A=1\r\n");
        // Overwrite proceeds.
        apply(&plan, &id, true, at(14, 12, 0)).unwrap();
        assert_eq!(std::fs::read(d.path().join(".env")).unwrap(), b"A=2\r\n");
    }

    #[test]
    fn apply_absent_expectation() {
        let d = tempfile::tempdir().unwrap();
        let plan = WritePlan { files: vec![fw(d.path(), FileKind::Whitelist, None, b"a.exe\r\n")] };
        let r = apply(&plan, &id, false, at(9, 0, 0)).unwrap();
        assert_eq!(r.written[0].backup, None);
        std::fs::write(d.path().join("whitelist.txt"), b"other\r\n").unwrap();
        assert_eq!(apply(&plan, &id, false, at(9, 0, 1)).unwrap_err().step, Step::Changed);
    }

    #[test]
    fn apply_keeps_five_newest_backups() {
        let d = tempfile::tempdir().unwrap();
        let mut cur = b"0".to_vec();
        std::fs::write(d.path().join(".env"), &cur).unwrap();
        for i in 1..=7u32 {
            let next = i.to_string().into_bytes();
            let plan = WritePlan { files: vec![fw(d.path(), FileKind::Env, Some(&cur), &next)] };
            apply(&plan, &id, false, at(10, i, 0)).unwrap();
            cur = next;
        }
        let mut baks: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".env.bak-"))
            .collect();
        baks.sort();
        assert_eq!(baks, (3..=7).map(|i| format!(".env.bak-20261014-10{i:02}00")).collect::<Vec<_>>());
        // whitelist backups are rotated separately and untouched
    }

    #[test]
    fn apply_same_second_backup_gets_suffix() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".env"), b"1").unwrap();
        apply(&WritePlan { files: vec![fw(d.path(), FileKind::Env, Some(b"1"), b"2")] }, &id, false, at(8, 0, 0)).unwrap();
        let r = apply(&WritePlan { files: vec![fw(d.path(), FileKind::Env, Some(b"2"), b"3")] }, &id, false, at(8, 0, 0)).unwrap();
        assert_eq!(r.written[0].backup.clone().unwrap().file_name().unwrap(), ".env.bak-20261014-080000-1");
    }

    #[test]
    fn apply_reports_partial_progress() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".env"), b"1").unwrap();
        std::fs::write(d.path().join("whitelist.txt"), b"changed").unwrap();
        let plan = WritePlan { files: vec![fw(d.path(), FileKind::Env, Some(b"1"), b"2"), fw(d.path(), FileKind::Whitelist, Some(b"orig"), b"new")] };
        let e = apply(&plan, &id, false, at(8, 0, 0)).unwrap_err();
        assert_eq!(e.kind, FileKind::Whitelist);
        assert_eq!(e.written_before, vec![FileKind::Env]);
    }
}
