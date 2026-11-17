#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_agent::config::{AgentConfig, AgentPaths};
use guardian_agent::store::{
    SeenIds, atomic_write, default_state, load_or_create_key, load_state, load_whitelist,
    save_state,
};
use guardian_proto::v1::{AgentState, StoredState};
use prost::Message;

fn paths() -> (tempfile::TempDir, AgentPaths) {
    let d = tempfile::tempdir().unwrap();
    let p = AgentPaths::under(d.path());
    std::fs::create_dir_all(&p.data).unwrap();
    (d, p)
}

#[test]
fn config_roundtrips_and_is_private() {
    let (_d, p) = paths();
    let c = AgentConfig {
        server: "wss://srv:8443".into(),
        cert_sha256: Some("ab".repeat(32)),
        computer_id: Some(7),
    };
    c.save(&p.config).unwrap();
    assert_eq!(AgentConfig::load(&p.config).unwrap(), c);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p.config).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn key_is_created_once_and_kept() {
    let (_d, p) = paths();
    let k1 = load_or_create_key(&p.key()).unwrap();
    let k2 = load_or_create_key(&p.key()).unwrap();
    assert_eq!(k1.public(), k2.public());
    std::fs::write(p.key(), [1u8; 31]).unwrap();
    assert!(load_or_create_key(&p.key()).is_err());
    assert_eq!(
        std::fs::read(p.key()).unwrap(),
        vec![1u8; 31],
        "a bad key is never overwritten"
    );
}

#[test]
fn state_roundtrips() {
    let (_d, p) = paths();
    let st = StoredState {
        state: Some(AgentState {
            version: 4,
            effective_profile_id: 2,
            profiles: vec![],
            room_id: 1,
        }),
        computer_id: 9,
        ..Default::default()
    };
    save_state(&p.state(), &st).unwrap();
    assert_eq!(load_state(&p.state()), Some(st));
}

#[test]
fn corrupt_state_is_ignored() {
    let (_d, p) = paths();
    std::fs::write(p.state(), [0xff, 0x13, 0x77, 0x01, 0x99, 0xaa]).unwrap();
    assert_eq!(load_state(&p.state()), None);
    let good = default_state().encode_to_vec();
    std::fs::write(p.state(), &good[..5.min(good.len() - 1)]).unwrap();
    assert_eq!(load_state(&p.state()), None);
    assert_eq!(load_state(&p.data.join("missing.pb")), None);
}

#[test]
fn atomic_write_leaves_no_temp_files() {
    let (_d, p) = paths();
    for i in 0..50u8 {
        atomic_write(&p.state(), &[i; 100], true).unwrap();
    }
    let names: Vec<_> = std::fs::read_dir(&p.data)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert_eq!(std::fs::read(p.state()).unwrap(), vec![49u8; 100]);
}

#[test]
fn seen_ids_dedupe_and_cap() {
    let (_d, p) = paths();
    let mut s = SeenIds::load(&p.seen());
    assert!(s.insert("a"));
    assert!(!s.insert("a"));
    for i in 0..1000 {
        s.insert(&format!("id{i}"));
    }
    assert!(s.insert("a"), "the oldest id left after 1000 newer ones");
    s.save(&p.seen()).unwrap();
    let mut back = SeenIds::load(&p.seen());
    assert!(!back.insert("id999"));
    assert!(!back.insert("a"));
}

#[test]
fn whitelist_parsing() {
    let (_d, p) = paths();
    std::fs::write(
        p.whitelist(),
        "# x\n\nprinter-tool.exe\nC:\\bad\n  Word.EXE \n",
    )
    .unwrap();
    let w: Vec<String> = load_whitelist(&p.whitelist())
        .iter()
        .map(|a| a.as_str().to_string())
        .collect();
    assert_eq!(w, ["printer-tool.exe", "word.exe"]);
    assert!(load_whitelist(&p.data.join("none.txt")).is_empty());
}

#[test]
fn default_state_is_an_empty_lesson() {
    let st = default_state();
    let s = st.state.unwrap();
    assert_eq!((s.version, s.effective_profile_id), (0, 0));
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.profiles[0].name, "Lecție");
    assert_eq!(
        s.profiles[0].mode,
        guardian_proto::v1::Mode::Blacklist as i32
    );
    assert!(s.profiles[0].apps.is_empty());
}
