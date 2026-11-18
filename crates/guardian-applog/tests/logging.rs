#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_applog::rotate;

const MIB: u64 = 1024 * 1024;

#[test]
fn rotates_over_one_mib() {
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("console.log");
    std::fs::write(&log, vec![b'x'; usize::try_from(MIB + 1).unwrap()]).unwrap();
    rotate(&log, MIB).unwrap();
    assert!(!log.exists());
    assert_eq!(
        std::fs::metadata(d.path().join("console.log.1"))
            .unwrap()
            .len(),
        MIB + 1
    );
}

#[test]
fn small_file_is_kept() {
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("console.log");
    std::fs::write(&log, b"short").unwrap();
    rotate(&log, MIB).unwrap();
    assert_eq!(std::fs::read(&log).unwrap(), b"short");
}

#[test]
fn keeps_at_most_two_old_files() {
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("console.log");
    for i in 0..4u8 {
        std::fs::write(&log, vec![b'0' + i; 11]).unwrap();
        rotate(&log, 10).unwrap();
    }
    assert_eq!(
        std::fs::read(d.path().join("console.log.1")).unwrap()[0],
        b'3'
    );
    assert_eq!(
        std::fs::read(d.path().join("console.log.2")).unwrap()[0],
        b'2'
    );
    assert!(!d.path().join("console.log.3").exists());
}

#[test]
fn missing_or_unwritable_dir_does_not_fail() {
    let d = tempfile::tempdir().unwrap();
    rotate(&d.path().join("nope/console.log"), MIB).unwrap();
}
