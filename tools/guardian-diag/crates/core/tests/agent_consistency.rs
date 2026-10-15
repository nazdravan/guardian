//! The diagnostics app mirrors facts from the agent's Go source. If the agent changes,
//! these tests fail in CI instead of the app silently lying on site (spec R29).

use std::path::PathBuf;

use guardian_core::agent;

fn agent_src(file: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../agent").join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The quoted strings inside `systemProcs := []string{ ... }` in isSystemProcess,
/// skipping commented-out lines.
fn go_protected_list() -> Vec<String> {
    let src = agent_src("main.go");
    let start = src.find("systemProcs := []string{").expect("systemProcs literal");
    let body = &src[start..];
    let end = body.find("\n\t}").expect("end of literal");
    let mut out = Vec::new();
    for line in body[..end].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        let mut rest = line;
        while let Some(a) = rest.find('"') {
            let after = &rest[a + 1..];
            let b = after.find('"').expect("closing quote");
            out.push(after[..b].to_string());
            rest = &after[b + 1..];
        }
    }
    out
}

#[test]
fn protected_list_matches_agent() {
    let mut go = go_protected_list();
    let mut ours: Vec<String> = agent::PROTECTED.iter().map(|s| s.to_string()).collect();
    go.sort();
    ours.sort();
    assert_eq!(ours, go);
    assert!(agent::is_protected("Guardian.exe"));
}

#[test]
fn file_names_and_keys_match_agent() {
    let all = format!("{}{}{}", agent_src("main.go"), agent_src("agent.go"), agent_src("client.go"));
    for needle in [
        format!("envFileName = \"{}\"", agent::ENV_FILE),
        format!("credentialsFileName = \"{}\"", agent::CREDENTIALS_FILE),
        format!("whitelistFile = \"{}\"", agent::WHITELIST_FILE),
        format!("os.WriteFile(\"{}\"", agent::SYNC_FILE),
        format!("get(\"{}\")", agent::KEY_SERVER_ADDRESS),
        format!("get(\"{}\")", agent::KEY_BINDING_TOKEN),
        format!("get(\"{}\")", agent::KEY_CHECK_INTERVAL),
        format!("defaultServiceInterval = {} * time.Second", agent::DEFAULT_SERVICE_INTERVAL_SECS),
    ] {
        assert!(all.contains(&needle), "agent source no longer contains `{needle}`");
    }
}

#[test]
fn log_strings_match_agent() {
    let src = agent_src("agent.go");
    assert!(src.contains(&format!("\"{}%s\"", agent::KILL_BLACKLIST_PREFIX)));
    assert!(src.contains(&format!("\"{}%s\"", agent::KILL_WHITELIST_PREFIX)));
    assert!(src.contains(&format!("\"{}%s\"", agent::START_MARKER)));
    assert!(src.contains(&format!("Infof(\"{}%s as %s (machine %s)\"", agent::ENROLLED_MARKER)));
    // The "Enrolled with" record comes right after removeEnvKey rewrote .env.
    let rewrite = src.find("removeEnvKey(envFileName, \"BINDING_TOKEN\")").expect("removeEnvKey call");
    let logged = src.find(&format!("\"{}%s", agent::ENROLLED_MARKER)).expect("Enrolled with record");
    assert!(rewrite < logged);
}

/// Every non-test Go source file in `agent/` and `server/`, concatenated.
fn go_sources() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let mut all = String::new();
    for dir in ["agent", "server"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".go") && !name.ends_with("_test.go") {
                all.push_str(&std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    all
}

#[test]
fn refusal_needles_exist_in_go_source() {
    let all = go_sources();
    for (needle, _) in guardian_core::diagnose::REFUSALS {
        assert!(all.contains(needle), "no agent/server source contains the refusal text `{needle}`");
    }
}
