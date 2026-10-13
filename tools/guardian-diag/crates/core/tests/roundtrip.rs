//! Spec R29: whatever we write, the agent must read back exactly the edit.

use guardian_core::edit::render::{EnvChange, render_env, render_whitelist};
use guardian_core::parse::{env_pairs, whitelist};

/// The agent's readEnvFile: last occurrence wins.
fn agent_get(bytes: &[u8], key: &str) -> Option<String> {
    env_pairs(&String::from_utf8_lossy(bytes)).into_iter().rfind(|(k, _)| k == key).map(|(_, v)| v)
}

const ENVS: &[&[u8]] = &[
    b"",
    b"SERVER_ADDRESS=https://a\r\nCHECK_INTERVAL=30\r\n",
    b"SERVER_ADDRESS=https://a\nSERVER_ADDRESS = https://b \nX=1",
    b"# c\r\n\r\nBINDING_TOKEN=old\r\nPROXY=p\r\nBINDING_TOKEN=old2\r\n",
    b"\xEF\xBB\xBF# bom first\r\nCHECK_INTERVAL=5\r\n",
    b"A=1\r\nB=2\nC=3",
    b"\xEF\xBB\xBFSERVER_ADDRESS=https://a\r\n",
];

#[test]
fn env_roundtrip() {
    let edits: Vec<Vec<EnvChange>> = vec![
        vec![EnvChange::Set("SERVER_ADDRESS", "https://new.example:8443".into())],
        vec![EnvChange::Set("CHECK_INTERVAL", "15".into()), EnvChange::Remove("BINDING_TOKEN")],
        vec![EnvChange::Set("BINDING_TOKEN", "tok=with=equals".into())],
        vec![EnvChange::Remove("CHECK_INTERVAL"), EnvChange::Set("SERVER_ADDRESS", "http://10.0.0.1".into())],
    ];
    for src in ENVS {
        let before = env_pairs(&String::from_utf8_lossy(src));
        for ch in &edits {
            let out = render_env(src, ch).unwrap();
            for c in ch {
                match c {
                    EnvChange::Set(k, v) => assert_eq!(agent_get(&out, k).as_deref(), Some(v.as_str()), "{src:?} {ch:?}"),
                    EnvChange::Remove(k) => assert_eq!(agent_get(&out, k), None, "{src:?} {ch:?}"),
                }
            }
            // Every key not touched reads the same as before.
            for (k, _) in &before {
                if !ch.iter().any(|c| matches!(c, EnvChange::Set(x, _) | EnvChange::Remove(x) if x == k)) {
                    assert_eq!(agent_get(&out, k), agent_get(src, k), "{src:?} {ch:?} key {k}");
                }
            }
        }
    }
}

#[test]
fn whitelist_roundtrip() {
    let srcs: &[&[u8]] = &[b"", b"# c\r\nA.exe\r\nb.exe\r\nA.EXE\r\n", b"x.exe", b"\xEF\xBB\xBFx.exe\n#y\n"];
    let sets: &[&[&str]] = &[&[], &["a.exe"], &["b.exe", "new.exe"], &["x.exe", "a.exe", "z.exe"]];
    for src in srcs {
        for set in sets {
            let entries: Vec<String> = set.iter().map(|s| s.to_string()).collect();
            let out = render_whitelist(src, &entries).unwrap();
            let mut got = whitelist(&out);
            got.dedup();
            let mut got_sorted = got.clone();
            got_sorted.sort();
            let mut want = entries.clone();
            want.sort();
            assert_eq!(got_sorted, want, "{src:?} {set:?}");
        }
    }
}
