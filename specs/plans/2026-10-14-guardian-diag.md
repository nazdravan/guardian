# Guardian Diag Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A portable `Guardian.exe` (Rust + egui) that reads the ProcSentinel agent installed on this Windows machine, diagnoses it, shows its config, policy, processes and Event Log, and — when run as administrator — edits `.env` and `whitelist.txt` and restarts the service.

**Architecture:** A cargo workspace in `tools/guardian-diag/` after PdfCraft (`~/Workspace/bucket/pdfcraft`): `core` (pure: parsing, `Snapshot`, diagnosis, enforce simulation, report, edit model and file rendering — runs and tests on any OS), `winhost` (collects a `Snapshot` on Windows; applies a `WritePlan` portably; restarts the service), `ui-egui` (four tabs over a `Snapshot`, edit draft, background work), `apps/guardian` (the `Guardian.exe` binary: window or `report` CLI).

**Tech Stack:** Rust 2024 (rust-version 1.90), egui/eframe 0.36 (glow only), roxmltree, serde/serde_json, sha2, chrono, ureq 3 + rustls-native-certs, windows-service, winreg, is_elevated, rfd, winresource, egui_kittest.

**Spec:** `specs/2026-10-10-guardian-diag-design.md` (revision 2026-10-13, decisions 2026-10-14). Requirement numbers below (R1…R33) refer to its numbered requirements.

## Global Constraints

- Target `i686-pc-windows-msvc`, static CRT (`-C target-feature=+crt-static`), one file, no DLLs next to it; runs on 32- and 64-bit Windows 10+ (R1).
- Binary name exactly `Guardian.exe` (the agent's protected list contains `guardian.exe`).
- Manifest `asInvoker` (R3). Never `requireAdministrator`.
- Writes to the machine only: agent `.env`, agent `whitelist.txt`, their `*.bak-YYYYMMDD-HHMMSS` copies, and `GuardianReports\` next to the exe (R2). `report` CLI writes nothing (R25).
- Secrets (`BINDING_TOKEN`, `agent_token`) never shown, copied, logged or reported; only `present/absent` + last 4 hex chars of SHA-256 (R4).
- UI text in English. Process names, paths, GUIDs, times in JetBrains Mono (R9, R31).
- Workspace lints: `unsafe_code = "forbid"`; every crate root has `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`; `clippy.toml` allows them in tests (R28).
- Read caps: 1 MB per file, 2000 Event Log records (R15/R20, R28).
- Rendering: eframe with `glow`, no wgpu (R30). Dark theme, colour only for state; amber for pending/unsaved (R31). Fonts via `include_bytes!` from `mobile/assets/fonts/` — no copies in the repo.
- `whitelist.txt` is never deleted (agent regenerates it from running processes if absent).
- `agent_credentials.json` and `sync.json` are never written.
- The installer archive (`server/installer.go`) is not touched: it keeps the Go console as `Guardian.exe` (Decisions, 2026-10-14).
- Commit messages: `feat(diag): …` / `test(diag): …` / `docs(diag): …` like the existing history; no Claude attribution or session trailers.

## Review Focus

1. **A `.env` hand-edited on site with odd shape** — duplicate keys, `KEY = value` with spaces, CRLF mixed with LF, UTF-8 BOM, no trailing newline. Expect: the agent's reading of the rendered file equals the edit and every untouched byte survives. Pinned in Task 6 (`render_env_*` golden tests + round-trip).
2. **The file changed between load and Save** — the agent deletes `BINDING_TOKEN` right after enrolling, often seconds after the admin opened the window. Expect: `Changed` error, nothing written, draft kept. Pinned in Task 7 (`apply_refuses_when_file_changed`).
3. **A 32-bit `Guardian.exe` on 64-bit Windows** reading/writing `System32\ProcSentinel\agent`. Expect: I/O through `Sysnative`, UI shows `System32`. Pinned in Task 2 (`io_path_*`) and manual check in Task 13.
4. **Whitelist pasted from Explorer** — `"C:\Program Files\Microsoft Office\WINWORD.EXE"` with quotes, or `notepad` without `.exe`. Expect: `winword.exe` added; `notepad` added with a warning. Pinned in Task 5 (`normalize_*`, `check_new_entry_*`).
5. **Machine without the agent, or a non-admin run** — no service, no access to the folder. Expect: a verdict ("Not installed" / "could not check"), read-only Configuration with "Run as administrator to edit", no crash. Pinned in Task 3 (`diagnose_not_installed`, `diagnose_denied_is_unknown`) and Task 11 (`edit_locked_when_not_elevated`).

---

## File Structure

```
tools/guardian-diag/
├── Cargo.toml                    # workspace, shared deps, lints, profiles
├── clippy.toml rustfmt.toml      # copied from PdfCraft
├── .cargo/config.toml            # +crt-static for i686-pc-windows-msvc
├── crates/core/
│   ├── Cargo.toml
│   ├── src/lib.rs                # module list + crate lints
│   ├── src/agent.rs              # constants mirrored from the agent (file names, keys, protected list, log strings)
│   ├── src/paths.rs              # unquote_image_path, io_path (Sysnative)
│   ├── src/read.rs               # Read<T>, SourceFile<T>, SecretTail, hashing
│   ├── src/parse.rs              # .env, credentials, sync.json, whitelist, tasklist CSV, wevtutil XML
│   ├── src/snapshot.rs           # Snapshot and its member types
│   ├── src/diagnose.rs           # checks + verdict
│   ├── src/enforce.rs            # verdict_for (simulation of agent's enforce)
│   ├── src/report.rs             # text + JSON report, exit code
│   ├── src/edit/mod.rs           # EnvEdit, WhitelistEdit, TokenEdit, Issue, validate, normalize_name, check_new_entry
│   ├── src/edit/render.rs        # render_env, render_whitelist
│   ├── src/edit/plan.rs          # WritePlan, FileWrite, plan()
│   └── tests/
│       ├── fixtures/…            # agent folders + tasklist/wevtutil dumps
│       ├── agent_consistency.rs  # reads ../../../../agent/*.go
│       └── roundtrip.rs          # render → agent-style parse == edit
├── crates/winhost/
│   ├── Cargo.toml
│   ├── src/lib.rs
│   ├── src/apply.rs              # portable: backup, rotate, temp+rename, verify
│   ├── src/files.rs              # capped file reading → Read<…>
│   ├── src/collect.rs            # build a Snapshot (Windows parts behind cfg)
│   ├── src/win.rs                # cfg(windows): SCM, registry, commands, elevation, runas, restart
│   └── src/health.rs             # GET /health
├── crates/ui-egui/
│   ├── Cargo.toml
│   ├── src/lib.rs                # GuardianApp (eframe::App)
│   ├── src/theme.rs              # fonts, palette
│   ├── src/state.rs              # pure UI state: tabs, draft, dirty count, edit lock
│   ├── src/worker.rs             # background collect/apply/restart with mpsc
│   ├── src/tabs/diagnostics.rs
│   ├── src/tabs/configuration.rs
│   ├── src/tabs/processes.rs
│   └── src/tabs/log.rs
└── apps/guardian/
    ├── Cargo.toml                # [[bin]] name = "Guardian"
    ├── build.rs                  # icon + manifest via winresource
    ├── guardian.manifest         # asInvoker, Common Controls 6, PerMonitorV2
    ├── src/main.rs               # arg parsing: window | report
    └── tests/report_cli.rs       # runs the binary against a fixture
.github/workflows/diag.yml
specs/guardian-diag.md            # final description (Task 13)
```

---

### Task 1: Workspace scaffold and agent constants

**Files:**
- Create: `tools/guardian-diag/Cargo.toml`, `tools/guardian-diag/clippy.toml`, `tools/guardian-diag/rustfmt.toml`, `tools/guardian-diag/.cargo/config.toml`, `tools/guardian-diag/.gitignore`
- Create: `tools/guardian-diag/crates/core/Cargo.toml`, `src/lib.rs`, `src/agent.rs`
- Test: `tools/guardian-diag/crates/core/tests/agent_consistency.rs`
- Create: `.github/workflows/diag.yml` (Linux job only)

**Interfaces:**
- Produces: `guardian_core::agent::{SERVICE_NAME, DEFAULT_INSTALL_DIR, ENV_FILE, CREDENTIALS_FILE, SYNC_FILE, WHITELIST_FILE, KEY_SERVER_ADDRESS, KEY_BINDING_TOKEN, KEY_CHECK_INTERVAL, DEFAULT_SERVICE_INTERVAL_SECS, PROTECTED, KILL_BLACKLIST_PREFIX, KILL_WHITELIST_PREFIX, START_MARKER, is_protected(&str) -> bool}`

- [ ] **Step 1: Workspace files**

`tools/guardian-diag/Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/*", "apps/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.90"
license = "MIT"

[workspace.dependencies]
guardian-core = { path = "crates/core" }
guardian-winhost = { path = "crates/winhost" }
guardian-ui = { path = "crates/ui-egui" }

serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
sha2 = "0.10"
chrono = { version = "0.4", default-features = false, features = ["std", "clock", "serde"] }
roxmltree = "0.20"
ureq = { version = "3.4", default-features = false, features = ["rustls-no-provider", "_ring"] }
rustls-native-certs = "0.8"
windows-service = "0.8"
winreg = "0.55"
is_elevated = "0.1"
egui = "0.36"
eframe = { version = "0.36", default-features = false, features = ["glow", "default_fonts", "accesskit"] }
egui_extras = { version = "0.36", default-features = false }
egui_kittest = "0.36"
rfd = "0.17"
tempfile = "3"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
dbg_macro = "warn"
todo = "warn"

[profile.release]
lto = "thin"
codegen-units = 1
opt-level = "s"
```

`clippy.toml`:
```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-indexing-slicing-in-tests = true
```

`rustfmt.toml`:
```toml
max_width = 150
use_small_heuristics = "Max"
```

`.cargo/config.toml`:
```toml
[target.i686-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

`.gitignore`:
```
/target
```

`crates/core/Cargo.toml`:
```toml
[package]
name = "guardian-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
sha2 = { workspace = true }
chrono = { workspace = true }
roxmltree = { workspace = true }
```

- [ ] **Step 2: Write the failing consistency test**

`crates/core/tests/agent_consistency.rs`:
```rust
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
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd tools/guardian-diag && cargo test -p guardian-core --test agent_consistency`
Expected: compile error, `agent` module not found.

- [ ] **Step 4: Implement `lib.rs` and `agent.rs`**

`crates/core/src/lib.rs`:
```rust
//! Guardian Diag core: everything that can be decided without an operating system.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod agent;
```

`crates/core/src/agent.rs`:
```rust
//! Facts mirrored from the agent's Go source (`agent/*.go`). `tests/agent_consistency.rs`
//! re-reads the source and fails if any of these drift.

pub const SERVICE_NAME: &str = "ProcSentinelAgent";
pub const DEFAULT_INSTALL_DIR: &str = r"C:\Windows\System32\ProcSentinel\agent";

pub const ENV_FILE: &str = ".env";
pub const CREDENTIALS_FILE: &str = "agent_credentials.json";
pub const SYNC_FILE: &str = "sync.json";
pub const WHITELIST_FILE: &str = "whitelist.txt";

pub const KEY_SERVER_ADDRESS: &str = "SERVER_ADDRESS";
pub const KEY_BINDING_TOKEN: &str = "BINDING_TOKEN";
pub const KEY_CHECK_INTERVAL: &str = "CHECK_INTERVAL";
pub const KNOWN_KEYS: [&str; 3] = [KEY_SERVER_ADDRESS, KEY_BINDING_TOKEN, KEY_CHECK_INTERVAL];

/// What the agent uses in service mode when CHECK_INTERVAL is absent or invalid.
pub const DEFAULT_SERVICE_INTERVAL_SECS: u32 = 20;

/// Event Log message prefixes (`Infof("Killed process: %s")` etc.).
pub const KILL_BLACKLIST_PREFIX: &str = "Killed process: ";
pub const KILL_WHITELIST_PREFIX: &str = "Killed non-whitelisted process: ";
/// Logged once per `runAgent`, i.e. once per service start; used as the start time.
pub const START_MARKER: &str = "Server address: ";

/// `isSystemProcess`'s hardcoded list, lower case, in the agent's order.
pub const PROTECTED: &[&str] = &[
    "system", "system idle process", "secure system", "registry",
    "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe",
    "services.exe", "lsass.exe", "lsaiso.exe",
    "svchost.exe", "dwm.exe", "fontdrvhost.exe", "conhost.exe",
    "wudfhost.exe", "wmiprvse.exe", "dllhost.exe", "spoolsv.exe",
    "audiodg.exe", "dashost.exe", "searchindexer.exe",
    "searchprotocolhost.exe", "searchfilterhost.exe",
    "explorer.exe", "sihost.exe", "taskhostw.exe",
    "runtimebroker.exe", "applicationframehost.exe",
    "shellexperiencehost.exe", "startmenuexperiencehost.exe",
    "searchhost.exe", "searchui.exe", "searchapp.exe",
    "textinputhost.exe", "lockapp.exe", "widgets.exe",
    "widgetservice.exe", "comppkgsrv.exe",
    "userinit.exe", "logonui.exe",
    "securityhealthservice.exe", "securityhealthsystray.exe",
    "securityhealthhost.exe", "smartscreen.exe",
    "msmpeng.exe", "nissrv.exe", "mpcmdrun.exe",
    "ctfmon.exe", "tabletinputservice.exe",
    "lsm.exe", "networkservice.exe", "localservice.exe",
    "tasklist.exe", "taskmgr.exe", "cmd.exe", "powershell.exe", "pwsh.exe",
    "windowsterminal.exe", "wt.exe", "openssh.exe", "sshd.exe",
    "procsentinel-agent64.exe", "procsentinel-agent32.exe",
    "guardian.exe",
];

pub fn is_protected(name: &str) -> bool {
    let lower = name.to_lowercase();
    PROTECTED.iter().any(|p| *p == lower)
}
```
Add `#[rustfmt::skip]` above `PROTECTED` so the grouping survives `cargo fmt`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p guardian-core --test agent_consistency`
Expected: 3 passed. If `file_names_and_keys_match_agent` fails on a needle, read the agent source and fix the needle's *format* (never the constant's value without checking the agent).

- [ ] **Step 6: Linux CI job**

`.github/workflows/diag.yml`:
```yaml
# Guardian Diag mirrors the agent's behaviour, so agent changes re-run it too.
name: diag

on:
  push:
    paths: ["tools/guardian-diag/**", "agent/**", ".github/workflows/diag.yml"]
  pull_request:
    paths: ["tools/guardian-diag/**", "agent/**", ".github/workflows/diag.yml"]

jobs:
  core:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: tools/guardian-diag
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: tools/guardian-diag
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```
(`ui-egui` and `apps/guardian` don't exist yet; when Task 10 adds eframe, add `sudo apt-get install -y libxkbcommon-dev libgl1-mesa-dev libx11-dev` before clippy.)

- [ ] **Step 7: fmt, clippy, commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
```bash
git add tools/guardian-diag .github/workflows/diag.yml
git commit -m "feat(diag): scaffold the workspace and mirror the agent's constants"
```

---

### Task 2: Paths, `Read<T>`, and file parsers

**Files:**
- Create: `crates/core/src/paths.rs`, `crates/core/src/read.rs`, `crates/core/src/parse.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod paths; pub mod read; pub mod parse;`)
- Create fixtures: `crates/core/tests/fixtures/healthy/{.env,agent_credentials.json,sync.json,whitelist.txt}`, `crates/core/tests/fixtures/tasklist.csv`, `crates/core/tests/fixtures/wevtutil.xml`
- Tests: unit tests inside each module (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `paths::unquote_image_path(&str) -> String`; `paths::io_path(canonical: &Path, wow64: bool, windir: &str) -> PathBuf`
  - `read::Read<T> { Ok(T), Missing, Denied, Failed(String) }` with `as_ok() -> Option<&T>`, `map`
  - `read::SecretTail(String)` (4 hex chars) with `SecretTail::of(&str)`; `read::sha256_hex(&[u8]) -> String`
  - `read::SourceFile<T> { bytes: Vec<u8>, hash: String, mtime: Option<DateTime<Utc>>, parsed: T }` (manual `Debug` that never prints `bytes`)
  - `read::Encoding { Utf8, Utf8Bom, Utf16, Invalid }`, `read::LineEnding { Crlf, Lf }`
  - `parse::EnvFile`, `parse::env(&[u8]) -> EnvFile`
  - `parse::Credentials`, `parse::credentials(&[u8]) -> Result<Credentials, String>`
  - `parse::SyncState`, `parse::Mode`, `parse::App`, `parse::sync(&[u8]) -> Result<SyncState, String>`
  - `parse::whitelist(&[u8]) -> Vec<String>`
  - `parse::tasklist(&str) -> Vec<String>`
  - `parse::AgentEvent`, `parse::Level`, `parse::EventKind`, `parse::KillReason`, `parse::wevtutil(&str) -> Result<Vec<AgentEvent>, String>`

- [ ] **Step 1: Fixtures**

`tests/fixtures/healthy/.env` (CRLF line endings — write with `printf`):
```bash
cd tools/guardian-diag/crates/core/tests/fixtures && mkdir -p healthy
printf 'SERVER_ADDRESS=https://guardian.example.com\r\nCHECK_INTERVAL=30\r\n' > healthy/.env
```
`healthy/agent_credentials.json`:
```json
{"agent_token":"agt_secret_value_123","machine_guid":"4c4c4544-0042-3510-8052-b4c04f4d4e31","server_address":"https://guardian.example.com","enrolled_at":"2026-10-01T09:00:00Z"}
```
`healthy/sync.json`:
```json
{"applications":[{"name":"winword.exe","mode":"whitelist"},{"name":"chrome.exe","mode":"whitelist"}],"mode":"whitelist"}
```
`healthy/whitelist.txt` (CRLF):
```bash
printf '# local\r\nnotepad.exe\r\nCalc.exe\r\n' > healthy/whitelist.txt
```
`tasklist.csv`:
```
"System Idle Process","0","Services","0","8 K"
"explorer.exe","4321","Console","1","120,000 K"
"WINWORD.EXE","5555","Console","1","300,000 K"
"game.exe","6666","Console","1","50,000 K"
"Guardian.exe","7777","Console","1","40,000 K"
```
`wevtutil.xml` (what `wevtutil qe … /f:xml` prints: concatenated `<Event>` elements, no root):
```xml
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='ProcSentinelAgent'/><EventID Qualifiers='0'>1</EventID><Level>4</Level><TimeCreated SystemTime='2026-10-10T11:14:05.1234567Z'/><Channel>Application</Channel><Computer>LAB-A-05</Computer></System><EventData><Data>Killed non-whitelisted process: game.exe</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='ProcSentinelAgent'/><EventID Qualifiers='0'>1</EventID><Level>3</Level><TimeCreated SystemTime='2026-10-10T11:13:00.0000000Z'/><Channel>Application</Channel><Computer>LAB-A-05</Computer></System><EventData><Data>Sync failed: dial tcp: lookup guardian.example.com: no such host. Keeping the last known policy.</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='ProcSentinelAgent'/><EventID Qualifiers='0'>1</EventID><Level>4</Level><TimeCreated SystemTime='2026-10-10T11:12:00.0000000Z'/><Channel>Application</Channel><Computer>LAB-A-05</Computer></System><EventData><Data>Synced: 2 applications, mode=whitelist</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='ProcSentinelAgent'/><EventID Qualifiers='0'>1</EventID><Level>4</Level><TimeCreated SystemTime='2026-10-10T11:11:00.0000000Z'/><Channel>Application</Channel><Computer>LAB-A-05</Computer></System><EventData><Data>Killed process: steam.exe</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='ProcSentinelAgent'/><EventID Qualifiers='0'>1</EventID><Level>4</Level><TimeCreated SystemTime='2026-10-10T11:10:00.0000000Z'/><Channel>Application</Channel><Computer>LAB-A-05</Computer></System><EventData><Data>Server address: https://guardian.example.com</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='ProcSentinelAgent'/><EventID Qualifiers='0'>1</EventID><Level>2</Level><TimeCreated SystemTime='2026-10-10T11:09:00.0000000Z'/><Channel>Application</Channel><Computer>LAB-A-05</Computer></System><EventData><Data>The server no longer accepts this computer's token (401). Reinstall.</Data></EventData></Event>
```
Before relying on this shape, confirm it on a real Windows box once (`wevtutil qe Application /q:"*[System[Provider[@Name='ProcSentinelAgent']]]" /c:3 /rd:true /f:xml`) and, if it differs, replace the fixture with the real output.

- [ ] **Step 2: `paths.rs` with failing tests**

```rust
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
    if s.len() >= sys32.len() && s[..sys32.len()].eq_ignore_ascii_case(&sys32) {
        return PathBuf::from(format!("{}\\Sysnative{}", windir.trim_end_matches('\\'), &s[sys32.len()..]));
    }
    canonical.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquote_image_path_cases() {
        assert_eq!(unquote_image_path(r#""C:\Windows\System32\ProcSentinel\agent\procsentinel-agent64.exe""#), r"C:\Windows\System32\ProcSentinel\agent\procsentinel-agent64.exe");
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
```
Run: `cargo test -p guardian-core paths` — Expected: PASS after adding `pub mod paths;` (write the tests first, run once against an empty `paths.rs` with only the tests to see the compile failure, then add the functions).

- [ ] **Step 3: `read.rs`**

```rust
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
        f.debug_struct("SourceFile").field("len", &self.bytes.len()).field("hash", &self.hash).field("mtime", &self.mtime).field("parsed", &self.parsed).finish()
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
```
Add `serde = { workspace = true }` is already a dependency. Run: `cargo test -p guardian-core read` — Expected: PASS.

- [ ] **Step 4: `parse.rs` — write the tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    fn fx(p: &str) -> Vec<u8> {
        std::fs::read(format!("{FX}/{p}")).unwrap()
    }

    #[test]
    fn env_reads_like_the_agent() {
        let e = env(b"# c\r\nSERVER_ADDRESS = https://a.example/ \r\nSERVER_ADDRESS=https://b.example//\r\nCHECK_INTERVAL=abc\r\nBINDING_TOKEN=tok\r\nTOKEN=old\r\nnoequals\r\n");
        assert_eq!(e.server_address.as_deref(), Some("https://b.example")); // last wins, trailing / trimmed
        assert_eq!(e.check_interval_raw.as_deref(), Some("abc"));
        assert_eq!(e.effective_check_interval(), 20);
        assert_eq!(e.binding_token, Some(SecretTail::of("tok")));
        assert_eq!(e.duplicates, vec![("SERVER_ADDRESS".to_string(), 2)]);
        assert_eq!(e.unknown, vec![("TOKEN".to_string(), "••••".to_string())]);
        assert_eq!(e.encoding, Encoding::Utf8);
        assert_eq!(e.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn env_unknown_values_shown_unless_secret_like() {
        let e = env(b"PROXY=http://p:3128\nAPI_KEY=x\nPASSWORD=y\n");
        assert_eq!(e.unknown, vec![("PROXY".into(), "http://p:3128".into()), ("API_KEY".into(), "••••".into()), ("PASSWORD".into(), "••••".into())]);
    }

    #[test]
    fn env_bom_is_not_part_of_the_first_key() {
        // The agent would see "\u{feff}SERVER_ADDRESS"; we report the BOM as a problem
        // but parse the key as the agent does, so diagnosis matches reality.
        let e = env(b"\xEF\xBB\xBFSERVER_ADDRESS=https://a\n");
        assert_eq!(e.encoding, Encoding::Utf8Bom);
        assert_eq!(e.server_address, None);
        assert_eq!(e.unknown.len(), 1);
    }

    #[test]
    fn credentials_drop_the_token() {
        let c = credentials(&fx("healthy/agent_credentials.json")).unwrap();
        assert_eq!(c.server_address, "https://guardian.example.com");
        assert_eq!(c.machine_guid, "4c4c4544-0042-3510-8052-b4c04f4d4e31");
        assert_eq!(c.token, Some(SecretTail::of("agt_secret_value_123")));
        assert!(!format!("{c:?}").contains("agt_secret"));
        assert!(credentials(b"{not json").is_err());
    }

    #[test]
    fn sync_state() {
        let s = sync(&fx("healthy/sync.json")).unwrap();
        assert_eq!(s.mode, Mode::Whitelist);
        assert_eq!(s.applications.len(), 2);
        assert_eq!(sync(br#"{"applications":null,"mode":"free"}"#).unwrap().mode, Mode::Free);
        assert_eq!(sync(br#"{"mode":"weird"}"#).unwrap().mode, Mode::Other("weird".into()));
        assert!(sync(b"{").is_err());
    }

    #[test]
    fn whitelist_like_the_agent() {
        assert_eq!(whitelist(&fx("healthy/whitelist.txt")), vec!["notepad.exe", "calc.exe"]);
        assert_eq!(whitelist(b"  A.exe  \n#x\n\n"), vec!["a.exe"]);
    }

    #[test]
    fn tasklist_names() {
        let t = tasklist(std::str::from_utf8(&fx("tasklist.csv")).unwrap());
        assert_eq!(t, vec!["System Idle Process", "explorer.exe", "WINWORD.EXE", "game.exe", "Guardian.exe"]);
    }

    #[test]
    fn wevtutil_events() {
        let ev = wevtutil(std::str::from_utf8(&fx("wevtutil.xml")).unwrap()).unwrap();
        assert_eq!(ev.len(), 6);
        assert_eq!(ev[0].kind, EventKind::Kill { process: "game.exe".into(), reason: KillReason::Whitelist });
        assert_eq!(ev[1].level, Level::Warning);
        assert_eq!(ev[1].kind, EventKind::Other);
        assert_eq!(ev[2].kind, EventKind::Sync);
        assert_eq!(ev[3].kind, EventKind::Kill { process: "steam.exe".into(), reason: KillReason::Blacklist });
        assert_eq!(ev[4].kind, EventKind::Start);
        assert_eq!(ev[5].level, Level::Error);
        assert_eq!(ev[0].time.to_rfc3339(), "2026-10-10T11:14:05.123456700+00:00");
        assert_eq!(wevtutil("").unwrap(), vec![]);
        assert!(wevtutil("<Event><broken").is_err());
    }
}
```
Run: `cargo test -p guardian-core parse` — Expected: compile errors (types missing).

- [ ] **Step 5: Implement `parse.rs`**

```rust
//! Parsers for every source the agent leaves behind. Each mirrors the agent's own reading
//! (see agent/client.go readEnvFile, agent/main.go loadWhitelist, extractProcessName).

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::agent;
use crate::read::{Encoding, LineEnding, SecretTail, detect_encoding, detect_line_ending};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvFile {
    /// As the agent sees it: last occurrence, trimmed, trailing '/' removed.
    pub server_address: Option<String>,
    /// Raw last value; the agent falls back to its default when it is not an int > 0.
    pub check_interval_raw: Option<String>,
    pub binding_token: Option<SecretTail>,
    /// Keys defined more than once, with the count.
    pub duplicates: Vec<(String, usize)>,
    /// Keys the agent ignores, with values masked when they look like secrets.
    pub unknown: Vec<(String, String)>,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

impl EnvFile {
    pub fn effective_check_interval(&self) -> u32 {
        self.check_interval_raw.as_deref().and_then(|v| v.parse::<i64>().ok()).filter(|v| *v > 0).map(|v| v.min(u32::MAX as i64) as u32).unwrap_or(agent::DEFAULT_SERVICE_INTERVAL_SECS)
    }
}

/// `KEY=value` lines exactly as the agent's readEnvFile splits them. Returns (key, value) pairs in file order.
pub fn env_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn looks_secret(key: &str) -> bool {
    let k = key.to_uppercase();
    ["TOKEN", "SECRET", "PASSWORD", "KEY"].iter().any(|s| k.contains(s))
}

pub fn env(bytes: &[u8]) -> EnvFile {
    let encoding = detect_encoding(bytes);
    let text = String::from_utf8_lossy(bytes);
    let pairs = env_pairs(&text);
    let mut e = EnvFile { encoding, line_ending: detect_line_ending(&text), ..Default::default() };
    let mut order: Vec<String> = Vec::new();
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    let mut last: std::collections::HashMap<String, String> = Default::default();
    for (k, v) in pairs {
        if !counts.contains_key(&k) {
            order.push(k.clone());
        }
        *counts.entry(k.clone()).or_default() += 1;
        last.insert(k, v);
    }
    for k in &order {
        let v = last.get(k).cloned().unwrap_or_default();
        let n = counts.get(k).copied().unwrap_or(0);
        if n > 1 {
            e.duplicates.push((k.clone(), n));
        }
        match k.as_str() {
            agent::KEY_SERVER_ADDRESS => e.server_address = Some(v.trim_end_matches('/').to_string()).filter(|s| !s.is_empty()),
            agent::KEY_CHECK_INTERVAL => e.check_interval_raw = Some(v),
            agent::KEY_BINDING_TOKEN => e.binding_token = Some(SecretTail::of(&v)).filter(|_| !v.is_empty()),
            _ => e.unknown.push((k.clone(), if looks_secret(k) { "••••".into() } else { v })),
        }
    }
    e
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub machine_guid: String,
    pub server_address: String,
    pub enrolled_at: String,
    pub token: Option<SecretTail>,
}

pub fn credentials(bytes: &[u8]) -> Result<Credentials, String> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        agent_token: String,
        #[serde(default)]
        machine_guid: String,
        #[serde(default)]
        server_address: String,
        #[serde(default)]
        enrolled_at: String,
    }
    let r: Raw = serde_json::from_slice(bytes).map_err(|e| format!("not valid JSON: {e}"))?;
    Ok(Credentials {
        machine_guid: r.machine_guid,
        server_address: r.server_address,
        enrolled_at: r.enrolled_at,
        token: Some(SecretTail::of(&r.agent_token)).filter(|_| !r.agent_token.is_empty()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Blacklist,
    Whitelist,
    Free,
    Other(String),
}

impl Mode {
    pub fn as_str(&self) -> &str {
        match self {
            Mode::Blacklist => "blacklist",
            Mode::Whitelist => "whitelist",
            Mode::Free => "free",
            Mode::Other(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct App {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub mode: Mode,
    pub applications: Vec<App>,
}

pub fn sync(bytes: &[u8]) -> Result<SyncState, String> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        applications: Option<Vec<App>>,
        #[serde(default)]
        mode: String,
    }
    let r: Raw = serde_json::from_slice(bytes).map_err(|e| format!("not valid JSON: {e}"))?;
    let mode = match r.mode.as_str() {
        "blacklist" => Mode::Blacklist,
        "whitelist" => Mode::Whitelist,
        "free" => Mode::Free,
        other => Mode::Other(other.to_string()),
    };
    Ok(SyncState { mode, applications: r.applications.unwrap_or_default() })
}

/// Entries as the agent's loadWhitelist sees them: trimmed, lower case, comments skipped.
pub fn whitelist(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes).split('\n').map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_lowercase).collect()
}

/// First quoted CSV field of each `tasklist /FO CSV /NH` line (agent's extractProcessName).
pub fn tasklist(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix('"')?;
            let end = rest.find('"')?;
            Some(rest[..end].to_string()).filter(|s| !s.is_empty())
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum KillReason {
    Blacklist,
    Whitelist,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum EventKind {
    Kill { process: String, reason: KillReason },
    Sync,
    Start,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentEvent {
    pub time: DateTime<Utc>,
    pub level: Level,
    pub message: String,
    pub kind: EventKind,
}

pub fn classify(message: &str) -> EventKind {
    if let Some(p) = message.strip_prefix(agent::KILL_WHITELIST_PREFIX) {
        EventKind::Kill { process: p.trim().to_string(), reason: KillReason::Whitelist }
    } else if let Some(p) = message.strip_prefix(agent::KILL_BLACKLIST_PREFIX) {
        EventKind::Kill { process: p.trim().to_string(), reason: KillReason::Blacklist }
    } else if message.starts_with("Synced:") {
        EventKind::Sync
    } else if message.starts_with(agent::START_MARKER) {
        EventKind::Start
    } else {
        EventKind::Other
    }
}

/// `wevtutil qe … /f:xml` output: concatenated <Event> elements without a root.
pub fn wevtutil(xml: &str) -> Result<Vec<AgentEvent>, String> {
    if xml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let wrapped = format!("<Events>{xml}</Events>");
    let doc = roxmltree::Document::parse(&wrapped).map_err(|e| format!("unreadable event log output: {e}"))?;
    let mut out = Vec::new();
    for ev in doc.root_element().children().filter(|n| n.has_tag_name("Event")) {
        let find = |name: &str| ev.descendants().find(|n| n.has_tag_name(name));
        let time = find("TimeCreated").and_then(|n| n.attribute("SystemTime")).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
        let Some(time) = time else { continue };
        let level = match find("Level").and_then(|n| n.text()).unwrap_or("4") {
            "1" | "2" => Level::Error,
            "3" => Level::Warning,
            _ => Level::Info,
        };
        let message: String = ev.descendants().filter(|n| n.has_tag_name("Data")).filter_map(|n| n.text()).collect::<Vec<_>>().join(" ");
        let message = message.trim().to_string();
        let kind = classify(&message);
        out.push(AgentEvent { time, level, message, kind });
    }
    Ok(out)
}
```
- [ ] **Step 6: Run tests, clippy, commit**

Run: `cargo test -p guardian-core && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): parse every file and log the agent leaves behind"
```

---

### Task 3: `Snapshot` and diagnosis

**Files:**
- Create: `crates/core/src/snapshot.rs`, `crates/core/src/diagnose.rs`
- Create: `crates/core/src/testkit.rs` (`#[cfg(any(test, feature = "testkit"))]`): builds a `Snapshot` from a fixture folder
- Modify: `crates/core/src/lib.rs`, `crates/core/Cargo.toml` (`[features] testkit = []`)

**Interfaces:**
- Consumes: Task 2 types.
- Produces:
  - `snapshot::{Snapshot, MachineInfo, InstallDir, DirSource, ServiceInfo, ServiceState, StartType, HealthProbe}`
  - `diagnose::{diagnose(&Snapshot) -> Diagnosis, Diagnosis, Verdict, VerdictKind, Check, CheckId, Status}`
  - `testkit::snapshot_from_dir(dir: &Path, now: DateTime<Utc>) -> Snapshot` — files read from `dir`, service `Running/Auto`, `elevated: true`, processes from `fixtures/tasklist.csv`, events from `fixtures/wevtutil.xml`, `health: None`. Tests then mutate fields.

- [ ] **Step 1: `snapshot.rs`**

```rust
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::parse::{AgentEvent, Credentials, EnvFile, SyncState};
use crate::read::{Read, SourceFile};

#[derive(Debug, Clone, PartialEq)]
pub struct MachineInfo {
    pub hostname: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirSource {
    Service,
    Default,
    Manual,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstallDir {
    /// Canonical (System32) path, as shown to the user.
    pub path: PathBuf,
    pub source: DirSource,
    pub exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Stopped,
    Pending,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartType {
    Auto,
    Manual,
    Disabled,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceInfo {
    pub state: ServiceState,
    pub start_type: StartType,
    pub image_path: String,
    pub exe_exists: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthProbe {
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub server_date: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub taken_at: DateTime<Utc>,
    pub machine: MachineInfo,
    pub install: InstallDir,
    pub elevated: bool,
    pub service: Read<ServiceInfo>,
    pub agent_version: Read<String>,
    pub env: Read<SourceFile<EnvFile>>,
    pub credentials: Read<Credentials>,
    pub sync: Read<SourceFile<SyncState>>,
    pub whitelist: Read<SourceFile<Vec<String>>>,
    pub processes: Read<Vec<String>>,
    pub events: Read<Vec<AgentEvent>>,
    /// None = not probed (`report --offline`, or no SERVER_ADDRESS).
    pub health: Option<Read<HealthProbe>>,
}

impl Snapshot {
    /// Last service start, from the agent's start marker in the Event Log.
    pub fn service_started_at(&self) -> Option<DateTime<Utc>> {
        self.events.as_ok()?.iter().filter(|e| e.kind == crate::parse::EventKind::Start).map(|e| e.time).max()
    }
    /// Manual folder: files from there, service and log from this machine. No service restart offered.
    pub fn is_manual_dir(&self) -> bool {
        self.install.source == DirSource::Manual
    }
}
```
`sync` is a `SourceFile` so its `mtime` (= last successful sync) and bytes are kept; `.parsed` is the `SyncState`.

- [ ] **Step 2: `testkit.rs`**

```rust
//! Snapshots from fixture folders, for tests in this crate and downstream crates.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::parse;
use crate::read::{Read, SourceFile};
use crate::snapshot::*;

fn file<T>(dir: &Path, name: &str, mtime: DateTime<Utc>, f: impl FnOnce(&[u8]) -> T) -> Read<SourceFile<T>> {
    match std::fs::read(dir.join(name)) {
        Ok(b) => Read::Ok(SourceFile::new(b, Some(mtime), f)),
        Err(_) => Read::Missing,
    }
}

pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn snapshot_from_dir(dir: &Path, now: DateTime<Utc>) -> Snapshot {
    let fx = fixtures_dir();
    let tasklist = std::fs::read_to_string(fx.join("tasklist.csv")).unwrap_or_default();
    let events = std::fs::read_to_string(fx.join("wevtutil.xml")).unwrap_or_default();
    let recent = now - chrono::Duration::seconds(12);
    Snapshot {
        taken_at: now,
        machine: MachineInfo { hostname: "LAB-A-05".into(), os: "Windows 10 Pro 19045".into(), arch: "x64".into() },
        install: InstallDir { path: dir.to_path_buf(), source: DirSource::Service, exists: true },
        elevated: true,
        service: Read::Ok(ServiceInfo { state: ServiceState::Running, start_type: StartType::Auto, image_path: dir.join("procsentinel-agent64.exe").display().to_string(), exe_exists: true }),
        agent_version: Read::Ok("3.0.0".into()),
        env: file(dir, crate::agent::ENV_FILE, recent - chrono::Duration::days(3), parse::env),
        credentials: match std::fs::read(dir.join(crate::agent::CREDENTIALS_FILE)) {
            Ok(b) => parse::credentials(&b).map_or_else(Read::Failed, Read::Ok),
            Err(_) => Read::Missing,
        },
        sync: match std::fs::read(dir.join(crate::agent::SYNC_FILE)) {
            Ok(b) => match parse::sync(&b) {
                Ok(s) => Read::Ok(SourceFile::new(b, Some(recent), |_| s)),
                Err(e) => Read::Failed(e),
            },
            Err(_) => Read::Missing,
        },
        whitelist: file(dir, crate::agent::WHITELIST_FILE, recent - chrono::Duration::days(3), parse::whitelist),
        processes: Read::Ok(parse::tasklist(&tasklist)),
        events: parse::wevtutil(&events).map_or_else(Read::Failed, Read::Ok),
        health: None,
    }
}
```
Wire up in `lib.rs`: `#[cfg(any(test, feature = "testkit"))] pub mod testkit;`. Tests in this crate use `now = "2026-10-10T11:15:00Z"` (after the fixture events, so `service_started_at` = 11:10 is after the files' mtime).

- [ ] **Step 3: Write failing diagnosis tests** (in `diagnose.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::Read;
    use crate::snapshot::*;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn now() -> chrono::DateTime<chrono::Utc> {
        "2026-10-10T11:15:00Z".parse().unwrap()
    }
    fn healthy() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), now())
    }
    fn status(d: &Diagnosis, id: CheckId) -> Status {
        d.checks.iter().find(|c| c.id == id).unwrap().status
    }

    #[test]
    fn diagnose_healthy() {
        let d = diagnose(&healthy());
        assert_eq!(d.verdict.kind, VerdictKind::On);
        assert_eq!(status(&d, CheckId::Sync), Status::Ok);
        assert_eq!(status(&d, CheckId::Health), Status::Unknown); // not probed
    }

    #[test]
    fn diagnose_not_installed() {
        let mut s = healthy();
        s.service = Read::Missing;
        s.install.exists = false;
        s.install.source = DirSource::Default;
        s.env = Read::Missing;
        s.sync = Read::Missing;
        s.credentials = Read::Missing;
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::NotInstalled);
    }

    #[test]
    fn diagnose_denied_is_unknown() {
        let mut s = healthy();
        s.env = Read::Denied;
        s.credentials = Read::Denied;
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::ServerAddress), Status::Unknown);
        assert_eq!(status(&d, CheckId::Enrollment), Status::Unknown);
    }

    #[test]
    fn diagnose_stopped_and_stale_and_free() {
        let mut s = healthy();
        if let Read::Ok(svc) = &mut s.service {
            svc.state = ServiceState::Stopped;
        }
        assert_eq!(diagnose(&s).verdict.kind, VerdictKind::Stopped);

        let mut s = healthy();
        if let Read::Ok(f) = &mut s.sync {
            f.mtime = Some(now() - chrono::Duration::hours(30));
        }
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::Stale);
        assert!(d.verdict.text.contains("30 h"));

        let mut s = healthy();
        if let Read::Ok(f) = &mut s.sync {
            f.parsed.mode = crate::parse::Mode::Free;
        }
        assert_eq!(diagnose(&s).verdict.kind, VerdictKind::Free);
    }

    #[test]
    fn diagnose_server_mismatch_means_not_enrolled() {
        let mut s = healthy();
        if let Read::Ok(e) = &mut s.env {
            e.parsed.server_address = Some("https://other.example".into());
        }
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::Enrollment), Status::Error);
        assert_eq!(d.verdict.kind, VerdictKind::NotEnrolled);
    }

    #[test]
    fn diagnose_pending_restart() {
        let mut s = healthy();
        if let Read::Ok(w) = &mut s.whitelist {
            w.mtime = Some(now() - chrono::Duration::minutes(1)); // after the 11:10 start
        }
        assert_eq!(status(&diagnose(&s), CheckId::PendingRestart), Status::Warn);
    }

    #[test]
    fn diagnose_server_refusal_from_log() {
        let d = diagnose(&healthy()); // fixture has "no longer accepts this computer's token"
        assert_eq!(status(&d, CheckId::ServerRefusal), Status::Error);
        assert_eq!(status(&d, CheckId::AgentErrors), Status::Error);
    }

    #[test]
    fn diagnose_clock_skew() {
        let mut s = healthy();
        s.health = Some(Read::Ok(HealthProbe { status: Some(200), latency_ms: 40, server_date: Some(now() - chrono::Duration::minutes(9)), error: None }));
        assert_eq!(status(&diagnose(&s), CheckId::Clock), Status::Warn);
        assert_eq!(status(&diagnose(&s), CheckId::Health), Status::Ok);
    }

    #[test]
    fn diagnose_leftover_token() {
        let mut s = healthy();
        if let Read::Ok(e) = &mut s.env {
            e.parsed.binding_token = Some(crate::read::SecretTail::of("t"));
        }
        assert_eq!(status(&diagnose(&s), CheckId::LeftoverToken), Status::Warn);
    }
}
```
Run: `cargo test -p guardian-core diagnose` — Expected: compile errors.

- [ ] **Step 4: Implement `diagnose.rs`**

```rust
//! Checks and the one-line verdict (spec R6, R7).

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::parse::{Level, Mode};
use crate::read::Read;
use crate::snapshot::{ServiceState, Snapshot, StartType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    Ok,
    Warn,
    Error,
    /// Could not check (no access, not probed). Never counts as a failure.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CheckId {
    Service,
    ServiceExe,
    AgentVersion,
    ServerAddress,
    Health,
    Enrollment,
    LeftoverToken,
    Sync,
    PendingRestart,
    AgentErrors,
    ServerRefusal,
    Clock,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub id: CheckId,
    pub title: &'static str,
    pub status: Status,
    pub fact: String,
    pub todo: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum VerdictKind {
    On,
    Stale,
    NotEnrolled,
    Stopped,
    NotInstalled,
    Free,
    NeverSynced,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub text: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnosis {
    pub verdict: Verdict,
    pub checks: Vec<Check>,
}

impl Diagnosis {
    pub fn worst(&self) -> Status {
        if self.checks.iter().any(|c| c.status == Status::Error) {
            Status::Error
        } else if self.checks.iter().any(|c| c.status == Status::Warn) {
            Status::Warn
        } else {
            Status::Ok
        }
    }
}

fn check(id: CheckId, title: &'static str, status: Status, fact: impl Into<String>, todo: impl Into<String>) -> Check {
    Check { id, title, status, fact: fact.into(), todo: todo.into() }
}

pub fn age(now: DateTime<Utc>, t: DateTime<Utc>) -> String {
    let s = (now - t).num_seconds().max(0);
    match s {
        0..=119 => format!("{s} s"),
        120..=7199 => format!("{} min", s / 60),
        7200..=172_799 => format!("{} h", s / 3600),
        _ => format!("{} days", s / 86_400),
    }
}

fn unknown<T>(r: &Read<T>) -> String {
    r.problem().unwrap_or_default()
}

const REFUSALS: [(&str, &str); 3] = [
    ("token is no longer valid", "The installer's binding token was revoked or expired. Get a new installer token in the cabinet."),
    ("no longer accepts this computer's token", "The computer was unenrolled in the cabinet. Enrol it again with a new binding token."),
    ("computer limit", "The plan's computer limit is reached. Upgrade the plan or unenrol an unused computer."),
];

pub fn diagnose(s: &Snapshot) -> Diagnosis {
    let now = s.taken_at;
    let mut checks = Vec::new();

    // Service
    checks.push(match &s.service {
        Read::Ok(svc) => {
            let st = match (svc.state, svc.start_type) {
                (ServiceState::Running, StartType::Auto) => Status::Ok,
                (ServiceState::Running, _) => Status::Warn,
                _ => Status::Error,
            };
            check(CheckId::Service, "Service", st, format!("{:?}, start type {:?}", svc.state, svc.start_type), match st {
                Status::Ok => "",
                Status::Warn => "Set the service's start type to Automatic so protection survives a reboot.",
                _ => "Start ProcSentinelAgent (services.msc) or restart it from the Configuration tab.",
            })
        }
        Read::Missing => check(CheckId::Service, "Service", Status::Error, "ProcSentinelAgent is not registered", "Install the agent with the installer from the cabinet."),
        other => check(CheckId::Service, "Service", Status::Unknown, unknown(other), "Run as administrator to check the service."),
    });
    checks.push(match &s.service {
        Read::Ok(svc) if svc.exe_exists => check(CheckId::ServiceExe, "Agent executable", Status::Ok, svc.image_path.clone(), ""),
        Read::Ok(svc) => check(CheckId::ServiceExe, "Agent executable", Status::Error, format!("{} does not exist", svc.image_path), "Reinstall the agent."),
        _ => check(CheckId::ServiceExe, "Agent executable", Status::Unknown, "service not readable", ""),
    });
    checks.push(match &s.agent_version {
        Read::Ok(v) => check(CheckId::AgentVersion, "Agent version", Status::Ok, v.clone(), ""),
        other => check(CheckId::AgentVersion, "Agent version", Status::Warn, format!("could not determine: {}", unknown(other)), ""),
    });

    // Config
    let env = s.env.as_ok().map(|f| &f.parsed);
    checks.push(match (&s.env, env.and_then(|e| e.server_address.clone())) {
        (Read::Ok(_), Some(a)) => check(CheckId::ServerAddress, "SERVER_ADDRESS", Status::Ok, a, ""),
        (Read::Ok(_), None) | (Read::Missing, _) => check(CheckId::ServerAddress, "SERVER_ADDRESS", Status::Error, "not set", "Set SERVER_ADDRESS on the Configuration tab."),
        (other, _) => check(CheckId::ServerAddress, "SERVER_ADDRESS", Status::Unknown, unknown(other), "Run as administrator to read .env."),
    });
    checks.push(match &s.health {
        None => check(CheckId::Health, "Server responds", Status::Unknown, "not checked", ""),
        Some(Read::Ok(h)) if h.status == Some(200) => check(CheckId::Health, "Server responds", Status::Ok, format!("200 in {} ms", h.latency_ms), ""),
        Some(Read::Ok(h)) => check(CheckId::Health, "Server responds", Status::Error, h.error.clone().unwrap_or_else(|| format!("HTTP {}", h.status.unwrap_or(0))), "Check the network and SERVER_ADDRESS. The agent keeps enforcing the last policy meanwhile."),
        Some(other) => check(CheckId::Health, "Server responds", Status::Unknown, unknown(other), ""),
    });

    // Enrollment
    let enrolled_here = match (&s.credentials, env.and_then(|e| e.server_address.as_deref())) {
        (Read::Ok(c), Some(addr)) if c.server_address.is_empty() || c.server_address == addr => Some(true),
        (Read::Ok(_), _) => Some(false),
        (Read::Missing, _) => Some(false),
        _ => None,
    };
    checks.push(match (&s.credentials, enrolled_here) {
        (Read::Ok(c), Some(true)) => check(CheckId::Enrollment, "Enrollment", Status::Ok, format!("enrolled since {} (machine {})", c.enrolled_at, c.machine_guid), ""),
        (Read::Ok(c), Some(false)) => check(CheckId::Enrollment, "Enrollment", Status::Error, format!("credentials were issued by {}, but SERVER_ADDRESS is different; the agent treats this computer as not enrolled", c.server_address), "Restore SERVER_ADDRESS, or enrol again with a new binding token."),
        (Read::Missing, _) => check(CheckId::Enrollment, "Enrollment", Status::Error, "not enrolled (no agent_credentials.json)", "Put a binding token from the cabinet into BINDING_TOKEN and restart the service."),
        (other, _) => check(CheckId::Enrollment, "Enrollment", Status::Unknown, unknown(other), "Run as administrator to read the credentials."),
    });
    checks.push(match (env.and_then(|e| e.binding_token.clone()), enrolled_here) {
        (Some(t), Some(true)) => check(CheckId::LeftoverToken, "Leftover BINDING_TOKEN", Status::Warn, format!("present (…{}) although enrolled; the agent ignores it", t.0), "Remove it on the Configuration tab."),
        _ => check(CheckId::LeftoverToken, "Leftover BINDING_TOKEN", Status::Ok, "none", ""),
    });

    // Sync
    let interval = env.map(|e| e.effective_check_interval()).unwrap_or(crate::agent::DEFAULT_SERVICE_INTERVAL_SECS);
    let last_sync = s.sync.as_ok().and_then(|f| f.mtime);
    let stale = last_sync.map(|t| (now - t).num_seconds() > 3 * i64::from(interval));
    checks.push(match (&s.sync, last_sync, stale) {
        (Read::Ok(_), Some(t), Some(false)) => check(CheckId::Sync, "Last sync", Status::Ok, format!("{} ago", age(now, t)), ""),
        (Read::Ok(_), Some(t), _) => check(CheckId::Sync, "Last sync", Status::Warn, format!("{} ago (expected every {interval} s)", age(now, t)), "See Server responds and the Log tab for the reason."),
        (Read::Missing, _, _) => check(CheckId::Sync, "Last sync", Status::Error, "never synced (no sync.json)", "See Enrollment and the Log tab."),
        (Read::Ok(_), None, _) => check(CheckId::Sync, "Last sync", Status::Unknown, "time unknown", ""),
        (other, _, _) => check(CheckId::Sync, "Last sync", if matches!(other, Read::Failed(_)) { Status::Warn } else { Status::Unknown }, unknown(other), ""),
    });

    // Pending restart (R7, R10)
    let started = s.service_started_at();
    let changed_after: Vec<&str> = [
        (crate::agent::ENV_FILE, s.env.as_ok().and_then(|f| f.mtime)),
        (crate::agent::WHITELIST_FILE, s.whitelist.as_ok().and_then(|f| f.mtime)),
    ]
    .into_iter()
    .filter(|(_, m)| matches!((m, started), (Some(m), Some(st)) if *m > st))
    .map(|(n, _)| n)
    .collect();
    checks.push(match started {
        None => check(CheckId::PendingRestart, "Changes applied", Status::Unknown, "service start time unknown (no start record in the Event Log)", ""),
        Some(_) if changed_after.is_empty() => check(CheckId::PendingRestart, "Changes applied", Status::Ok, "files unchanged since the service started", ""),
        Some(st) => check(CheckId::PendingRestart, "Changes applied", Status::Warn, format!("{} changed after the service started ({} ago)", changed_after.join(", "), age(now, st)), "Changes apply after the service restarts."),
    });

    // Event Log
    let day_ago = now - chrono::Duration::hours(24);
    let errors: Vec<String> = s.events.as_ok().map(|ev| ev.iter().filter(|e| e.level == Level::Error && e.time >= day_ago).take(3).map(|e| e.message.clone()).collect()).unwrap_or_default();
    checks.push(match &s.events {
        Read::Ok(_) if errors.is_empty() => check(CheckId::AgentErrors, "Agent errors (24 h)", Status::Ok, "none", ""),
        Read::Ok(_) => check(CheckId::AgentErrors, "Agent errors (24 h)", Status::Error, errors.join(" | "), "See the Log tab."),
        other => check(CheckId::AgentErrors, "Agent errors (24 h)", Status::Unknown, unknown(other), ""),
    });
    let refusal = s.events.as_ok().and_then(|ev| ev.iter().filter(|e| e.time >= day_ago).find_map(|e| REFUSALS.iter().find(|(needle, _)| e.message.contains(needle))));
    checks.push(match refusal {
        Some((needle, todo)) => check(CheckId::ServerRefusal, "Server refusal", Status::Error, format!("the server answered: {needle}"), *todo),
        None => check(CheckId::ServerRefusal, "Server refusal", if s.events.as_ok().is_some() { Status::Ok } else { Status::Unknown }, "none", ""),
    });

    // Clock
    checks.push(match s.health.as_ref().and_then(|h| h.as_ok()).and_then(|h| h.server_date) {
        Some(d) if (now - d).num_minutes().abs() > 5 => check(CheckId::Clock, "Clock", Status::Warn, format!("this computer is {} off the server", age(now.max(d), now.min(d))), "Fix the date and time; TLS and logs depend on them."),
        Some(_) => check(CheckId::Clock, "Clock", Status::Ok, "in sync with the server", ""),
        None => check(CheckId::Clock, "Clock", Status::Unknown, "not checked", ""),
    });

    let verdict = verdict(s, &checks, enrolled_here, last_sync, stale);
    Diagnosis { verdict, checks }
}

fn verdict(s: &Snapshot, _checks: &[Check], enrolled_here: Option<bool>, last_sync: Option<DateTime<Utc>>, stale: Option<bool>) -> Verdict {
    let v = |kind, text: String, hint: &str| Verdict { kind, text, hint: hint.to_string() };
    if matches!(s.service, Read::Missing) && !s.install.exists {
        return v(VerdictKind::NotInstalled, "Not installed".into(), "Service not found, default folder is empty. Choose a folder to inspect a copy.");
    }
    if let Read::Ok(svc) = &s.service {
        if svc.state != ServiceState::Running {
            return v(VerdictKind::Stopped, "Service stopped".into(), "Nothing is enforced while the service is stopped.");
        }
    }
    if enrolled_here == Some(false) {
        return v(VerdictKind::NotEnrolled, "Not enrolled".into(), "The agent enforces the last saved policy but gets no updates.");
    }
    if matches!(s.sync, Read::Missing) {
        return v(VerdictKind::NeverSynced, "Never synced".into(), "See Enrollment and the Log tab.");
    }
    if let Some(f) = s.sync.as_ok() {
        if f.parsed.mode == Mode::Free {
            return v(VerdictKind::Free, "Free mode: nothing is closed".into(), "The computer is not in a room, the room's protection is off, or the room was deleted. Check the cabinet.");
        }
    }
    if stale == Some(true) {
        let ago = last_sync.map(|t| age(s.taken_at, t)).unwrap_or_default();
        return v(VerdictKind::Stale, format!("Protection is on with a stale policy (no contact with the server for {ago})"), "See Server responds and the Log tab.");
    }
    if stale == Some(false) {
        return v(VerdictKind::On, "Protection is on".into(), "");
    }
    v(VerdictKind::Unknown, "Could not check".into(), "Run as administrator to read everything.")
}
```
The `_checks` parameter is kept for later verdict rules; a leading underscore keeps clippy quiet.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p guardian-core && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS. Fix the test or the code — but if a test's *expectation* is wrong against the spec table R7, fix the test to the spec, not the other way round.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): diagnose the agent from a snapshot"
```

---

### Task 4: Enforce simulation and the report

> **Spec correction (apply to R12/R17 in Task 13).** The agent kills with `taskkill /F /IM <rule name>` (`agent/main.go` `killProcess`), which matches the **exact image name** (with `*` wildcards). The blacklist's substring test over the whole `tasklist` text only decides whether to *try*. So rule `code.exe` does **not** close `vscode.exe`; it makes the agent run `taskkill /IM code.exe` every second, which fails and logs `Failed to kill process code.exe: …`. The simulation below implements the real behaviour: a process is closed only on an exact (or wildcard) match, and a substring-only match is reported as a note on a process that is *not* closed.

**Files:**
- Create: `crates/core/src/enforce.rs`, `crates/core/src/report.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `Snapshot`, `Diagnosis`, `diagnose`, `parse::{Mode, AgentEvent, EventKind}`, `agent::is_protected`.
- Produces:
  - `enforce::{ProcessVerdict, verdict_for(&Snapshot, &str) -> ProcessVerdict, image_matches(rule: &str, name: &str) -> bool}`
  - `enforce::looks_locked(&Snapshot) -> bool` (whitelist mode with empty `applications`)
  - `report::{Report { text: String, json: serde_json::Value }, report(&Snapshot, &Diagnosis) -> Report, exit_code(&Diagnosis) -> i32}`
  - `report::kill_summary(&[AgentEvent]) -> Vec<KillSummary { process: String, reason: KillReason, count: usize, first: DateTime<Utc>, last: DateTime<Utc> }>` (used by the Log tab too)

- [ ] **Step 1: Failing tests** (`enforce.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{App, Mode};
    use crate::read::Read;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap(mode: Mode, apps: &[&str]) -> Snapshot {
        let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        if let Read::Ok(f) = &mut s.sync {
            f.parsed.mode = mode;
            f.parsed.applications = apps.iter().map(|n| App { name: n.to_string(), mode: String::new() }).collect();
        }
        s
    }

    #[test]
    fn whitelist_mode() {
        let s = snap(Mode::Whitelist, &["WinWord.exe"]);
        assert_eq!(verdict_for(&s, "explorer.exe"), ProcessVerdict::Protected);
        assert_eq!(verdict_for(&s, "Guardian.exe"), ProcessVerdict::Protected);
        assert_eq!(verdict_for(&s, "NOTEPAD.EXE"), ProcessVerdict::AllowedWhitelistFile);
        assert_eq!(verdict_for(&s, "winword.exe"), ProcessVerdict::AllowedPolicy);
        assert_eq!(verdict_for(&s, "game.exe"), ProcessVerdict::WillClose { rule: None });
    }

    #[test]
    fn blacklist_is_exact_image_name_with_substring_note() {
        let s = snap(Mode::Blacklist, &["code.exe", "steam*"]);
        assert_eq!(verdict_for(&s, "Code.exe"), ProcessVerdict::WillClose { rule: Some("code.exe".into()) });
        assert_eq!(verdict_for(&s, "vscode.exe"), ProcessVerdict::NotAffected { substring_rule: Some("code.exe".into()) });
        assert_eq!(verdict_for(&s, "steamwebhelper.exe"), ProcessVerdict::WillClose { rule: Some("steam*".into()) });
        assert_eq!(verdict_for(&s, "notepad.exe"), ProcessVerdict::NotAffected { substring_rule: None });
        // Protected processes are not exempt in blacklist mode: the agent does not check isSystemProcess there.
        let s = snap(Mode::Blacklist, &["explorer.exe"]);
        assert_eq!(verdict_for(&s, "explorer.exe"), ProcessVerdict::WillClose { rule: Some("explorer.exe".into()) });
    }

    #[test]
    fn free_and_unknown() {
        assert_eq!(verdict_for(&snap(Mode::Free, &[]), "game.exe"), ProcessVerdict::NotAffected { substring_rule: None });
        let mut s = snap(Mode::Whitelist, &[]);
        assert!(looks_locked(&s));
        s.sync = Read::Missing;
        assert!(matches!(verdict_for(&s, "game.exe"), ProcessVerdict::Unknown(_)));
    }

    #[test]
    fn wildcards() {
        assert!(image_matches("STEAM*", "steam.exe"));
        assert!(image_matches("*.scr", "x.scr"));
        assert!(!image_matches("code.exe", "vscode.exe"));
        assert!(image_matches("a*b*c", "aXXbYYc"));
    }
}
```
Run: `cargo test -p guardian-core enforce` — Expected: compile errors.

- [ ] **Step 2: Implement `enforce.rs`**

```rust
//! What the agent's `enforce` would do to a process right now (spec R16–R18).

use crate::agent;
use crate::parse::Mode;
use crate::read::Read;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessVerdict {
    /// On the agent's hardcoded list (whitelist mode only).
    Protected,
    AllowedWhitelistFile,
    AllowedPolicy,
    /// `rule` is the blacklist rule that matches; `None` in whitelist mode.
    WillClose { rule: Option<String> },
    /// `substring_rule`: a blacklist rule contained in this name — the agent tries
    /// `taskkill /IM <rule>` every second, which does not touch this process.
    NotAffected { substring_rule: Option<String> },
    Unknown(String),
}

impl ProcessVerdict {
    pub fn label(&self) -> String {
        match self {
            ProcessVerdict::Protected => "Protected by the system".into(),
            ProcessVerdict::AllowedWhitelistFile => "Allowed (whitelist.txt)".into(),
            ProcessVerdict::AllowedPolicy => "Allowed (policy)".into(),
            ProcessVerdict::WillClose { rule: Some(r) } => format!("Will be closed: blacklist rule `{r}`"),
            ProcessVerdict::WillClose { rule: None } => "Will be closed: whitelist mode, not in the room policy or whitelist.txt".into(),
            ProcessVerdict::NotAffected { substring_rule: Some(r) } => format!("Not affected (substring match: rule `{r}` makes the agent try to close `{r}`, not this process)"),
            ProcessVerdict::NotAffected { substring_rule: None } => "Not affected".into(),
            ProcessVerdict::Unknown(why) => format!("Unknown: {why}"),
        }
    }
    pub fn will_close(&self) -> bool {
        matches!(self, ProcessVerdict::WillClose { .. })
    }
}

/// taskkill /IM semantics: case-insensitive, `*` matches any run of characters.
pub fn image_matches(rule: &str, name: &str) -> bool {
    let (rule, name) = (rule.to_lowercase(), name.to_lowercase());
    let parts: Vec<&str> = rule.split('*').collect();
    if parts.len() == 1 {
        return rule == name;
    }
    let mut rest = name.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(pos) = rest.find(part) else { return false };
            rest = &rest[pos + part.len()..];
        }
    }
    true
}

pub fn looks_locked(s: &Snapshot) -> bool {
    s.sync.as_ok().is_some_and(|f| f.parsed.mode == Mode::Whitelist && f.parsed.applications.is_empty())
}

pub fn verdict_for(s: &Snapshot, process: &str) -> ProcessVerdict {
    let policy = match &s.sync {
        Read::Ok(f) => &f.parsed,
        other => return ProcessVerdict::Unknown(format!("no policy ({})", other.problem().unwrap_or_default())),
    };
    let lower = process.trim().to_lowercase();
    match policy.mode {
        Mode::Whitelist => {
            if agent::is_protected(&lower) {
                ProcessVerdict::Protected
            } else if s.whitelist.as_ok().is_some_and(|w| w.parsed.contains(&lower)) {
                ProcessVerdict::AllowedWhitelistFile
            } else if policy.applications.iter().any(|a| a.name.to_lowercase() == lower) {
                ProcessVerdict::AllowedPolicy
            } else {
                ProcessVerdict::WillClose { rule: None }
            }
        }
        Mode::Blacklist => {
            let rules = policy.applications.iter().map(|a| a.name.as_str()).filter(|n| !n.is_empty());
            let mut substring = None;
            for r in rules {
                if image_matches(r, &lower) {
                    return ProcessVerdict::WillClose { rule: Some(r.to_string()) };
                }
                if substring.is_none() && lower.contains(&r.to_lowercase()) {
                    substring = Some(r.to_string());
                }
            }
            ProcessVerdict::NotAffected { substring_rule: substring }
        }
        Mode::Free | Mode::Other(_) => ProcessVerdict::NotAffected { substring_rule: None },
    }
}
```
Wait-check against the agent: in whitelist mode the agent checks `allowedSet` then `isSystemProcess` (builtin + `whitelist.txt`); the *outcome* is identical regardless of order, so the label order follows spec R16.

- [ ] **Step 3: Failing report tests** (`report.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnose::diagnose;
    use crate::read::Read;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    #[test]
    fn report_has_no_secrets() {
        let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        // Put a binding token in .env too.
        s.env = Read::Ok(crate::read::SourceFile::new(b"SERVER_ADDRESS=https://guardian.example.com\nBINDING_TOKEN=bt_secret_xyz\n".to_vec(), None, crate::parse::env));
        let r = report(&s, &diagnose(&s));
        for secret in ["agt_secret_value_123", "bt_secret_xyz"] {
            assert!(!r.text.contains(secret));
            assert!(!r.json.to_string().contains(secret));
        }
        assert!(r.text.contains("LAB-A-05"));
        assert!(r.text.contains("game.exe"));
        assert_eq!(r.json["verdict"]["kind"], "On");
        assert_eq!(r.json["config"]["binding_token"], format!("present (…{})", crate::read::SecretTail::of("bt_secret_xyz").0));
    }

    #[test]
    fn exit_codes() {
        let s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        let d = diagnose(&s);
        assert_eq!(exit_code(&d), 11); // fixture log has an error
    }

    #[test]
    fn kill_summary_groups() {
        let s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        let k = kill_summary(s.events.as_ok().unwrap());
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].process, "game.exe");
        assert_eq!(k[0].count, 1);
    }
}
```
The healthy fixture is enrolled and its sync is fresh, so the verdict is `On` even though the log has an error (the verdict summarises protection; the error shows up in the checks and the exit code).

- [ ] **Step 4: Implement `report.rs`**

```rust
//! Text and JSON report (spec R24, R25). Built from parsed data only: secrets never reach it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::diagnose::{Diagnosis, Status};
use crate::enforce::verdict_for;
use crate::parse::{AgentEvent, EventKind, KillReason};
use crate::snapshot::Snapshot;

pub struct Report {
    pub text: String,
    pub json: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSummary {
    pub process: String,
    pub reason: KillReason,
    pub count: usize,
    pub first: DateTime<Utc>,
    pub last: DateTime<Utc>,
}

/// Kills grouped by (process, reason), most recent first.
pub fn kill_summary(events: &[AgentEvent]) -> Vec<KillSummary> {
    let mut map: BTreeMap<(String, String), KillSummary> = BTreeMap::new();
    for e in events {
        if let EventKind::Kill { process, reason } = &e.kind {
            let key = (process.to_lowercase(), format!("{reason:?}"));
            let k = map.entry(key).or_insert(KillSummary { process: process.clone(), reason: *reason, count: 0, first: e.time, last: e.time });
            k.count += 1;
            k.first = k.first.min(e.time);
            k.last = k.last.max(e.time);
        }
    }
    let mut v: Vec<KillSummary> = map.into_values().collect();
    v.sort_by(|a, b| b.last.cmp(&a.last));
    v
}

pub fn exit_code(d: &Diagnosis) -> i32 {
    match d.worst() {
        Status::Error => 11,
        Status::Warn => 10,
        _ => 0,
    }
}

pub fn report(s: &Snapshot, d: &Diagnosis) -> Report {
    let env = s.env.as_ok().map(|f| &f.parsed);
    let config = json!({
        "install_dir": s.install.path.display().to_string(),
        "install_dir_source": format!("{:?}", s.install.source),
        "server_address": env.and_then(|e| e.server_address.clone()),
        "check_interval": env.and_then(|e| e.check_interval_raw.clone()),
        "binding_token": env.map(|e| match &e.binding_token { Some(t) => format!("present (…{})", t.0), None => "absent".into() }),
        "unknown_keys": env.map(|e| e.unknown.clone()),
        "credentials": s.credentials.as_ok().map(|c| json!({"machine_guid": c.machine_guid, "server_address": c.server_address, "enrolled_at": c.enrolled_at, "agent_token": c.token.as_ref().map(|t| format!("present (…{})", t.0))})),
        "whitelist": s.whitelist.as_ok().map(|w| w.parsed.clone()),
    });
    let policy = s.sync.as_ok().map(|f| json!({"mode": f.parsed.mode.as_str(), "synced_at": f.mtime, "applications": f.parsed.applications.iter().map(|a| a.name.clone()).collect::<Vec<_>>()}));
    let processes: Vec<Value> = s.processes.as_ok().map(|p| p.iter().map(|n| json!({"name": n, "verdict": verdict_for(s, n).label()})).collect()).unwrap_or_default();
    let events: Vec<&AgentEvent> = s.events.as_ok().map(|e| e.iter().take(500).collect()).unwrap_or_default();
    let json = json!({
        "taken_at": s.taken_at,
        "machine": {"hostname": s.machine.hostname, "os": s.machine.os, "arch": s.machine.arch},
        "versions": {"agent": s.agent_version.as_ok(), "guardian": env!("CARGO_PKG_VERSION")},
        "verdict": d.verdict,
        "checks": d.checks,
        "config": config,
        "policy": policy,
        "processes": processes,
        "events": events,
    });

    let mut t = String::new();
    let mut line = |l: String| {
        t.push_str(&l);
        t.push_str("\r\n");
    };
    line(format!("Guardian report — {} — {}", s.machine.hostname, s.taken_at.to_rfc3339()));
    line(format!("{} ({}) | agent {} | guardian {}", s.machine.os, s.machine.arch, s.agent_version.as_ok().map(String::as_str).unwrap_or("?"), env!("CARGO_PKG_VERSION")));
    line(String::new());
    line(format!("VERDICT: {}", d.verdict.text));
    if !d.verdict.hint.is_empty() {
        line(format!("  {}", d.verdict.hint));
    }
    line(String::new());
    for c in &d.checks {
        line(format!("[{:?}] {}: {}", c.status, c.title, c.fact));
        if !c.todo.is_empty() && c.status != Status::Ok {
            line(format!("        -> {}", c.todo));
        }
    }
    line(String::new());
    line("CONFIGURATION".into());
    line(serde_json::to_string_pretty(&json["config"]).unwrap_or_default());
    line(String::new());
    line("POLICY".into());
    line(serde_json::to_string_pretty(&json["policy"]).unwrap_or_default());
    line(String::new());
    line("PROCESSES".into());
    for p in &processes {
        line(format!("  {:<32} {}", p["name"].as_str().unwrap_or(""), p["verdict"].as_str().unwrap_or("")));
    }
    line(String::new());
    line("EVENT LOG (newest first)".into());
    for e in &events {
        line(format!("  {}  {:?}  {}", e.time.to_rfc3339(), e.level, e.message));
    }
    Report { text: t, json }
}
```
`Verdict`, `Check` already derive `Serialize` (Task 3); `AgentEvent` derives `Serialize` (Task 2).

- [ ] **Step 5: Run and commit**

Run: `cargo test -p guardian-core && cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): simulate the agent's enforcement and build the report"
```

---

### Task 5: Edit model — normalisation and validation

**Files:**
- Create: `crates/core/src/edit/mod.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod edit;`)

**Interfaces:**
- Consumes: `Snapshot`, `parse::EnvFile`, `agent::{is_protected}`.
- Produces:
  - `edit::TokenEdit { Keep, Set(String), Remove }`
  - `edit::EnvEdit { server_address: String, check_interval: String, binding_token: TokenEdit }` with `EnvEdit::from_snapshot(&Snapshot) -> EnvEdit` (current values, token `Keep`). `check_interval` is the raw field text; empty means "remove the key".
  - `edit::WhitelistEdit { entries: Vec<String> }` with `WhitelistEdit::from_snapshot(&Snapshot)` (current entries, deduplicated, file order)
  - `edit::{Severity { Error, Warning }, Field { ServerAddress, CheckInterval, BindingToken, Whitelist }, Issue { severity, field, message: String }}`
  - `edit::normalize_name(&str) -> String`
  - `edit::check_new_entry(current: &[String], raw: &str) -> Result<(String, Vec<Issue>), Issue>`
  - `edit::validate(&Snapshot, &EnvEdit, &WhitelistEdit) -> Vec<Issue>`
  - `edit::canonical_server_address(&str) -> String` (trim + trailing `/` removed — what the agent will see)

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }
    fn errors(v: &[Issue]) -> Vec<Field> {
        v.iter().filter(|i| i.severity == Severity::Error).map(|i| i.field).collect()
    }
    fn warns(v: &[Issue]) -> usize {
        v.iter().filter(|i| i.severity == Severity::Warning).count()
    }

    #[test]
    fn normalize_cases() {
        assert_eq!(normalize_name(r#"  "C:\Program Files\Microsoft Office\WINWORD.EXE"  "#), "winword.exe");
        assert_eq!(normalize_name("'Notepad.exe'"), "notepad.exe");
        assert_eq!(normalize_name("/usr/bin/foo"), "foo");
    }

    #[test]
    fn check_new_entry_cases() {
        let cur = vec!["notepad.exe".to_string()];
        assert_eq!(check_new_entry(&cur, "WINWORD.EXE").unwrap(), ("winword.exe".to_string(), vec![]));
        let (n, w) = check_new_entry(&cur, "notepad2").unwrap();
        assert_eq!(n, "notepad2");
        assert_eq!(w.len(), 1); // no .exe
        let (_, w) = check_new_entry(&cur, "explorer.exe").unwrap();
        assert!(w[0].message.contains("already protected"));
        assert!(check_new_entry(&cur, "Notepad.EXE").is_err()); // duplicate
        assert!(check_new_entry(&cur, "   ").is_err());
        assert!(check_new_entry(&cur, "#x.exe").is_err());
        assert!(check_new_entry(&cur, "a\nb.exe").is_err());
    }

    #[test]
    fn from_snapshot_roundtrips_current_values() {
        let s = snap();
        let e = EnvEdit::from_snapshot(&s);
        assert_eq!(e.server_address, "https://guardian.example.com");
        assert_eq!(e.check_interval, "30");
        assert_eq!(e.binding_token, TokenEdit::Keep);
        assert!(validate(&s, &e, &WhitelistEdit::from_snapshot(&s)).is_empty());
    }

    #[test]
    fn server_address_rules() {
        let s = snap();
        let wl = WhitelistEdit::from_snapshot(&s);
        let with = |a: &str| EnvEdit { server_address: a.into(), ..EnvEdit::from_snapshot(&s) };
        for bad in ["", "guardian.example.com", "ftp://x", "https://", "https://x/api", "https://x?y", "https://x:99999", "https://x:ab", " https://x", "https://x y", "#https://x"] {
            assert_eq!(errors(&validate(&s, &with(bad), &wl)), vec![Field::ServerAddress], "{bad:?}");
        }
        for good in ["https://guardian.example.com/", "http://10.0.0.5:8080", "https://guardian.example.com"] {
            assert!(errors(&validate(&s, &with(good), &wl)).is_empty(), "{good:?}");
        }
        assert_eq!(canonical_server_address("https://a.example//"), "https://a.example");
    }

    #[test]
    fn server_change_without_token_warns_drop_out() {
        let s = snap();
        let e = EnvEdit { server_address: "https://other.example".into(), ..EnvEdit::from_snapshot(&s) };
        let v = validate(&s, &e, &WhitelistEdit::from_snapshot(&s));
        assert!(v.iter().any(|i| i.severity == Severity::Warning && i.message.contains("drop out of the fleet")));
        let e = EnvEdit { binding_token: TokenEdit::Set("tok".into()), ..e };
        assert!(!validate(&s, &e, &WhitelistEdit::from_snapshot(&s)).iter().any(|i| i.message.contains("drop out")));
    }

    #[test]
    fn token_while_enrolled_warns_ignored() {
        let s = snap();
        let e = EnvEdit { binding_token: TokenEdit::Set("tok".into()), ..EnvEdit::from_snapshot(&s) };
        let v = validate(&s, &e, &WhitelistEdit::from_snapshot(&s));
        assert!(v.iter().any(|i| i.field == Field::BindingToken && i.message.contains("Unenrol it in the cabinet first")));
        let e = EnvEdit { binding_token: TokenEdit::Set(" tok".into()), ..EnvEdit::from_snapshot(&s) };
        assert_eq!(errors(&validate(&s, &e, &WhitelistEdit::from_snapshot(&s))), vec![Field::BindingToken]);
    }

    #[test]
    fn check_interval_rules() {
        let s = snap();
        let wl = WhitelistEdit::from_snapshot(&s);
        let with = |v: &str| EnvEdit { check_interval: v.into(), ..EnvEdit::from_snapshot(&s) };
        assert!(validate(&s, &with(""), &wl).is_empty());
        assert_eq!(errors(&validate(&s, &with("0"), &wl)), vec![Field::CheckInterval]);
        assert_eq!(errors(&validate(&s, &with("86401"), &wl)), vec![Field::CheckInterval]);
        assert_eq!(errors(&validate(&s, &with("ten"), &wl)), vec![Field::CheckInterval]);
        assert_eq!(warns(&validate(&s, &with("3"), &wl)), 1);
    }
}
```
Run: `cargo test -p guardian-core edit` — Expected: compile errors.

- [ ] **Step 2: Implement `edit/mod.rs`**

```rust
//! The edit model (spec R11–R13): pure, no I/O. The UI holds an `EnvEdit` and a
//! `WhitelistEdit` as its draft; `plan` (plan.rs) turns them into bytes to write.

pub mod plan;
pub mod render;

use crate::agent;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEdit {
    Keep,
    Set(String),
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEdit {
    pub server_address: String,
    /// Raw field text; empty = remove the key (agent default applies).
    pub check_interval: String,
    pub binding_token: TokenEdit,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhitelistEdit {
    pub entries: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    ServerAddress,
    CheckInterval,
    BindingToken,
    Whitelist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub field: Field,
    pub message: String,
}

fn err(field: Field, m: impl Into<String>) -> Issue {
    Issue { severity: Severity::Error, field, message: m.into() }
}
fn warn(field: Field, m: impl Into<String>) -> Issue {
    Issue { severity: Severity::Warning, field, message: m.into() }
}

impl EnvEdit {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        let e = s.env.as_ok().map(|f| &f.parsed);
        EnvEdit {
            server_address: e.and_then(|e| e.server_address.clone()).unwrap_or_default(),
            check_interval: e.and_then(|e| e.check_interval_raw.clone()).unwrap_or_default(),
            binding_token: TokenEdit::Keep,
        }
    }
}

impl WhitelistEdit {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        let mut entries: Vec<String> = Vec::new();
        for e in s.whitelist.as_ok().map(|f| f.parsed.clone()).unwrap_or_default() {
            if !entries.contains(&e) {
                entries.push(e);
            }
        }
        WhitelistEdit { entries }
    }
}

/// tools/whitelist-gui normalizeName: trim, lower case, strip quotes, keep only the file name.
pub fn normalize_name(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    match s.rfind(['\\', '/']) {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

pub fn check_new_entry(current: &[String], raw: &str) -> Result<(String, Vec<Issue>), Issue> {
    if raw.contains(['\n', '\r']) {
        return Err(err(Field::Whitelist, "A name cannot contain a line break."));
    }
    let name = normalize_name(raw);
    if name.is_empty() {
        return Err(err(Field::Whitelist, "Type a program name, e.g. winword.exe."));
    }
    if name.starts_with('#') {
        return Err(err(Field::Whitelist, "A name starting with # would be read as a comment."));
    }
    if current.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
        return Err(err(Field::Whitelist, format!("{name} is already in the list.")));
    }
    let mut w = Vec::new();
    if !name.ends_with(".exe") {
        w.push(warn(Field::Whitelist, "tasklist reports names with .exe; without it this entry may never match."));
    }
    if agent::is_protected(&name) {
        w.push(warn(Field::Whitelist, format!("{name} is already protected by the agent; the entry changes nothing.")));
    }
    Ok((name, w))
}

pub fn canonical_server_address(v: &str) -> String {
    v.trim().trim_end_matches('/').to_string()
}

/// R13: anything the agent's TrimSpace or comment rule would alter is refused.
fn env_value_problem(v: &str) -> Option<&'static str> {
    if v.contains(['\n', '\r']) {
        Some("cannot contain a line break")
    } else if v.starts_with('#') {
        Some("cannot start with #")
    } else if v != v.trim() {
        Some("cannot start or end with a space")
    } else {
        None
    }
}

fn server_address_problem(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return Some("SERVER_ADDRESS is required.".into());
    }
    if let Some(p) = env_value_problem(raw) {
        return Some(format!("SERVER_ADDRESS {p}."));
    }
    let v = canonical_server_address(raw);
    let rest = v.strip_prefix("https://").or_else(|| v.strip_prefix("http://"));
    let Some(rest) = rest else { return Some("SERVER_ADDRESS must start with http:// or https://.".into()) };
    if rest.contains(['/', '?', '#', ' ', '\t']) {
        return Some("SERVER_ADDRESS is a server, not a page: no path, query or spaces.".into());
    }
    // IPv6 literals are out of scope: schools use host names or IPv4.
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if host.is_empty() {
        return Some("SERVER_ADDRESS has no host.".into());
    }
    if let Some(p) = port {
        if !matches!(p.parse::<u32>(), Ok(1..=65535)) {
            return Some("The port must be a number from 1 to 65535.".into());
        }
    }
    None
}

pub fn validate(s: &Snapshot, env: &EnvEdit, _wl: &WhitelistEdit) -> Vec<Issue> {
    let mut out = Vec::new();
    if let Some(m) = server_address_problem(&env.server_address) {
        out.push(err(Field::ServerAddress, m));
    }
    let ci = env.check_interval.trim();
    if env.check_interval != ci {
        out.push(err(Field::CheckInterval, "CHECK_INTERVAL cannot start or end with a space."));
    } else if !ci.is_empty() {
        match ci.parse::<u32>() {
            Ok(n @ 1..=86_400) => {
                if n < 5 {
                    out.push(warn(Field::CheckInterval, "A short interval adds load on the server."));
                }
            }
            _ => out.push(err(Field::CheckInterval, "CHECK_INTERVAL must be a whole number of seconds from 1 to 86400.")),
        }
    }
    if let TokenEdit::Set(t) = &env.binding_token {
        if t.is_empty() {
            out.push(err(Field::BindingToken, "The new binding token is empty."));
        } else if let Some(p) = env_value_problem(t) {
            out.push(err(Field::BindingToken, format!("BINDING_TOKEN {p}.")));
        }
    }

    let new_addr = canonical_server_address(&env.server_address);
    if let Some(c) = s.credentials.as_ok() {
        let enrolled_to_new = c.server_address.is_empty() || c.server_address == new_addr;
        let token_set = matches!(env.binding_token, TokenEdit::Set(_));
        if !enrolled_to_new && !token_set {
            out.push(warn(Field::ServerAddress, "This computer will drop out of the fleet until it gets a new binding token."));
        }
        if enrolled_to_new && token_set {
            out.push(warn(Field::BindingToken, "The agent ignores a binding token while this computer is enrolled. Unenrol it in the cabinet first, then save the token again."));
        }
    }
    out
}
```
Create empty `edit/plan.rs` and `edit/render.rs` (Task 6 fills them) so the module compiles.

- [ ] **Step 3: Run and commit**

Run: `cargo test -p guardian-core edit && cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): model and validate an edit of the agent's config"
```

---

### Task 6: Rendering the edited files and the write plan

**Files:**
- Modify: `crates/core/src/edit/render.rs`, `crates/core/src/edit/plan.rs`
- Test: `crates/core/tests/roundtrip.rs`, unit tests in `render.rs` and `plan.rs`

**Interfaces:**
- Consumes: `EnvEdit`, `WhitelistEdit`, `TokenEdit`, `canonical_server_address`, `parse::{env_pairs, whitelist}`, `read::{detect_encoding, detect_line_ending, Encoding, LineEnding}`.
- Produces:
  - `render::{RenderError { Encoding(&'static str), TooLarge }, EnvChange { Set(&'static str, String), Remove(&'static str) }, render_env(original: &[u8], changes: &[EnvChange]) -> Result<Vec<u8>, RenderError>, render_whitelist(original: &[u8], entries: &[String]) -> Result<Vec<u8>, RenderError>, EMPTY_WHITELIST_COMMENT: &str}`
  - `plan::{FileKind { Env, Whitelist }, Expected { Absent, Hash(String) }, FileWrite { kind, path: PathBuf, expected: Expected, new_bytes: Vec<u8>, summary: Vec<String> }, WritePlan { files: Vec<FileWrite> }, plan(&Snapshot, &EnvEdit, &WhitelistEdit) -> Result<WritePlan, Vec<Issue>>, env_changes(&Snapshot, &EnvEdit) -> Vec<EnvChange>}`
  - `WritePlan::is_empty()`; `FileWrite::summary` is the human diff for the confirm dialog (`"SERVER_ADDRESS: a → b"`, `"BINDING_TOKEN: set"`, `"+ winword.exe"`, `"− game.exe"`).

- [ ] **Step 1: Failing render tests** (`render.rs`)

```rust
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
        let out = render_env(b"BINDING_TOKEN=a\r\nX=1\r\n  BINDING_TOKEN = b\r\n#BINDING_TOKEN=c\r\n", &[EnvChange::Remove("BINDING_TOKEN")]).unwrap();
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
}
```
Run: `cargo test -p guardian-core render` — Expected: compile errors.

- [ ] **Step 2: Implement `render.rs`**

```rust
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
    if let Some(last) = lines.last_mut() {
        if !last.ends_with('\n') {
            last.push_str(e);
        }
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
```

- [ ] **Step 3: Run render tests** — `cargo test -p guardian-core render` — Expected: PASS.

- [ ] **Step 4: Failing plan tests** (`plan.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{EnvEdit, TokenEdit, WhitelistEdit};
    use crate::read::Read;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }

    #[test]
    fn unchanged_draft_writes_nothing() {
        let s = snap();
        let p = plan(&s, &EnvEdit::from_snapshot(&s), &WhitelistEdit::from_snapshot(&s)).unwrap();
        assert!(p.is_empty());
    }

    #[test]
    fn trailing_slash_only_is_not_a_change() {
        let s = snap();
        let e = EnvEdit { server_address: "https://guardian.example.com/".into(), ..EnvEdit::from_snapshot(&s) };
        assert!(plan(&s, &e, &WhitelistEdit::from_snapshot(&s)).unwrap().is_empty());
    }

    #[test]
    fn env_and_whitelist_changes() {
        let s = snap();
        let e = EnvEdit { check_interval: String::new(), binding_token: TokenEdit::Set("bt_new".into()), ..EnvEdit::from_snapshot(&s) };
        let mut wl = WhitelistEdit::from_snapshot(&s);
        wl.entries.retain(|x| x != "calc.exe");
        wl.entries.push("winword.exe".into());
        let p = plan(&s, &e, &wl).unwrap();
        assert_eq!(p.files.len(), 2);
        let env = &p.files[0];
        assert_eq!(env.kind, FileKind::Env);
        assert_eq!(env.expected, Expected::Hash(s.env.as_ok().unwrap().hash.clone()));
        assert_eq!(String::from_utf8(env.new_bytes.clone()).unwrap(), "SERVER_ADDRESS=https://guardian.example.com\r\nBINDING_TOKEN=bt_new\r\n");
        assert_eq!(env.summary, vec!["CHECK_INTERVAL: 30 → (default)".to_string(), "BINDING_TOKEN: set".to_string()]);
        assert!(!format!("{:?}", env.summary).contains("bt_new"));
        assert_eq!(p.files[1].summary, vec!["+ winword.exe".to_string(), "− calc.exe".to_string()]);
        assert_eq!(p.files[1].path, fixtures_dir().join("healthy").join("whitelist.txt"));
    }

    #[test]
    fn missing_files_expect_absent() {
        let mut s = snap();
        s.whitelist = Read::Missing;
        let wl = WhitelistEdit { entries: vec!["a.exe".into()] };
        let p = plan(&s, &EnvEdit::from_snapshot(&s), &wl).unwrap();
        assert_eq!(p.files[0].expected, Expected::Absent);
    }

    #[test]
    fn errors_block_the_plan_and_unreadable_files_refuse() {
        let s = snap();
        let e = EnvEdit { server_address: "nope".into(), ..EnvEdit::from_snapshot(&s) };
        assert!(plan(&s, &e, &WhitelistEdit::from_snapshot(&s)).is_err());
        let mut s = snap();
        s.env = Read::Denied;
        assert!(plan(&s, &EnvEdit { check_interval: "9".into(), ..EnvEdit::from_snapshot(&snap()) }, &WhitelistEdit::from_snapshot(&s)).is_err());
    }
}
```

- [ ] **Step 5: Implement `plan.rs`**

```rust
//! Turns a validated draft into exactly the bytes to write (spec R14).

use std::path::PathBuf;

use super::render::{EnvChange, render_env, render_whitelist};
use super::{EnvEdit, Field, Issue, Severity, TokenEdit, WhitelistEdit, canonical_server_address, validate};
use crate::agent;
use crate::read::Read;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Env,
    Whitelist,
}

impl FileKind {
    pub fn file_name(self) -> &'static str {
        match self {
            FileKind::Env => agent::ENV_FILE,
            FileKind::Whitelist => agent::WHITELIST_FILE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    Absent,
    Hash(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub kind: FileKind,
    /// Canonical path; winhost maps it to an I/O path.
    pub path: PathBuf,
    pub expected: Expected,
    pub new_bytes: Vec<u8>,
    pub summary: Vec<String>,
}

impl std::fmt::Debug for FileWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWrite").field("kind", &self.kind).field("path", &self.path).field("expected", &self.expected).field("len", &self.new_bytes.len()).field("summary", &self.summary).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WritePlan {
    pub files: Vec<FileWrite>,
}

impl WritePlan {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// The `.env` changes this draft makes relative to what the agent currently reads.
pub fn env_changes(s: &Snapshot, e: &EnvEdit) -> (Vec<EnvChange>, Vec<String>) {
    let cur = s.env.as_ok().map(|f| f.parsed.clone()).unwrap_or_default();
    let mut ch = Vec::new();
    let mut summary = Vec::new();
    let new_addr = canonical_server_address(&e.server_address);
    if cur.server_address.as_deref() != Some(new_addr.as_str()) {
        summary.push(format!("SERVER_ADDRESS: {} → {}", cur.server_address.as_deref().unwrap_or("(none)"), new_addr));
        ch.push(EnvChange::Set(agent::KEY_SERVER_ADDRESS, new_addr));
    }
    let new_ci = e.check_interval.trim();
    if cur.check_interval_raw.as_deref().unwrap_or("") != new_ci {
        let old = cur.check_interval_raw.as_deref().unwrap_or("(default)");
        if new_ci.is_empty() {
            summary.push(format!("CHECK_INTERVAL: {old} → (default)"));
            ch.push(EnvChange::Remove(agent::KEY_CHECK_INTERVAL));
        } else {
            summary.push(format!("CHECK_INTERVAL: {old} → {new_ci}"));
            ch.push(EnvChange::Set(agent::KEY_CHECK_INTERVAL, new_ci.to_string()));
        }
    }
    match &e.binding_token {
        TokenEdit::Keep => {}
        TokenEdit::Set(t) => {
            summary.push("BINDING_TOKEN: set".into());
            ch.push(EnvChange::Set(agent::KEY_BINDING_TOKEN, t.clone()));
        }
        TokenEdit::Remove => {
            summary.push("BINDING_TOKEN: removed".into());
            ch.push(EnvChange::Remove(agent::KEY_BINDING_TOKEN));
        }
    }
    (ch, summary)
}

fn source<T>(r: &Read<crate::read::SourceFile<T>>, field: Field) -> Result<(Vec<u8>, Expected), Issue> {
    match r {
        Read::Ok(f) => Ok((f.bytes.clone(), Expected::Hash(f.hash.clone()))),
        Read::Missing => Ok((Vec::new(), Expected::Absent)),
        other => Err(Issue { severity: Severity::Error, field, message: format!("Cannot edit: {}", other.problem().unwrap_or_default()) }),
    }
}

pub fn plan(s: &Snapshot, env: &EnvEdit, wl: &WhitelistEdit) -> Result<WritePlan, Vec<Issue>> {
    let errors: Vec<Issue> = validate(s, env, wl).into_iter().filter(|i| i.severity == Severity::Error).collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut files = Vec::new();

    let (changes, summary) = env_changes(s, env);
    if !changes.is_empty() {
        let (bytes, expected) = source(&s.env, Field::ServerAddress).map_err(|e| vec![e])?;
        let new_bytes = render_env(&bytes, &changes).map_err(|e| vec![Issue { severity: Severity::Error, field: Field::ServerAddress, message: e.to_string() }])?;
        files.push(FileWrite { kind: FileKind::Env, path: s.install.path.join(agent::ENV_FILE), expected, new_bytes, summary });
    }

    let current = super::WhitelistEdit::from_snapshot(s).entries;
    if current != wl.entries || (matches!(s.whitelist, Read::Missing) && !wl.entries.is_empty()) {
        let (bytes, expected) = source(&s.whitelist, Field::Whitelist).map_err(|e| vec![e])?;
        let new_bytes = render_whitelist(&bytes, &wl.entries).map_err(|e| vec![Issue { severity: Severity::Error, field: Field::Whitelist, message: e.to_string() }])?;
        let mut summary: Vec<String> = wl.entries.iter().filter(|e| !current.contains(e)).map(|e| format!("+ {e}")).collect();
        summary.extend(current.iter().filter(|c| !wl.entries.contains(c)).map(|c| format!("− {c}")));
        files.push(FileWrite { kind: FileKind::Whitelist, path: s.install.path.join(agent::WHITELIST_FILE), expected, new_bytes, summary });
    }
    Ok(WritePlan { files })
}
```

- [ ] **Step 6: Round-trip test against the agent's parsing** (`tests/roundtrip.rs`)

```rust
//! Spec R29: whatever we write, the agent must read back exactly the edit.

use guardian_core::edit::render::{EnvChange, render_env, render_whitelist};
use guardian_core::parse::{env_pairs, whitelist};

/// The agent's readEnvFile: last occurrence wins.
fn agent_get(bytes: &[u8], key: &str) -> Option<String> {
    env_pairs(&String::from_utf8_lossy(bytes)).into_iter().filter(|(k, _)| k == key).last().map(|(_, v)| v)
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
```
The two BOM sources are deliberate: after a BOM the first key is `\u{feff}KEY`, invisible to the agent (Go's `TrimSpace` does not strip U+FEFF, and neither does Rust's `trim`). The round-trip asserts what the agent reads, so a `Set` of `SERVER_ADDRESS` on `b"\xEF\xBB\xBFSERVER_ADDRESS=https://a\r\n"` must append a readable line rather than rewrite the invisible one — which is what `decode` keeping U+FEFF in the text achieves. Diagnosis already flags a BOM `.env` (Task 2 `env_bom_is_not_part_of_the_first_key`); the Configuration tab shows it as "starts with a byte-order mark the agent cannot read past; the first line is ignored".

- [ ] **Step 7: Run all core tests, commit**

Run: `cargo test -p guardian-core && cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): render config edits the agent reads back exactly"
```

---

### Task 7: Applying a write plan (portable)

> **Spec note.** R14.3 names `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`. Calling it directly needs `unsafe`, which the workspace forbids. `std::fs::rename` on Windows *is* `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`; durability comes from `File::sync_all` on the temp file before the rename. Record this in `specs/guardian-diag.md` (Task 13). The upside: `apply` is portable and its tests run on Linux CI too.

**Files:**
- Create: `crates/winhost/Cargo.toml`, `crates/winhost/src/lib.rs`, `crates/winhost/src/apply.rs`

**Interfaces:**
- Consumes: `guardian_core::edit::plan::{WritePlan, FileWrite, FileKind, Expected}`, `guardian_core::read::sha256_hex`.
- Produces:
  - `apply::{apply(plan: &WritePlan, io: &dyn Fn(&Path) -> PathBuf, overwrite: bool, now: chrono::DateTime<chrono::Local>) -> Result<Applied, ApplyError>, Applied { written: Vec<Written { kind: FileKind, backup: Option<PathBuf> }> }, ApplyError { kind: FileKind, step: Step, backup: Option<PathBuf>, written_before: Vec<FileKind>, message: String }, Step { Changed, Backup, Write, Replace, Verify }, KEEP_BACKUPS: usize = 5}`
  - Backup paths in `Applied`/`ApplyError` are canonical (the `FileWrite.path`'s directory), not I/O paths.

- [ ] **Step 1: Crate files**

`crates/winhost/Cargo.toml`:
```toml
[package]
name = "guardian-winhost"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
guardian-core = { workspace = true }
chrono = { workspace = true }
ureq = { workspace = true }
rustls-native-certs = { workspace = true }

[target.'cfg(windows)'.dependencies]
windows-service = { workspace = true }
winreg = { workspace = true }
is_elevated = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
guardian-core = { workspace = true, features = ["testkit"] }
```
`src/lib.rs`:
```rust
//! Everything that touches the machine. `apply` is portable; collection is Windows-first
//! with a folder-only fallback elsewhere (for `report --dir` and development).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod apply;
```

- [ ] **Step 2: Failing tests** (`apply.rs`)

```rust
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
        FileWrite { kind, path: dir.join(kind.file_name()), expected: old.map_or(Expected::Absent, |o| Expected::Hash(sha256_hex(o))), new_bytes: new.to_vec(), summary: vec![] }
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
        let mut baks: Vec<String> = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".env.bak-")).collect();
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
```
Run: `cargo test -p guardian-winhost apply` — Expected: compile errors.

- [ ] **Step 3: Implement `apply.rs`**

```rust
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
    let mut baks: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&prefix)).collect();
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
```
Note: `Expected::Absent` with `overwrite: true` and an existing file backs it up — correct.

- [ ] **Step 4: Run, commit**

Run: `cargo test -p guardian-winhost && cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): write config edits with backups and an atomic replace"
```

---

### Task 8: Collecting a snapshot (Windows + folder fallback), health probe, service restart

**Files:**
- Create: `crates/winhost/src/files.rs`, `crates/winhost/src/health.rs`, `crates/winhost/src/collect.rs`, `crates/winhost/src/text.rs`, `crates/winhost/src/win.rs`
- Modify: `crates/winhost/src/lib.rs`

**Interfaces:**
- Consumes: all of `guardian_core` (parsers, `Snapshot`, `paths`).
- Produces:
  - `collect::{CollectOptions { manual_dir: Option<PathBuf>, probe_health: bool }, collect(&CollectOptions) -> Snapshot, io_path(&Path) -> PathBuf}` — `io_path` applies the Sysnative rewrite on WOW64, identity elsewhere; pass it to `apply::apply`.
  - `health::probe(server: &str, timeout: Duration) -> HealthProbe`
  - `text::decode_output(&[u8]) -> String` (UTF-16LE with/without BOM, else lossy UTF-8)
  - `files::{read_capped(&Path) -> Read<(Vec<u8>, Option<DateTime<Utc>>)>, MAX_FILE: u64 = 1_048_576}`
  - Windows only (`#[cfg(windows)] pub mod win`): `win::{is_elevated() -> bool, relaunch_elevated(args: &[String]) -> Result<(), String>, restart_service(progress: &dyn Fn(&str)) -> Result<(), String>}`; on other targets the same three functions exist in `collect` as `elevated()`, `relaunch_elevated`, `restart_service` returning `false` / `Err("only on Windows")`. Re-export them at the crate root: `guardian_winhost::{elevated, relaunch_elevated, restart_service}`.

- [ ] **Step 1: Failing portable tests**

`files.rs` tests:
```rust
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
```
`text.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_output_cases() {
        assert_eq!(decode_output(b"abc"), "abc");
        let utf16: Vec<u8> = "\u{feff}<Event/>".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_output(&utf16), "<Event/>");
        let no_bom: Vec<u8> = "<Event/>".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_output(&no_bom), "<Event/>");
    }
}
```
`health.rs` test with a one-shot local server:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn serve_once(resp: &'static str) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn probe_ok_with_date() {
        let url = serve_once("HTTP/1.1 200 OK\r\nDate: Wed, 14 Oct 2026 10:00:00 GMT\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let h = probe(&url, Duration::from_secs(5));
        assert_eq!(h.status, Some(200));
        assert_eq!(h.server_date.unwrap().to_rfc3339(), "2026-10-14T10:00:00+00:00");
        assert_eq!(h.error, None);
    }

    #[test]
    fn probe_503_and_refused() {
        let url = serve_once("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let h = probe(&url, Duration::from_secs(5));
        assert_eq!(h.status, Some(503));
        let h = probe("http://127.0.0.1:1", Duration::from_secs(2));
        assert_eq!(h.status, None);
        assert!(h.error.is_some());
    }
}
```
`collect.rs` test (portable path — a manual folder):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::snapshot::DirSource;

    #[test]
    fn manual_dir_reads_the_agent_files() {
        let dir = guardian_core::testkit::fixtures_dir().join("healthy");
        let s = collect(&CollectOptions { manual_dir: Some(dir.clone()), probe_health: false });
        assert_eq!(s.install.source, DirSource::Manual);
        assert!(s.install.exists);
        assert_eq!(s.env.as_ok().unwrap().parsed.server_address.as_deref(), Some("https://guardian.example.com"));
        assert_eq!(s.whitelist.as_ok().unwrap().parsed, vec!["notepad.exe", "calc.exe"]);
        assert!(s.sync.as_ok().unwrap().mtime.is_some());
        assert!(s.credentials.as_ok().is_some());
        assert!(s.health.is_none());
    }
}
```
Run: `cargo test -p guardian-winhost` — Expected: compile errors.

- [ ] **Step 2: Implement the portable modules**

`files.rs`:
```rust
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
```
`text.rs`:
```rust
/// Windows tools print UTF-16LE when their output is redirected in some locales; detect it.
pub fn decode_output(b: &[u8]) -> String {
    let utf16 = b.starts_with(&[0xFF, 0xFE]) || (b.len() >= 4 && b.len() % 2 == 0 && b[1] == 0 && b[3] == 0);
    if utf16 {
        let body = b.strip_prefix(&[0xFF, 0xFE][..]).unwrap_or(b);
        let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(b).into_owned()
}
```
(`clippy::indexing_slicing` is not denied in non-test code; `b[1]`/`b[3]` are guarded by the length check.)

`health.rs` (TLS setup copied from PdfCraft `apps/pdfcraft/src/updates.rs`):
```rust
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use guardian_core::snapshot::HealthProbe;

fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err("no trusted root certificates found on this computer".into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// Anonymous `GET {server}/health` (spec R7). Never disables certificate checks.
pub fn probe(server: &str, timeout: Duration) -> HealthProbe {
    let url = format!("{}/health", server.trim_end_matches('/'));
    let mut cfg = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false);
    if url.starts_with("https://") {
        match os_roots() {
            Ok(roots) => cfg = cfg.tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build()),
            Err(e) => return HealthProbe { status: None, latency_ms: 0, server_date: None, error: Some(e) },
        }
    }
    let agent: ureq::Agent = cfg.build().into();
    let start = Instant::now();
    match agent.get(&url).header("User-Agent", concat!("Guardian/", env!("CARGO_PKG_VERSION"))).call() {
        Ok(resp) => {
            let server_date = resp.headers().get("date").and_then(|v| v.to_str().ok()).and_then(|s| DateTime::parse_from_rfc2822(s).ok()).map(|d| d.with_timezone(&Utc));
            let status = resp.status().as_u16();
            HealthProbe { status: Some(status), latency_ms: start.elapsed().as_millis() as u64, server_date, error: (status != 200).then(|| format!("HTTP {status}")) }
        }
        Err(e) => HealthProbe { status: None, latency_ms: start.elapsed().as_millis() as u64, server_date: None, error: Some(e.to_string()) },
    }
}
```
If `ureq` 3.x's builder method names differ from the above (`timeout_global`, `http_status_as_error`, `tls_config`), check `cargo doc -p ureq --open` or context7 and adjust — the PdfCraft call site is the reference for the TLS part.

- [ ] **Step 3: `win.rs` (Windows only)**

```rust
//! Windows-only collection and actions. All via safe APIs (windows-service, winreg,
//! is_elevated, std::process with CREATE_NO_WINDOW); no unsafe in this crate.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use guardian_core::agent;
use guardian_core::read::Read;
use guardian_core::snapshot::{MachineInfo, ServiceInfo, ServiceState, StartType};
use windows_service::service::{ServiceAccess, ServiceStartType, ServiceState as WsState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).output()
}

pub fn wow64() -> bool {
    cfg!(target_pointer_width = "32") && std::env::var_os("PROCESSOR_ARCHITEW6432").is_some()
}

pub fn windir() -> String {
    std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())
}

pub fn io_path(p: &Path) -> PathBuf {
    guardian_core::paths::io_path(p, wow64(), &windir())
}

pub fn image_path() -> Option<String> {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(format!(r"SYSTEM\CurrentControlSet\Services\{}", agent::SERVICE_NAME)).ok()?;
    let v: String = key.get_value("ImagePath").ok()?;
    Some(guardian_core::paths::unquote_image_path(&v))
}

fn win_err<T>(e: windows_service::Error) -> Read<T> {
    if let windows_service::Error::Winapi(io) = &e {
        match io.raw_os_error() {
            Some(1060) => return Read::Missing, // ERROR_SERVICE_DOES_NOT_EXIST
            Some(5) => return Read::Denied,
            _ => {}
        }
    }
    Read::Failed(e.to_string())
}

pub fn service_info() -> Read<ServiceInfo> {
    let mgr = match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) {
        Ok(m) => m,
        Err(e) => return win_err(e),
    };
    let svc = match mgr.open_service(agent::SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG) {
        Ok(s) => s,
        Err(e) => return win_err(e),
    };
    let state = match svc.query_status() {
        Ok(st) => match st.current_state {
            WsState::Running => ServiceState::Running,
            WsState::Stopped => ServiceState::Stopped,
            WsState::StartPending | WsState::StopPending | WsState::ContinuePending | WsState::PausePending => ServiceState::Pending,
            _ => ServiceState::Other,
        },
        Err(e) => return win_err(e),
    };
    let start_type = match svc.query_config() {
        Ok(c) => match c.start_type {
            ServiceStartType::AutoStart => StartType::Auto,
            ServiceStartType::OnDemand => StartType::Manual,
            ServiceStartType::Disabled => StartType::Disabled,
            _ => StartType::Other,
        },
        Err(e) => return win_err(e),
    };
    let image = image_path().unwrap_or_default();
    let exe_exists = !image.is_empty() && io_path(Path::new(&image)).is_file();
    Read::Ok(ServiceInfo { state, start_type, image_path: image, exe_exists })
}

pub fn agent_version(image: &str) -> Read<String> {
    if image.is_empty() {
        return Read::Missing;
    }
    let exe = io_path(Path::new(image));
    match Command::new(&exe).arg("-version").creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) if o.status.success() => Read::Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(o) => Read::Failed(format!("exit {}", o.status)),
        Err(e) => crate::files::from_io(e),
    }
}

pub fn tasklist() -> Read<Vec<String>> {
    match run_hidden("tasklist", &["/FO", "CSV", "/NH"]) {
        Ok(o) if o.status.success() => Read::Ok(guardian_core::parse::tasklist(&crate::text::decode_output(&o.stdout))),
        Ok(o) => Read::Failed(crate::text::decode_output(&o.stderr).trim().to_string()),
        Err(e) => Read::Failed(e.to_string()),
    }
}

pub fn events() -> Read<Vec<guardian_core::parse::AgentEvent>> {
    let q = format!("/q:*[System[Provider[@Name='{}']]]", agent::SERVICE_NAME);
    match run_hidden("wevtutil", &["qe", "Application", &q, "/c:2000", "/rd:true", "/f:xml"]) {
        Ok(o) if o.status.success() => guardian_core::parse::wevtutil(&crate::text::decode_output(&o.stdout)).map_or_else(Read::Failed, Read::Ok),
        Ok(o) => {
            let err = crate::text::decode_output(&o.stderr);
            if err.to_lowercase().contains("access is denied") { Read::Denied } else { Read::Failed(err.trim().to_string()) }
        }
        Err(e) => Read::Failed(e.to_string()),
    }
}

pub fn machine() -> MachineInfo {
    let cv = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok();
    let get = |k: &str| cv.as_ref().and_then(|c| c.get_value::<String, _>(k).ok()).unwrap_or_default();
    let os = format!("{} {} (build {})", get("ProductName"), get("DisplayVersion"), get("CurrentBuild")).trim().to_string();
    let arch = std::env::var("PROCESSOR_ARCHITEW6432").or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE")).unwrap_or_default();
    MachineInfo { hostname: std::env::var("COMPUTERNAME").unwrap_or_default(), os, arch }
}

pub fn is_elevated() -> bool {
    is_elevated::is_elevated()
}

/// Re-launch this exe with UAC (spec R3). Ok(()) once the elevated process has started;
/// Err when the user cancelled UAC or PowerShell is unavailable.
pub fn relaunch_elevated(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let arglist = if args.is_empty() { String::new() } else { format!(" -ArgumentList {}", args.iter().map(|a| q(a)).collect::<Vec<_>>().join(",")) };
    let script = format!("Start-Process -FilePath {}{} -Verb RunAs", q(&exe.to_string_lossy()), arglist);
    match run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]) {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => Err("Elevation was cancelled".into()),
        Err(e) => Err(e.to_string()),
    }
}

fn wait_for(svc: &windows_service::service::Service, want: WsState, limit: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let st = svc.query_status().map_err(|e| e.to_string())?;
        if st.current_state == want {
            return Ok(());
        }
        if start.elapsed() > limit {
            return Err(format!("the service is still {:?} after {} s", st.current_state, limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Stop + start with 30 s per phase (spec R14.5).
pub fn restart_service(progress: &dyn Fn(&str)) -> Result<(), String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
    let svc = mgr.open_service(agent::SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START).map_err(|e| e.to_string())?;
    let limit = Duration::from_secs(30);
    if svc.query_status().map_err(|e| e.to_string())?.current_state != WsState::Stopped {
        progress("Stopping ProcSentinelAgent…");
        svc.stop().map_err(|e| e.to_string())?;
        wait_for(&svc, WsState::Stopped, limit)?;
    }
    progress("Starting ProcSentinelAgent…");
    svc.start::<&str>(&[]).map_err(|e| e.to_string())?;
    wait_for(&svc, WsState::Running, limit)
}
```

- [ ] **Step 4: `collect.rs`**

```rust
use std::path::{Path, PathBuf};

use chrono::Utc;
use guardian_core::agent;
use guardian_core::parse;
use guardian_core::read::Read;
use guardian_core::snapshot::{DirSource, InstallDir, MachineInfo, ServiceInfo, Snapshot};

use crate::files::{read_source, read_source_fallible};

pub struct CollectOptions {
    pub manual_dir: Option<PathBuf>,
    pub probe_health: bool,
}

#[cfg(windows)]
pub fn io_path(p: &Path) -> PathBuf {
    crate::win::io_path(p)
}
#[cfg(not(windows))]
pub fn io_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

#[cfg(windows)]
fn platform() -> (Read<ServiceInfo>, Option<String>, Read<Vec<String>>, Read<Vec<parse::AgentEvent>>, MachineInfo, bool) {
    let svc = crate::win::service_info();
    (svc, crate::win::image_path(), crate::win::tasklist(), crate::win::events(), crate::win::machine(), crate::win::is_elevated())
}
#[cfg(not(windows))]
fn platform() -> (Read<ServiceInfo>, Option<String>, Read<Vec<String>>, Read<Vec<parse::AgentEvent>>, MachineInfo, bool) {
    let na = || "only available on Windows".to_string();
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into());
    (Read::Failed(na()), None, Read::Failed(na()), Read::Failed(na()), MachineInfo { hostname: host, os: std::env::consts::OS.into(), arch: std::env::consts::ARCH.into() }, false)
}

pub fn collect(opts: &CollectOptions) -> Snapshot {
    let (service, image, processes, events, machine, elevated) = platform();
    let (path, source) = match (&opts.manual_dir, &image) {
        (Some(d), _) => (d.clone(), DirSource::Manual),
        (None, Some(img)) if !img.is_empty() => (Path::new(img).parent().map(Path::to_path_buf).unwrap_or_default(), DirSource::Service),
        _ => (PathBuf::from(agent::DEFAULT_INSTALL_DIR), DirSource::Default),
    };
    let exists = io_path(&path).is_dir();
    let f = |name: &str| io_path(&path.join(name));

    #[cfg(windows)]
    let agent_version = crate::win::agent_version(image.as_deref().unwrap_or(""));
    #[cfg(not(windows))]
    let agent_version = Read::Failed("only available on Windows".into());

    let env = read_source(&f(agent::ENV_FILE), parse::env);
    let credentials = match crate::files::read_capped(&f(agent::CREDENTIALS_FILE)) {
        Read::Ok((b, _)) => parse::credentials(&b).map_or_else(Read::Failed, Read::Ok),
        Read::Missing => Read::Missing,
        Read::Denied => Read::Denied,
        Read::Failed(e) => Read::Failed(e),
    };
    let sync = read_source_fallible(&f(agent::SYNC_FILE), parse::sync);
    let whitelist = read_source(&f(agent::WHITELIST_FILE), parse::whitelist);

    let server = env.as_ok().and_then(|e| e.parsed.server_address.clone());
    let health = match (opts.probe_health, server) {
        (true, Some(s)) => Some(Read::Ok(crate::health::probe(&s, std::time::Duration::from_secs(5)))),
        _ => None,
    };

    Snapshot { taken_at: Utc::now(), machine, install: InstallDir { path, source, exists }, elevated, service, agent_version, env, credentials, sync, whitelist, processes, events, health }
}

#[cfg(not(windows))]
pub fn elevated() -> bool {
    false
}
#[cfg(not(windows))]
pub fn relaunch_elevated(_args: &[String]) -> Result<(), String> {
    Err("only available on Windows".into())
}
#[cfg(not(windows))]
pub fn restart_service(_progress: &dyn Fn(&str)) -> Result<(), String> {
    Err("only available on Windows".into())
}
#[cfg(windows)]
pub use crate::win::{is_elevated as elevated, relaunch_elevated, restart_service};
```
`lib.rs`:
```rust
pub mod apply;
pub mod collect;
pub mod files;
pub mod health;
pub mod text;
#[cfg(windows)]
pub mod win;

pub use collect::{CollectOptions, collect, elevated, io_path, relaunch_elevated, restart_service};
```

- [ ] **Step 5: Run tests (Linux/macOS), check the Windows build compiles**

Run: `cargo test -p guardian-winhost && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.
Run: `rustup target add x86_64-pc-windows-gnu && cargo check -p guardian-winhost --target x86_64-pc-windows-gnu` (a type-check of `win.rs` without a Windows box; linking is not needed). Expected: no errors. If a `windows-service`/`winreg` signature differs, fix against `cargo doc`.

- [ ] **Step 6: Commit**

```bash
git add tools/guardian-diag
git commit -m "feat(diag): collect a snapshot of the machine and restart the service"
```

---

### Task 9: `Guardian.exe` binary, `report` CLI, manifest and icon

**Files:**
- Create: `apps/guardian/Cargo.toml`, `apps/guardian/build.rs`, `apps/guardian/guardian.manifest`, `apps/guardian/src/main.rs`, `apps/guardian/src/args.rs`
- Test: `apps/guardian/tests/report_cli.rs`, unit tests in `args.rs`

**Interfaces:**
- Consumes: `guardian_winhost::{collect, CollectOptions}`, `guardian_core::{diagnose::diagnose, report::{report, exit_code}}`.
- Produces: `args::{Command { Window { tab: Option<String>, dir: Option<PathBuf> }, Report { json: bool, dir: Option<PathBuf>, offline: bool } }, parse(&[String]) -> Result<Command, String>}`; `guardian_ui::run(tab: Option<String>, dir: Option<PathBuf>) -> Result<(), String>` is called for `Window` (added in Task 10; until then `main` prints "window not built yet" and exits 2 — replaced in Task 10).

- [ ] **Step 1: Failing arg tests** (`args.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_cases() {
        assert_eq!(parse(&a(&[])), Ok(Command::Window { tab: None, dir: None }));
        assert_eq!(parse(&a(&["--tab", "configuration"])), Ok(Command::Window { tab: Some("configuration".into()), dir: None }));
        assert_eq!(parse(&a(&["report", "--json", "--dir", "X", "--offline"])), Ok(Command::Report { json: true, dir: Some("X".into()), offline: true }));
        assert!(parse(&a(&["report", "--bogus"])).is_err());
        assert!(parse(&a(&["--tab"])).is_err());
        assert!(parse(&a(&["--tab", "nope"])).is_err());
    }
}
```

- [ ] **Step 2: Implement `args.rs`**

```rust
use std::path::PathBuf;

pub const TABS: [&str; 4] = ["diagnostics", "configuration", "processes", "log"];
pub const USAGE: &str = "Usage:\n  Guardian.exe [--tab diagnostics|configuration|processes|log] [--dir <agent folder>]\n  Guardian.exe report [--json] [--dir <agent folder>] [--offline]\n\nExit codes of report: 0 protection works, 10 warnings, 11 errors, 2 usage error.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Window { tab: Option<String>, dir: Option<PathBuf> },
    Report { json: bool, dir: Option<PathBuf>, offline: bool },
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter().peekable();
    let report = it.peek().is_some_and(|a| *a == "report");
    if report {
        it.next();
    }
    let (mut tab, mut dir, mut json, mut offline) = (None, None, false, false);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(it.next().ok_or("--dir needs a folder")?)),
            "--tab" if !report => {
                let t = it.next().ok_or("--tab needs a name")?.to_lowercase();
                if !TABS.contains(&t.as_str()) {
                    return Err(format!("unknown tab {t}"));
                }
                tab = Some(t);
            }
            "--json" if report => json = true,
            "--offline" if report => offline = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(if report { Command::Report { json, dir, offline } } else { Command::Window { tab, dir } })
}
```

- [ ] **Step 3: `main.rs`, `Cargo.toml`, `build.rs`, manifest**

`apps/guardian/Cargo.toml`:
```toml
[package]
name = "guardian"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true

[[bin]]
name = "Guardian"
path = "src/main.rs"

[dependencies]
guardian-core = { workspace = true }
guardian-winhost = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
rfd = { workspace = true }

[build-dependencies]
winresource = "0.1"
```
`src/main.rs`:
```rust
//! Guardian.exe: the window, or `report` for the same diagnosis without one.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// A double-click must not open a console window.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod args;

use std::io::Write;

use guardian_core::diagnose::diagnose;
use guardian_core::report::{exit_code, report};
use guardian_winhost::{CollectOptions, collect};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match args::parse(&argv) {
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "{e}\n\n{}", args::USAGE);
            2
        }
        Ok(args::Command::Report { json, dir, offline }) => run_report(json, dir, offline),
        Ok(args::Command::Window { tab: _, dir: _ }) => {
            let _ = writeln!(std::io::stderr(), "the window is not built yet");
            2
        }
    };
    std::process::exit(code);
}

fn run_report(json: bool, dir: Option<std::path::PathBuf>, offline: bool) -> i32 {
    let s = collect(&CollectOptions { manual_dir: dir, probe_health: !offline });
    let d = diagnose(&s);
    let r = report(&s, &d);
    let body = if json { serde_json::to_string_pretty(&r.json).unwrap_or_default() } else { r.text.clone() };
    // A GUI-subsystem exe has no console of its own: output reaches a redirect or pipe
    // (`Guardian.exe report > report.txt`), but not a bare console. If stdout is
    // unusable, save the report next to the exe instead and say where (spec R25).
    let mut out = std::io::stdout();
    if out.write_all(body.as_bytes()).and_then(|_| out.flush()).is_err() {
        let msg = match save_report_next_to_exe(&s.machine.hostname, &r) {
            Ok(p) => format!("Saved the report to {}", p.display()),
            Err(e) => format!("Could not print or save the report: {e}"),
        };
        rfd::MessageDialog::new().set_title("Guardian report").set_description(msg).show();
    }
    exit_code(&d)
}

fn save_report_next_to_exe(host: &str, r: &guardian_core::report::Report) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no exe folder")?.join("GuardianReports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = format!("{host}-{}", chrono::Local::now().format("%Y%m%d-%H%M"));
    std::fs::write(dir.join(format!("{stem}.txt")), &r.text).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_string_pretty(&r.json).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{stem}.txt")))
}
```
(Task 12 moves `save_report_next_to_exe` into `guardian_core::report::file_stem` + `guardian_winhost::save_report` so the window's Save report shares it — do it then, not now.)

`guardian.manifest`:
```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Guardian" version="0.1.0.0"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
</assembly>
```
`build.rs`:
```rust
//! Windows only: icon, VERSIONINFO and the asInvoker manifest. A missing resource compiler is a
//! warning (cross-compiles still link) unless GUARDIAN_REQUIRE_WINRES=1 (CI release builds).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=guardian.manifest");
    println!("cargo:rerun-if-changed=../../../whitelist-gui/guardian.ico");
    println!("cargo:rerun-if-env-changed=GUARDIAN_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../../whitelist-gui/guardian.ico")
        .set_manifest_file("guardian.manifest")
        .set("ProductName", "Guardian")
        .set("FileDescription", "Guardian — agent diagnostics and settings")
        .set("OriginalFilename", "Guardian.exe")
        .set("InternalName", "Guardian");
    if let Err(e) = res.compile() {
        if std::env::var_os("GUARDIAN_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources failed: {e}");
            return;
        }
        println!("cargo:warning=Guardian.exe built without icon/manifest: {e}");
    }
}
```

- [ ] **Step 4: CLI smoke test** (`tests/report_cli.rs`)

```rust
//! `Guardian.exe report` against a fixture folder (spec R25, R32).

use std::process::Command;

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/core/tests/fixtures/healthy")
}

#[test]
fn report_text_and_exit_code() {
    let out = Command::new(env!("CARGO_BIN_EXE_Guardian")).args(["report", "--offline", "--dir"]).arg(fixture()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("VERDICT:"), "{text}");
    assert!(!text.contains("agt_secret_value_123"));
    // Off Windows the service is unreadable → "could not check" (Unknown), never 0 with errors hidden.
    let code = out.status.code().unwrap();
    assert!([0, 10, 11].contains(&code), "{code}");
}

#[test]
fn report_json_parses() {
    let out = Command::new(env!("CARGO_BIN_EXE_Guardian")).args(["report", "--json", "--offline", "--dir"]).arg(fixture()).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["config"]["server_address"], "https://guardian.example.com");
}

#[test]
fn usage_error_is_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_Guardian")).args(["report", "--nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
```
Run: `cargo test -p guardian` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/guardian-diag
git commit -m "feat(diag): Guardian.exe with the report command"
```

---

### Task 10: Window shell, theme, background worker, Diagnostics tab

**Files:**
- Create: `crates/ui-egui/Cargo.toml`, `src/lib.rs`, `src/theme.rs`, `src/state.rs`, `src/worker.rs`, `src/tabs/mod.rs`, `src/tabs/diagnostics.rs`
- Modify: `apps/guardian/Cargo.toml` (add `guardian-ui`, `eframe`), `apps/guardian/src/main.rs` (call `guardian_ui::run`), `.github/workflows/diag.yml` (apt packages for eframe)
- Test: `crates/ui-egui/tests/shell.rs` (egui_kittest), unit tests in `state.rs`

**Interfaces:**
- Consumes: `guardian_winhost::{collect, CollectOptions, elevated, relaunch_elevated}`, `guardian_core::diagnose`.
- Produces:
  - `guardian_ui::run(tab: Option<String>, dir: Option<PathBuf>) -> Result<(), String>`
  - `guardian_ui::GuardianApp::with_snapshot(snapshot: Snapshot, tab: Tab) -> GuardianApp` (for tests: no worker, no I/O)
  - `state::{Tab { Diagnostics, Configuration, Processes, Log }, Tab::from_arg(&str) -> Option<Tab>, Tab::arg(self) -> &'static str}`
  - `worker::{Job { Collect(CollectOptions), Apply { plan: WritePlan, overwrite: bool }, Restart, SaveReport }, Done { Collected(Box<Snapshot>), Applied(Result<Applied, ApplyError>), Restarted(Result<(), String>), ReportSaved(Result<PathBuf, String>) }, Worker { fn spawn(job: Job, ctx: egui::Context) -> Self, fn poll(&self) -> Option<Done>, fn progress(&self) -> Option<String> }}`
  - `theme::{install(ctx), Palette { ok, warn, error, muted, bg, panel, text }, palette() -> Palette, mono(text) -> egui::RichText, heading(text) -> egui::RichText}`

- [ ] **Step 1: Crate setup**

`crates/ui-egui/Cargo.toml`:
```toml
[package]
name = "guardian-ui"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
guardian-core = { workspace = true }
guardian-winhost = { workspace = true }
egui = { workspace = true }
eframe = { workspace = true }
egui_extras = { workspace = true }
rfd = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
egui_kittest = { workspace = true, features = ["eframe"] }
guardian-core = { workspace = true, features = ["testkit"] }
```
If `egui_kittest`'s feature for `build_eframe` is named differently, copy the line from `~/Workspace/bucket/pdfcraft/crates/ui-egui/Cargo.toml`.

CI: in `diag.yml` before clippy add
```yaml
      - run: sudo apt-get update && sudo apt-get install -y libxkbcommon-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgtk-3-dev
```
(`libgtk-3-dev` is for `rfd` on Linux.)

- [ ] **Step 2: `theme.rs`** — the cabinet's palette, values copied from `server/webui/app.css` `:root` (`--void`, `--panel`, `--ink`, `--muted`, `--guardian`, `--amber`, `--danger`). If `app.css` changes, change these with it.

```rust
use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, RichText};

pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
}

/// Values copied from server/webui/app.css (dark only; colour carries state and nothing else).
pub fn palette() -> Palette {
    Palette {
        bg: Color32::from_rgb(0x07, 0x09, 0x0a),    // --void
        panel: Color32::from_rgb(0x0e, 0x14, 0x12), // --panel
        text: Color32::from_rgb(0xe6, 0xf0, 0xea),  // --ink
        muted: Color32::from_rgb(0x8f, 0xa3, 0x9a), // --muted
        ok: Color32::from_rgb(0x4a, 0xde, 0x80),    // --guardian
        warn: Color32::from_rgb(0xf5, 0xc4, 0x51),  // --amber
        error: Color32::from_rgb(0xff, 0x5c, 0x6c), // --danger
    }
}

pub fn font_definitions() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("Inter", include_bytes!("../../../../../mobile/assets/fonts/Inter-Regular.ttf"));
    add("Inter-SemiBold", include_bytes!("../../../../../mobile/assets/fonts/Inter-SemiBold.ttf"));
    add("SpaceGrotesk", include_bytes!("../../../../../mobile/assets/fonts/SpaceGrotesk-Medium.ttf"));
    add("JetBrainsMono", include_bytes!("../../../../../mobile/assets/fonts/JetBrainsMono-Regular.ttf"));
    f.families.entry(FontFamily::Proportional).or_default().insert(0, "Inter".to_owned());
    f.families.entry(FontFamily::Monospace).or_default().insert(0, "JetBrainsMono".to_owned());
    f.families.insert(FontFamily::Name("heading".into()), vec!["SpaceGrotesk".into(), "Inter".into()]);
    f
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    let p = palette();
    let mut v = egui::Visuals::dark();
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.panel;
    v.override_text_color = Some(p.text);
    ctx.set_visuals(v);
}

pub fn mono(t: impl Into<String>) -> RichText {
    RichText::new(t).family(FontFamily::Monospace)
}

pub fn heading(t: impl Into<String>) -> RichText {
    RichText::new(t).family(FontFamily::Name("heading".into())).size(20.0)
}

pub fn status_color(s: guardian_core::diagnose::Status) -> Color32 {
    use guardian_core::diagnose::Status;
    let p = palette();
    match s {
        Status::Ok => p.ok,
        Status::Warn => p.warn,
        Status::Error => p.error,
        Status::Unknown => p.muted,
    }
}
```
(Named families need one frame before first use, as PdfCraft notes — `GuardianApp` keeps a `fonts_ready` flag and skips drawing on the first frame.)

- [ ] **Step 3: `state.rs` with tests** — pure state shared by the tabs

```rust
use guardian_core::edit::{EnvEdit, WhitelistEdit};
use guardian_core::snapshot::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Diagnostics,
    Configuration,
    Processes,
    Log,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Diagnostics, Tab::Configuration, Tab::Processes, Tab::Log];
    pub fn from_arg(s: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.arg() == s)
    }
    pub fn arg(self) -> &'static str {
        match self {
            Tab::Diagnostics => "diagnostics",
            Tab::Configuration => "configuration",
            Tab::Processes => "processes",
            Tab::Log => "log",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Tab::Diagnostics => "Diagnostics",
            Tab::Configuration => "Configuration",
            Tab::Processes => "Processes",
            Tab::Log => "Log",
        }
    }
}

/// The edit draft (spec R14): built from a snapshot, compared against it for the dirty count.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub env: EnvEdit,
    pub whitelist: WhitelistEdit,
    /// The text in the BINDING_TOKEN password field; folded into env.binding_token on Save.
    pub token_field: String,
}

impl Draft {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        Draft { env: EnvEdit::from_snapshot(s), whitelist: WhitelistEdit::from_snapshot(s), token_field: String::new() }
    }
    /// How many things differ from what is on disk (for "N unsaved changes").
    pub fn dirty_count(&self, s: &Snapshot) -> usize {
        let base = Draft::from_snapshot(s);
        let env = guardian_core::edit::plan::env_changes(s, &self.effective_env()).0.len();
        let added = self.whitelist.entries.iter().filter(|e| !base.whitelist.entries.contains(e)).count();
        let removed = base.whitelist.entries.iter().filter(|e| !self.whitelist.entries.contains(e)).count();
        env + added + removed
    }
    /// env with the password field applied: non-empty field = Set, otherwise whatever Remove/Keep says.
    pub fn effective_env(&self) -> EnvEdit {
        let mut e = self.env.clone();
        if !self.token_field.is_empty() {
            e.binding_token = guardian_core::edit::TokenEdit::Set(self.token_field.clone());
        }
        e
    }
}

/// Spec R3/R5: who may edit, and why not.
pub fn edit_lock(s: &Snapshot) -> Option<&'static str> {
    if !s.elevated && !s.is_manual_dir() {
        Some("Run as administrator to edit")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }

    #[test]
    fn tabs_roundtrip() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_arg(t.arg()), Some(t));
        }
    }

    #[test]
    fn dirty_count_counts_env_and_whitelist() {
        let s = snap();
        let mut d = Draft::from_snapshot(&s);
        assert_eq!(d.dirty_count(&s), 0);
        d.env.check_interval = "15".into();
        d.whitelist.entries.push("winword.exe".into());
        d.whitelist.entries.retain(|e| e != "calc.exe");
        d.token_field = "tok".into();
        assert_eq!(d.dirty_count(&s), 4);
    }

    #[test]
    fn edit_locked_when_not_elevated() {
        let mut s = snap();
        s.elevated = false;
        assert_eq!(edit_lock(&s), Some("Run as administrator to edit"));
        s.elevated = true;
        assert_eq!(edit_lock(&s), None);
    }
}
```
Note on a manual folder (R5): editing is allowed without elevation when the folder isn't the live install (the write will fail with "no access" if it is protected — that error path is already covered by `apply`).

- [ ] **Step 4: `worker.rs`**

```rust
//! One background job at a time; the window polls it every frame and never blocks (spec R26).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use guardian_core::edit::plan::WritePlan;
use guardian_core::snapshot::Snapshot;
use guardian_winhost::CollectOptions;
use guardian_winhost::apply::{ApplyError, Applied};

pub enum Job {
    Collect(CollectOptions),
    Apply { plan: WritePlan, overwrite: bool },
    Restart,
    SaveReport(Box<Snapshot>),
}

pub enum Done {
    Collected(Box<Snapshot>),
    Applied(Result<Applied, ApplyError>),
    Restarted(Result<(), String>),
    ReportSaved(Result<PathBuf, String>),
}

pub struct Worker {
    rx: Receiver<Done>,
    progress: Arc<Mutex<Option<String>>>,
}

impl Worker {
    pub fn spawn(job: Job, ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let progress = Arc::new(Mutex::new(Some(match &job {
            Job::Collect(_) => "Reading service, files, log…".to_string(),
            Job::Apply { .. } => "Saving…".into(),
            Job::Restart => "Restarting ProcSentinelAgent…".into(),
            Job::SaveReport(_) => "Saving report…".into(),
        })));
        let p = Arc::clone(&progress);
        std::thread::spawn(move || {
            let set = |m: &str| {
                if let Ok(mut g) = p.lock() {
                    *g = Some(m.to_string());
                }
                ctx.request_repaint();
            };
            let done = match job {
                Job::Collect(o) => Done::Collected(Box::new(guardian_winhost::collect(&o))),
                Job::Apply { plan, overwrite } => Done::Applied(guardian_winhost::apply::apply(&plan, &guardian_winhost::io_path, overwrite, chrono::Local::now())),
                Job::Restart => Done::Restarted(guardian_winhost::restart_service(&set)),
                Job::SaveReport(s) => Done::ReportSaved(guardian_winhost::save_report(&s)),
            };
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        Worker { rx, progress }
    }

    pub fn poll(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }

    pub fn progress(&self) -> Option<String> {
        self.progress.lock().ok().and_then(|g| g.clone())
    }
}
```
`guardian_winhost::save_report` is added in Task 12; until then make `Job::SaveReport` return `Err("not yet".into())` **only within this task's commit** and replace it in Task 12. (Alternatively, do Task 12 Step 1 first — reviewer's choice; both end in the same code.)

- [ ] **Step 5: `lib.rs` — `GuardianApp`, top bar, tabs, Diagnostics**

```rust
//! Guardian's window: four tabs over a Snapshot (spec R6–R23).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod state;
pub mod tabs;
pub mod theme;
pub mod worker;

use std::path::PathBuf;

use guardian_core::diagnose::{Diagnosis, diagnose};
use guardian_core::snapshot::Snapshot;
use guardian_winhost::CollectOptions;

use state::{Draft, Tab};
use worker::{Done, Job, Worker};

pub struct GuardianApp {
    pub tab: Tab,
    pub snapshot: Option<Snapshot>,
    pub diagnosis: Option<Diagnosis>,
    pub draft: Option<Draft>,
    pub manual_dir: Option<PathBuf>,
    pub worker: Option<Worker>,
    /// A message shown in the top bar until the next action (errors and confirmations).
    pub notice: Option<(guardian_core::diagnose::Status, String)>,
    pub dialog: Option<tabs::configuration::Dialog>,
    pub why_query: String,
    pub process_filter: String,
    pub only_closing: bool,
    pub log_filter: tabs::log::Filter,
    pub log_period: tabs::log::Period,
    pub log_search: String,
    pub kill_summary: bool,
    pub new_entry: String,
    fonts_ready: bool,
}

impl GuardianApp {
    pub fn new(cc: &eframe::CreationContext<'_>, tab: Option<String>, dir: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self::empty(tab.as_deref().and_then(Tab::from_arg).unwrap_or(Tab::Diagnostics));
        app.manual_dir = dir;
        app.refresh(&cc.egui_ctx);
        app
    }

    fn empty(tab: Tab) -> Self {
        GuardianApp {
            tab,
            snapshot: None,
            diagnosis: None,
            draft: None,
            manual_dir: None,
            worker: None,
            notice: None,
            dialog: None,
            why_query: String::new(),
            process_filter: String::new(),
            only_closing: false,
            log_filter: tabs::log::Filter::All,
            log_period: tabs::log::Period::All,
            log_search: String::new(),
            kill_summary: false,
            new_entry: String::new(),
            fonts_ready: false,
        }
    }

    /// Tests: a ready window over a fixed snapshot, no worker, no I/O.
    pub fn with_snapshot(s: Snapshot, tab: Tab) -> Self {
        let mut app = Self::empty(tab);
        app.set_snapshot(s);
        app
    }

    pub fn set_snapshot(&mut self, s: Snapshot) {
        self.diagnosis = Some(diagnose(&s));
        self.draft = Some(Draft::from_snapshot(&s));
        self.snapshot = Some(s);
    }

    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.worker = Some(Worker::spawn(Job::Collect(CollectOptions { manual_dir: self.manual_dir.clone(), probe_health: true }), ctx.clone()));
    }

    pub fn dirty(&self) -> usize {
        match (&self.draft, &self.snapshot) {
            (Some(d), Some(s)) => d.dirty_count(s),
            _ => 0,
        }
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let Some(done) = self.worker.as_ref().and_then(Worker::poll) else { return };
        self.worker = None;
        match done {
            Done::Collected(s) => self.set_snapshot(*s),
            other => tabs::configuration::on_done(self, other, ctx),
        }
    }
}

impl eframe::App for GuardianApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            self.fonts_ready = true;
            ctx.request_repaint();
            return;
        }
        self.poll_worker(&ctx);
        tabs::top_bar(self, ui);
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(_) = &self.snapshot else {
                ui.centered_and_justified(|ui| ui.label(self.worker.as_ref().and_then(Worker::progress).unwrap_or_default()));
                return;
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Diagnostics => tabs::diagnostics::show(self, ui),
                Tab::Configuration => tabs::configuration::show(self, ui),
                Tab::Processes => tabs::processes::show(self, ui),
                Tab::Log => tabs::log::show(self, ui),
            });
        });
        tabs::configuration::dialogs(self, &ctx);
    }
}

pub fn run(tab: Option<String>, dir: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default().with_title("Guardian").with_inner_size([1100.0, 720.0]).with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native("Guardian", options, Box::new(move |cc| Ok(Box::new(GuardianApp::new(cc, tab, dir))))).map_err(|e| e.to_string())
}
```
In `run`'s caller (`apps/guardian/src/main.rs`), on `Err(e)` show `rfd::MessageDialog` with: `"Guardian could not open its window ({e}). Run: Guardian.exe report > report.txt"` (R30) and exit 1.

`tabs/mod.rs` — top bar (verdict line, Refresh, admin button, tab strip, notice, progress):
```rust
pub mod configuration;
pub mod diagnostics;
pub mod log;
pub mod processes;

use crate::state::{Tab, edit_lock};
use crate::theme;
use crate::{GuardianApp, worker::Worker};

pub fn top_bar(app: &mut GuardianApp, ui: &mut egui::Ui) {
    egui::Panel::top("top").show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::heading("Guardian"));
            if let (Some(s), Some(d)) = (&app.snapshot, &app.diagnosis) {
                ui.label(theme::mono(&s.machine.hostname));
                // The Diagnostics tab shows the verdict large; elsewhere it rides in the bar.
                if app.tab != Tab::Diagnostics {
                    ui.separator();
                    ui.colored_label(theme::status_color(d.worst_for_verdict()), &d.verdict.text);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = app.busy();
                if ui.add_enabled(!busy, egui::Button::new("Refresh")).clicked() {
                    if app.dirty() > 0 {
                        app.dialog = Some(configuration::Dialog::DiscardOnRefresh);
                    } else {
                        app.refresh(ui.ctx());
                    }
                }
                if let Some(s) = &app.snapshot {
                    if edit_lock(s).is_some() && ui.add_enabled(!busy, egui::Button::new("Restart as administrator")).clicked() {
                        let mut args = vec!["--tab".to_string(), app.tab.arg().to_string()];
                        if let Some(d) = &app.manual_dir {
                            args.extend(["--dir".to_string(), d.display().to_string()]);
                        }
                        match guardian_winhost::relaunch_elevated(&args) {
                            Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                            Err(e) => app.notice = Some((guardian_core::diagnose::Status::Warn, e)),
                        }
                    }
                }
                if let Some(p) = app.worker.as_ref().and_then(Worker::progress) {
                    ui.spinner();
                    ui.label(p);
                }
            });
        });
        ui.horizontal(|ui| {
            for t in Tab::ALL {
                let label = if t == Tab::Configuration && app.dirty() > 0 { format!("{} •", t.title()) } else { t.title().to_string() };
                ui.selectable_value(&mut app.tab, t, label);
            }
        });
        if let Some((st, msg)) = &app.notice {
            ui.colored_label(theme::status_color(*st), msg);
        }
        ui.add_space(4.0);
    });
}
```
Add to `Diagnosis` in core (Task 3 file) a small helper the UI needs:
```rust
impl Diagnosis {
    /// Colour of the verdict line: green only for On.
    pub fn worst_for_verdict(&self) -> Status {
        match self.verdict.kind {
            VerdictKind::On => Status::Ok,
            VerdictKind::Stale | VerdictKind::Free | VerdictKind::Unknown => Status::Warn,
            _ => Status::Error,
        }
    }
}
```

`tabs/diagnostics.rs`:
```rust
use guardian_core::diagnose::Status;

use crate::GuardianApp;
use crate::theme;

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let (Some(s), Some(d)) = (&app.snapshot, &app.diagnosis) else { return };
    ui.add_space(8.0);
    ui.label(theme::heading(&d.verdict.text).color(theme::status_color(d.worst_for_verdict())).size(26.0));
    if !d.verdict.hint.is_empty() {
        ui.label(&d.verdict.hint);
    }
    if guardian_core::enforce::looks_locked(s) {
        ui.colored_label(theme::palette().warn, "Looks like this computer is locked in the cabinet (or the room allows nothing).");
    }
    if s.is_manual_dir() {
        ui.label(format!("Files from {} (chosen manually); service and log are this computer's.", s.install.path.display()));
    }
    ui.add_space(12.0);
    egui::Grid::new("checks").num_columns(3).spacing([16.0, 8.0]).striped(true).show(ui, |ui| {
        for c in &d.checks {
            let mark = match c.status {
                Status::Ok => "●  ok",
                Status::Warn => "●  warning",
                Status::Error => "●  error",
                Status::Unknown => "○  not checked",
            };
            ui.colored_label(theme::status_color(c.status), mark);
            ui.label(egui::RichText::new(c.title).strong());
            ui.vertical(|ui| {
                ui.label(theme::mono(&c.fact));
                if c.status != Status::Ok && !c.todo.is_empty() {
                    ui.label(egui::RichText::new(&c.todo).color(theme::palette().muted));
                }
            });
            ui.end_row();
        }
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Choose agent folder…").clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                app.manual_dir = Some(dir);
                app.refresh(ui.ctx());
            }
        }
        if ui.add_enabled(!app.busy(), egui::Button::new("Save report")).clicked() {
            if let Some(s) = app.snapshot.clone() {
                app.worker = Some(crate::worker::Worker::spawn(crate::worker::Job::SaveReport(Box::new(s)), ui.ctx().clone()));
            }
        }
    });
}
```
Create `tabs/configuration.rs`, `tabs/processes.rs`, `tabs/log.rs` as stubs with `pub fn show(_: &mut GuardianApp, ui: &mut egui::Ui) { ui.label("…"); }`, `pub enum Dialog { DiscardOnRefresh }`, `pub fn dialogs(..) {}`, `pub fn on_done(..) {}`, and `log::{Filter, Period}` enums with `All` variants so the shell compiles; Tasks 11–12 fill them.

- [ ] **Step 6: kittest shell test** (`tests/shell.rs`)

```rust
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};
use guardian_ui::GuardianApp;
use guardian_ui::state::Tab;

fn snap() -> guardian_core::snapshot::Snapshot {
    snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
}

#[test]
fn diagnostics_tab_shows_verdict_and_checks() {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 720.0)).build_eframe(|cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        GuardianApp::with_snapshot(snap(), Tab::Diagnostics)
    });
    h.run_steps(2);
    h.get_by_label_contains("Protection is on");
    h.get_by_label("Last sync");
    h.get_by_label("Server refusal");
}
```
Run: `cargo test -p guardian-ui` — Expected: PASS (state tests + kittest).

- [ ] **Step 7: Wire `main.rs` to the window, run on the dev machine, commit**

In `apps/guardian/src/main.rs` replace the `Window` arm with:
```rust
Ok(args::Command::Window { tab, dir }) => match guardian_ui::run(tab, dir) {
    Ok(()) => 0,
    Err(e) => {
        rfd::MessageDialog::new().set_title("Guardian").set_description(format!("Guardian could not open its window ({e}). Run: Guardian.exe report > report.txt")).show();
        1
    }
},
```
Add `guardian-ui = { workspace = true }` to `apps/guardian/Cargo.toml`.
Run: `cargo run -p guardian -- --dir crates/core/tests/fixtures/healthy` — Expected: a window with the verdict and the check list (on macOS/Linux most checks are "not checked"). Close it.
```bash
git add tools/guardian-diag .github/workflows/diag.yml
git commit -m "feat(diag): window shell and the Diagnostics tab"
```

---

### Task 11: Configuration tab — view, edit, Save, restart

**Files:**
- Modify: `crates/ui-egui/src/tabs/configuration.rs`
- Test: `crates/ui-egui/tests/configuration.rs` (kittest), unit tests for `save_flow` helpers in `configuration.rs`

**Interfaces:**
- Consumes: `state::{Draft, edit_lock}`, `guardian_core::edit::{validate, check_new_entry, TokenEdit, Severity, Field, Issue, plan::{plan, WritePlan}}`, `worker::{Job, Done}`, `guardian_core::agent::{PROTECTED, is_protected}`.
- Produces:
  - `configuration::Dialog { DiscardOnRefresh, Confirm { plan: WritePlan, warnings: Vec<Issue> }, ChangedOnDisk { plan: WritePlan }, Restart, Error(String) }`
  - `configuration::{show, dialogs, on_done, begin_save(app: &mut GuardianApp)}`
  - `configuration::add_entry(app: &mut GuardianApp, raw: &str) -> Result<Vec<Issue>, Issue>` — used by Processes and Log tabs ("Add to local whitelist").

- [ ] **Step 1: Failing kittest** (`tests/configuration.rs`)

```rust
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};
use guardian_ui::GuardianApp;
use guardian_ui::state::Tab;
use guardian_ui::tabs::configuration::Dialog;

fn snap(elevated: bool) -> guardian_core::snapshot::Snapshot {
    let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
    s.elevated = elevated;
    s
}

fn harness(elevated: bool) -> Harness<'static, GuardianApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_eframe(move |cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        GuardianApp::with_snapshot(snap(elevated), Tab::Configuration)
    });
    h.run_steps(2);
    h
}

#[test]
fn read_only_without_admin() {
    let h = harness(false);
    h.get_by_label("Run as administrator to edit");
    assert!(h.query_by_label("Save").is_none());
    h.get_by_label("notepad.exe");
}

#[test]
fn add_entry_and_save_opens_confirm() {
    let mut h = harness(true);
    assert_eq!(guardian_ui::tabs::configuration::add_entry(h.state_mut(), "\"C:\\Office\\WINWORD.EXE\"").unwrap(), vec![]);
    h.run_steps(2);
    h.get_by_label("1 unsaved change");
    h.get_by_label("Save").click();
    h.run_steps(2);
    match &h.state().dialog {
        Some(Dialog::Confirm { plan, warnings }) => {
            assert_eq!(plan.files.len(), 1);
            assert_eq!(plan.files[0].summary, vec!["+ winword.exe".to_string()]);
            assert!(warnings.is_empty());
        }
        _ => panic!("expected the confirm dialog"),
    }
}

#[test]
fn token_field_never_echoes_and_warns_when_enrolled() {
    let mut h = harness(true);
    h.state_mut().draft.as_mut().unwrap().token_field = "bt_typed_secret".into();
    h.run_steps(2);
    assert!(h.query_by_label_contains("bt_typed_secret").is_none());
    guardian_ui::tabs::configuration::begin_save(h.state_mut());
    match &h.state().dialog {
        Some(Dialog::Confirm { warnings, plan }) => {
            assert!(warnings.iter().any(|w| w.message.contains("Unenrol it in the cabinet first")));
            assert_eq!(plan.files[0].summary, vec!["BINDING_TOKEN: set".to_string()]);
        }
        _ => panic!("expected the confirm dialog"),
    }
}

#[test]
fn invalid_server_address_disables_save() {
    let mut h = harness(true);
    h.state_mut().draft.as_mut().unwrap().env.server_address = "guardian.example.com".into();
    h.run_steps(2);
    h.get_by_label_contains("must start with http:// or https://");
    guardian_ui::tabs::configuration::begin_save(h.state_mut());
    assert!(h.state().dialog.is_none());
}
```
Run: `cargo test -p guardian-ui --test configuration` — Expected: FAIL (stubs).

- [ ] **Step 2: Implement `configuration.rs`**

```rust
//! Configuration tab (spec R9–R15): view everything, edit .env and whitelist.txt as admin.

use guardian_core::agent;
use guardian_core::diagnose::Status;
use guardian_core::edit::plan::{WritePlan, plan};
use guardian_core::edit::{Field, Issue, Severity, TokenEdit, check_new_entry, validate};
use guardian_core::read::Read;

use crate::state::edit_lock;
use crate::worker::{Done, Job, Worker};
use crate::{GuardianApp, theme};

pub enum Dialog {
    DiscardOnRefresh,
    Confirm { plan: WritePlan, warnings: Vec<Issue> },
    ChangedOnDisk { plan: WritePlan },
    Restart,
    Error(String),
}

pub fn add_entry(app: &mut GuardianApp, raw: &str) -> Result<Vec<Issue>, Issue> {
    let Some(d) = app.draft.as_mut() else { return Ok(vec![]) };
    let (name, warnings) = check_new_entry(&d.whitelist.entries, raw)?;
    d.whitelist.entries.push(name);
    Ok(warnings)
}

pub fn begin_save(app: &mut GuardianApp) {
    let (Some(s), Some(d)) = (&app.snapshot, &app.draft) else { return };
    let env = d.effective_env();
    match plan(s, &env, &d.whitelist) {
        Ok(p) if p.is_empty() => app.notice = Some((Status::Ok, "Nothing to save.".into())),
        Ok(p) => {
            let warnings = validate(s, &env, &d.whitelist).into_iter().filter(|i| i.severity == Severity::Warning).collect();
            app.dialog = Some(Dialog::Confirm { plan: p, warnings });
        }
        Err(errors) => app.notice = Some((Status::Error, errors.into_iter().map(|e| e.message).collect::<Vec<_>>().join(" "))),
    }
}

fn issues_for<'a>(issues: &'a [Issue], f: Field) -> impl Iterator<Item = &'a Issue> {
    issues.iter().filter(move |i| i.field == f)
}

fn show_issues(ui: &mut egui::Ui, issues: &[Issue], f: Field) {
    for i in issues_for(issues, f) {
        let c = if i.severity == Severity::Error { theme::palette().error } else { theme::palette().warn };
        ui.colored_label(c, &i.message);
    }
}

fn read_problem<T>(ui: &mut egui::Ui, what: &str, r: &Read<T>) {
    if let Some(p) = r.problem() {
        ui.colored_label(theme::palette().muted, format!("{what}: {p}"));
    }
}

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let Some(s) = app.snapshot.clone() else { return };
    let lock = edit_lock(&s);
    let editable = lock.is_none();
    let p = theme::palette();

    if let Some(msg) = lock {
        ui.colored_label(p.warn, msg);
    }
    // R10: pending restart banner
    if let Some(c) = app.diagnosis.as_ref().and_then(|d| d.checks.iter().find(|c| c.id == guardian_core::diagnose::CheckId::PendingRestart)) {
        if c.status == Status::Warn {
            ui.horizontal(|ui| {
                ui.colored_label(p.warn, "Changes apply after the service restarts.");
                if editable && !s.is_manual_dir() && ui.add_enabled(!app.busy(), egui::Button::new("Restart service")).clicked() {
                    app.worker = Some(Worker::spawn(Job::Restart, ui.ctx().clone()));
                }
            });
        }
    }
    ui.label(egui::RichText::new("whitelist.txt applies only in whitelist mode. The agent reads .env and whitelist.txt once, when the service starts.").color(p.muted));

    // Save bar
    let dirty = app.dirty();
    if editable && dirty > 0 {
        ui.horizontal(|ui| {
            ui.colored_label(p.warn, if dirty == 1 { "1 unsaved change".to_string() } else { format!("{dirty} unsaved changes") });
            if ui.add_enabled(!app.busy(), egui::Button::new("Save")).clicked() {
                begin_save(app);
            }
            if ui.button("Revert").clicked() {
                app.draft = Some(crate::state::Draft::from_snapshot(&s));
            }
        });
    }

    let issues = app.draft.as_ref().map(|d| validate(&s, &d.effective_env(), &d.whitelist)).unwrap_or_default();

    // Agent settings
    ui.add_space(8.0);
    ui.label(theme::heading("Agent settings"));
    ui.label(theme::mono(s.install.path.join(agent::ENV_FILE).display().to_string()));
    read_problem(ui, ".env", &s.env);
    if let Some(e) = s.env.as_ok() {
        if e.parsed.encoding == guardian_core::read::Encoding::Utf8Bom {
            ui.colored_label(p.warn, "The file starts with a byte-order mark the agent cannot read past; the first line is ignored.");
        }
        for (k, n) in &e.parsed.duplicates {
            ui.colored_label(p.warn, format!("{k} is defined {n} times; the last one is used."));
        }
    }
    if let Some(d) = app.draft.as_mut() {
        egui::Grid::new("env").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("SERVER_ADDRESS");
            ui.add_enabled(editable, egui::TextEdit::singleline(&mut d.env.server_address).font(egui::TextStyle::Monospace).desired_width(420.0));
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| {
                show_issues(ui, &issues, Field::ServerAddress);
                if let Some(c) = s.credentials.as_ok() {
                    let cur = guardian_core::edit::canonical_server_address(&d.env.server_address);
                    if editable && !c.server_address.is_empty() && c.server_address != cur && ui.button(format!("Use {}", c.server_address)).clicked() {
                        d.env.server_address = c.server_address.clone();
                    }
                }
            });
            ui.end_row();

            ui.label("CHECK_INTERVAL");
            ui.horizontal(|ui| {
                ui.add_enabled(editable, egui::TextEdit::singleline(&mut d.env.check_interval).font(egui::TextStyle::Monospace).desired_width(80.0));
                ui.label(format!("seconds (empty = agent default, {} s)", agent::DEFAULT_SERVICE_INTERVAL_SECS));
            });
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| show_issues(ui, &issues, Field::CheckInterval));
            ui.end_row();

            ui.label("BINDING_TOKEN");
            ui.vertical(|ui| {
                let current = s.env.as_ok().and_then(|e| e.parsed.binding_token.clone());
                let state = match (&d.env.binding_token, &current) {
                    (TokenEdit::Remove, _) => "will be removed".to_string(),
                    (_, Some(t)) => format!("present (…{})", t.0),
                    (_, None) => "absent".into(),
                };
                ui.label(theme::mono(state));
                ui.horizontal(|ui| {
                    ui.add_enabled(editable, egui::TextEdit::singleline(&mut d.token_field).password(true).hint_text("paste a new binding token").desired_width(320.0));
                    if current.is_some() && editable {
                        let removing = d.env.binding_token == TokenEdit::Remove;
                        if ui.button(if removing { "Keep" } else { "Remove" }).clicked() {
                            d.env.binding_token = if removing { TokenEdit::Keep } else { TokenEdit::Remove };
                        }
                    }
                });
                show_issues(ui, &issues, Field::BindingToken);
            });
            ui.end_row();
        });
        if let Some(e) = s.env.as_ok() {
            if !e.parsed.unknown.is_empty() {
                ui.collapsing("Other keys (ignored by the agent)", |ui| {
                    for (k, v) in &e.parsed.unknown {
                        ui.label(theme::mono(format!("{k}={v}")));
                    }
                });
            }
        }
    }

    // Enrollment and policy (read-only)
    ui.add_space(12.0);
    ui.label(theme::heading("Enrollment"));
    read_problem(ui, "agent_credentials.json", &s.credentials);
    if let Some(c) = s.credentials.as_ok() {
        ui.label(theme::mono(format!("machine {}\nserver  {}\nsince   {}", c.machine_guid, c.server_address, c.enrolled_at)));
    }
    ui.add_space(12.0);
    ui.label(theme::heading("Policy (from the last sync)"));
    read_problem(ui, "sync.json", &s.sync);
    if let Some(f) = s.sync.as_ok() {
        let at = f.mtime.map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        ui.label(theme::mono(format!("mode {} · synced {at}", f.parsed.mode.as_str())));
        for a in &f.parsed.applications {
            ui.label(theme::mono(&a.name));
        }
    }

    // Local whitelist
    ui.add_space(12.0);
    ui.label(theme::heading("Local whitelist"));
    ui.label(theme::mono(s.install.path.join(agent::WHITELIST_FILE).display().to_string()));
    match &s.whitelist {
        Read::Missing => {
            ui.colored_label(p.muted, "absent — the agent generates it from the running processes on its next start");
        }
        other => read_problem(ui, "whitelist.txt", other),
    }
    if editable {
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut app.new_entry).hint_text("program.exe or a pasted path").desired_width(320.0));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Add").clicked() || enter {
                let raw = std::mem::take(&mut app.new_entry);
                match add_entry(app, &raw) {
                    Ok(w) if w.is_empty() => app.notice = None,
                    Ok(w) => app.notice = Some((Status::Warn, w.into_iter().map(|i| i.message).collect::<Vec<_>>().join(" "))),
                    Err(e) => {
                        app.new_entry = raw;
                        app.notice = Some((Status::Error, e.message));
                    }
                }
            }
            ui.menu_button("Add from running", |ui| {
                let running = s.processes.as_ok().cloned().unwrap_or_default();
                let mut names: Vec<String> = running.iter().map(|n| n.to_lowercase()).collect();
                names.sort();
                names.dedup();
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for n in names {
                        let have = app.draft.as_ref().is_some_and(|d| d.whitelist.entries.contains(&n));
                        let label = if agent::is_protected(&n) { format!("{n}  (protected)") } else if have { format!("{n}  (in list)") } else { n.clone() };
                        if ui.add_enabled(!have && !agent::is_protected(&n), egui::Button::new(theme::mono(label))).clicked() {
                            let _ = add_entry(app, &n);
                        }
                    }
                });
            });
        });
    }
    if let Some(d) = app.draft.as_mut() {
        let mut remove = None;
        for (i, e) in d.whitelist.entries.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(theme::mono(e));
                if agent::is_protected(e) {
                    ui.colored_label(p.muted, "already protected");
                }
                if editable && ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            d.whitelist.entries.remove(i);
        }
    }

    ui.add_space(12.0);
    ui.collapsing("Built-in protected processes", |ui| {
        for n in agent::PROTECTED {
            ui.label(theme::mono(*n));
        }
    });
}

pub fn dialogs(app: &mut GuardianApp, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.take() else { return };
    let mut keep = true;
    egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
        ui.set_width(520.0);
        match &dialog {
            Dialog::DiscardOnRefresh => {
                ui.label("Discard unsaved changes?");
                ui.horizontal(|ui| {
                    if ui.button("Discard and refresh").clicked() {
                        keep = false;
                        app.refresh(ctx);
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::Confirm { plan, warnings } => {
                ui.label(theme::heading("Save changes?"));
                for f in &plan.files {
                    ui.label(egui::RichText::new(f.kind.file_name()).strong());
                    for l in &f.summary {
                        ui.label(theme::mono(l));
                    }
                }
                for w in warnings {
                    ui.colored_label(theme::palette().warn, &w.message);
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        keep = false;
                        app.worker = Some(Worker::spawn(Job::Apply { plan: plan.clone(), overwrite: false }, ctx.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::ChangedOnDisk { plan } => {
                let names: Vec<&str> = plan.files.iter().map(|f| f.kind.file_name()).collect();
                ui.label(format!("{} changed on disk since it was loaded.", names.join(", ")));
                ui.horizontal(|ui| {
                    if ui.button("Reload").clicked() {
                        keep = false;
                        app.draft = None;
                        app.refresh(ctx);
                    }
                    if ui.button("Overwrite").clicked() {
                        keep = false;
                        app.worker = Some(Worker::spawn(Job::Apply { plan: plan.clone(), overwrite: true }, ctx.clone()));
                    }
                });
            }
            Dialog::Restart => {
                ui.label(theme::heading("Restart ProcSentinelAgent now?"));
                ui.label("Saved files take effect when the service restarts.");
                if app.snapshot.as_ref().and_then(|s| s.sync.as_ok()).is_some_and(|f| f.parsed.mode == guardian_core::parse::Mode::Whitelist) {
                    ui.colored_label(theme::palette().warn, "In whitelist mode the computer is unprotected for a few seconds while the service restarts.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Restart").clicked() {
                        keep = false;
                        app.worker = Some(Worker::spawn(Job::Restart, ctx.clone()));
                    }
                    if ui.button("Later").clicked() {
                        keep = false;
                        app.refresh(ctx);
                    }
                });
            }
            Dialog::Error(m) => {
                ui.colored_label(theme::palette().error, m);
                if ui.button("OK").clicked() {
                    keep = false;
                }
            }
        }
    });
    // A button that opened a follow-up (worker → on_done) leaves app.dialog for on_done to set.
    if keep && app.dialog.is_none() {
        app.dialog = Some(dialog);
    }
}

pub fn on_done(app: &mut GuardianApp, done: Done, ctx: &egui::Context) {
    match done {
        Done::Collected(_) => {}
        Done::Applied(Ok(_)) => {
            // Keep the token out of memory once written (R4).
            if let Some(d) = app.draft.as_mut() {
                d.token_field.clear();
            }
            let manual = app.snapshot.as_ref().is_some_and(|s| s.is_manual_dir());
            app.notice = Some((Status::Ok, "Saved.".into()));
            if manual {
                app.refresh(ctx);
            } else {
                app.dialog = Some(Dialog::Restart);
            }
        }
        Done::Applied(Err(e)) if e.step == guardian_winhost::apply::Step::Changed => {
            let (Some(s), Some(d)) = (&app.snapshot, &app.draft) else { return };
            if let Ok(p) = plan(s, &d.effective_env(), &d.whitelist) {
                app.dialog = Some(Dialog::ChangedOnDisk { plan: p });
            }
        }
        Done::Applied(Err(e)) => app.dialog = Some(Dialog::Error(e.to_string())),
        Done::Restarted(Ok(())) => {
            app.notice = Some((Status::Ok, "ProcSentinelAgent restarted.".into()));
            app.refresh(ctx);
        }
        Done::Restarted(Err(e)) => {
            app.dialog = Some(Dialog::Error(format!("The files are saved, but the service did not restart: {e}. See the Log tab.")));
            app.refresh(ctx);
        }
        Done::ReportSaved(Ok(p)) => app.notice = Some((Status::Ok, format!("Report saved to {}", p.display()))),
        Done::ReportSaved(Err(e)) => app.dialog = Some(Dialog::Error(format!("Could not save the report: {e}. Choose another folder."))),
    }
}
```
`set_snapshot` (Task 10) rebuilds the draft from the new snapshot; that's right after a save or a Reload. After a plain Refresh with no unsaved changes, it's also right.

`Done::ReportSaved(Err)` should offer a folder picker (R2): in the `Dialog::Error` branch for report errors, add a "Choose folder…" button that calls `rfd::FileDialog::new().pick_folder()` and `guardian_winhost::save_report_to(&snapshot, &dir)` (Task 12 adds `save_report_to`). Implement it in Task 12 Step 3.

- [ ] **Step 3: Run tests, try it, commit**

Run: `cargo test -p guardian-ui && cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS.
Manual: `cp -r crates/core/tests/fixtures/healthy /tmp/agent-copy && cargo run -p guardian -- --dir /tmp/agent-copy --tab configuration`; add `winword.exe`, Save, confirm; check `/tmp/agent-copy/whitelist.txt` and the `.bak-*` file. (Manual folder → no restart dialog.)
```bash
git add tools/guardian-diag
git commit -m "feat(diag): edit the agent's settings and whitelist from the Configuration tab"
```

---

### Task 12: Processes and Log tabs, Save report

**Files:**
- Modify: `crates/ui-egui/src/tabs/processes.rs`, `crates/ui-egui/src/tabs/log.rs`
- Create: `crates/winhost/src/report_file.rs` (`save_report`, `save_report_to`), re-export from `lib.rs`
- Modify: `apps/guardian/src/main.rs` (use `guardian_winhost::save_report` instead of its local copy)
- Test: `crates/winhost/src/report_file.rs` unit test, `crates/ui-egui/tests/processes_log.rs`

**Interfaces:**
- Consumes: `enforce::{verdict_for, ProcessVerdict}`, `report::{report, kill_summary}`, `configuration::add_entry`.
- Produces: `guardian_winhost::{save_report(&Snapshot) -> Result<PathBuf, String>, save_report_to(&Snapshot, dir: &Path) -> Result<PathBuf, String>}`; `log::{Filter { All, Kills, Problems, Sync }, Period { Hour, Day, Week, All }, visible(events, filter, period, search, now) -> Vec<&AgentEvent>}`.

- [ ] **Step 1: `report_file.rs` with a test**

```rust
use std::path::{Path, PathBuf};

use guardian_core::diagnose::diagnose;
use guardian_core::report::report;
use guardian_core::snapshot::Snapshot;

/// `<dir>\<hostname>-<YYYYMMDD-HHMM>.txt` and `.json` (spec R24). Returns the .txt path.
pub fn save_report_to(s: &Snapshot, dir: &Path) -> Result<PathBuf, String> {
    let r = report(s, &diagnose(s));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let host = if s.machine.hostname.is_empty() { "computer" } else { s.machine.hostname.as_str() };
    let stem = format!("{host}-{}", s.taken_at.with_timezone(&chrono::Local).format("%Y%m%d-%H%M"));
    let txt = dir.join(format!("{stem}.txt"));
    std::fs::write(&txt, &r.text).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_string_pretty(&r.json).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(txt)
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

    #[test]
    fn writes_txt_and_json_without_secrets() {
        let s = guardian_core::testkit::snapshot_from_dir(&guardian_core::testkit::fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        let d = tempfile::tempdir().unwrap();
        let p = save_report_to(&s, d.path()).unwrap();
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("LAB-A-05-2026101"));
        let txt = std::fs::read_to_string(&p).unwrap();
        assert!(!txt.contains("agt_secret_value_123"));
        assert!(p.with_extension("json").exists());
    }
}
```
Add `serde_json = { workspace = true }` to winhost's dependencies. Replace `save_report_next_to_exe` in `apps/guardian/src/main.rs` with `guardian_winhost::save_report(&s)`. In `configuration::dialogs`' report-error path, add "Choose folder…" → `rfd::FileDialog::new().pick_folder()` → `save_report_to`.

- [ ] **Step 2: Failing kittest** (`tests/processes_log.rs`)

```rust
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};
use guardian_ui::GuardianApp;
use guardian_ui::state::Tab;

fn harness(tab: Tab) -> Harness<'static, GuardianApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_eframe(move |cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        GuardianApp::with_snapshot(snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap()), tab)
    });
    h.run_steps(2);
    h
}

#[test]
fn processes_verdicts_and_add_to_whitelist() {
    let mut h = harness(Tab::Processes);
    h.get_by_label("game.exe");
    h.get_by_label_contains("Will be closed");
    h.get_by_label("Add to local whitelist").click();
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Configuration);
    assert!(h.state().draft.as_ref().unwrap().whitelist.entries.contains(&"game.exe".to_string()));
}

#[test]
fn why_is_it_closed() {
    let mut h = harness(Tab::Processes);
    // chrome.exe is in the policy but not running, so the only match is the answer itself.
    h.state_mut().why_query = "chrome.exe".into();
    h.run_steps(2);
    h.get_by_label_contains("Allowed (policy)");
}

#[test]
fn log_kill_summary() {
    let mut h = harness(Tab::Log);
    h.state_mut().kill_summary = true;
    h.run_steps(2);
    h.get_by_label("game.exe");
    h.get_by_label("steam.exe");
    h.get_by_label_contains("6 records");
}
```
Run: `cargo test -p guardian-ui --test processes_log` — Expected: FAIL.

- [ ] **Step 3: `processes.rs`**

```rust
use guardian_core::enforce::{ProcessVerdict, verdict_for};

use crate::state::{Tab, edit_lock};
use crate::{GuardianApp, theme};

fn colour(v: &ProcessVerdict) -> egui::Color32 {
    let p = theme::palette();
    match v {
        ProcessVerdict::WillClose { .. } => p.error,
        ProcessVerdict::Unknown(_) => p.muted,
        _ => p.text,
    }
}

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let Some(s) = app.snapshot.clone() else { return };
    let editable = edit_lock(&s).is_none();

    ui.label(theme::heading("Why is … closed?"));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.why_query).hint_text("program.exe").desired_width(260.0));
        if !app.why_query.trim().is_empty() {
            let v = verdict_for(&s, app.why_query.trim());
            ui.colored_label(colour(&v), v.label());
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.process_filter).hint_text("filter").desired_width(200.0));
        ui.checkbox(&mut app.only_closing, "only what will be closed");
    });
    if let Some(problem) = s.processes.problem() {
        ui.colored_label(theme::palette().muted, format!("Running processes: {problem}"));
        return;
    }
    ui.label(egui::RichText::new("Verdicts use the saved files, not unsaved edits.").color(theme::palette().muted));
    let mut names: Vec<String> = s.processes.as_ok().cloned().unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by_key(|n| n.to_lowercase());
    let filter = app.process_filter.to_lowercase();
    let mut add: Option<String> = None;
    egui::Grid::new("procs").num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
        for n in &names {
            let v = verdict_for(&s, n);
            if (!filter.is_empty() && !n.to_lowercase().contains(&filter)) || (app.only_closing && !v.will_close()) {
                continue;
            }
            ui.label(theme::mono(n));
            ui.colored_label(colour(&v), v.label());
            if editable && v.will_close() && matches!(v, ProcessVerdict::WillClose { rule: None }) && ui.small_button("Add to local whitelist").clicked() {
                add = Some(n.clone());
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    if let Some(n) = add {
        if let Err(e) = crate::tabs::configuration::add_entry(app, &n) {
            app.notice = Some((guardian_core::diagnose::Status::Error, e.message));
        }
        app.tab = Tab::Configuration;
    }
}
```
"Add to local whitelist" only for whitelist-mode closes (`rule: None`): in blacklist mode `whitelist.txt` does not help, and offering it would mislead.

- [ ] **Step 4: `log.rs`**

```rust
use chrono::{DateTime, Utc};
use guardian_core::parse::{AgentEvent, EventKind, KillReason, Level};
use guardian_core::report::kill_summary;

use crate::state::{Tab, edit_lock};
use crate::{GuardianApp, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Kills,
    Problems,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Hour,
    Day,
    Week,
    All,
}

pub fn visible<'a>(events: &'a [AgentEvent], f: Filter, p: Period, search: &str, now: DateTime<Utc>) -> Vec<&'a AgentEvent> {
    let since = match p {
        Period::Hour => Some(now - chrono::Duration::hours(1)),
        Period::Day => Some(now - chrono::Duration::hours(24)),
        Period::Week => Some(now - chrono::Duration::days(7)),
        Period::All => None,
    };
    let q = search.to_lowercase();
    events
        .iter()
        .filter(|e| since.is_none_or(|s| e.time >= s))
        .filter(|e| match f {
            Filter::All => true,
            Filter::Kills => matches!(e.kind, EventKind::Kill { .. }),
            Filter::Problems => e.level != Level::Info,
            Filter::Sync => matches!(e.kind, EventKind::Sync) || e.message.starts_with("Sync failed"),
        })
        .filter(|e| q.is_empty() || e.message.to_lowercase().contains(&q))
        .collect()
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let Some(s) = app.snapshot.clone() else { return };
    let Some(events) = s.events.as_ok() else {
        ui.colored_label(theme::palette().muted, format!("Event Log: {}", s.events.problem().unwrap_or_default()));
        return;
    };
    if events.is_empty() {
        ui.label("No ProcSentinelAgent records. If the agent was only ever run from a console, it never wrote to the Event Log.");
        return;
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.kill_summary, "Kill summary");
        if !app.kill_summary {
            for (f, l) in [(Filter::All, "All"), (Filter::Kills, "Kills"), (Filter::Problems, "Errors and warnings"), (Filter::Sync, "Sync")] {
                ui.selectable_value(&mut app.log_filter, f, l);
            }
            ui.separator();
            for (p, l) in [(Period::Hour, "1 h"), (Period::Day, "24 h"), (Period::Week, "7 days"), (Period::All, "all")] {
                ui.selectable_value(&mut app.log_period, p, l);
            }
            ui.add(egui::TextEdit::singleline(&mut app.log_search).hint_text("search").desired_width(180.0));
        }
    });
    let oldest = events.iter().map(|e| e.time).min();
    ui.label(egui::RichText::new(format!("{} records, the log holds entries since {}", events.len(), oldest.map(local).unwrap_or_default())).color(theme::palette().muted));

    let editable = edit_lock(&s).is_none();
    let mut add: Option<String> = None;
    if app.kill_summary {
        egui::Grid::new("kills").num_columns(5).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
            for k in kill_summary(events) {
                ui.label(theme::mono(&k.process));
                ui.label(format!("×{}", k.count));
                ui.label(theme::mono(format!("{} … {}", local(k.first), local(k.last))));
                ui.label(match k.reason {
                    KillReason::Blacklist => "blacklist",
                    KillReason::Whitelist => "whitelist",
                });
                if editable && k.reason == KillReason::Whitelist && ui.small_button("Add to local whitelist").clicked() {
                    add = Some(k.process.clone());
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    } else {
        egui::Grid::new("log").num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
            for e in visible(events, app.log_filter, app.log_period, &app.log_search, s.taken_at) {
                ui.label(theme::mono(local(e.time)));
                let c = match e.level {
                    Level::Error => theme::palette().error,
                    Level::Warning => theme::palette().warn,
                    Level::Info => theme::palette().muted,
                };
                ui.colored_label(c, format!("{:?}", e.level));
                ui.label(&e.message);
                ui.end_row();
            }
        });
    }
    if let Some(n) = add {
        if let Err(e) = crate::tabs::configuration::add_entry(app, &n) {
            app.notice = Some((guardian_core::diagnose::Status::Error, e.message));
        }
        app.tab = Tab::Configuration;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};

    #[test]
    fn visible_filters() {
        let now: DateTime<Utc> = "2026-10-10T11:15:00Z".parse().unwrap();
        let s = snapshot_from_dir(&fixtures_dir().join("healthy"), now);
        let ev = s.events.as_ok().unwrap();
        assert_eq!(visible(ev, Filter::All, Period::All, "", now).len(), 6);
        assert_eq!(visible(ev, Filter::Kills, Period::All, "", now).len(), 2);
        assert_eq!(visible(ev, Filter::Problems, Period::All, "", now).len(), 2);
        assert_eq!(visible(ev, Filter::Sync, Period::All, "", now).len(), 2);
        assert_eq!(visible(ev, Filter::All, Period::All, "STEAM", now).len(), 1);
        assert_eq!(visible(ev, Filter::All, Period::Hour, "", now + chrono::Duration::hours(2)).len(), 0);
    }
}
```

- [ ] **Step 5: Run, commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS.
```bash
git add tools/guardian-diag
git commit -m "feat(diag): Processes and Log tabs, and Save report"
```

---

### Task 13: Windows CI, manual verification, documentation

**Files:**
- Modify: `.github/workflows/diag.yml` (Windows job)
- Create: `specs/guardian-diag.md`
- Modify: `specs/2026-10-10-guardian-diag-design.md` (R12/R17 substring correction, R14.3 rename note, R25 console note), `specs/whitelist-gui.md` (status note), `CLAUDE.md` (component, build, tests, the "Guardian Console" line), `README.md` (one line)

- [ ] **Step 1: Windows CI job** — append to `diag.yml`:

```yaml
  windows:
    runs-on: windows-latest
    defaults:
      run:
        working-directory: tools/guardian-diag
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: i686-pc-windows-msvc
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: tools/guardian-diag
      - name: tests (winhost apply on a temp folder, report CLI)
        run: cargo test -p guardian-core -p guardian-winhost -p guardian --target i686-pc-windows-msvc
      - name: release build
        env:
          GUARDIAN_REQUIRE_WINRES: "1"
        run: cargo build --release -p guardian --target i686-pc-windows-msvc
      - name: smoke test
        shell: pwsh
        run: |
          $exe = "target/i686-pc-windows-msvc/release/Guardian.exe"
          & $exe report --offline --dir crates/core/tests/fixtures/healthy > report.txt
          $code = $LASTEXITCODE
          if (@(0,10,11) -notcontains $code) { throw "unexpected exit code $code" }
          if (-not (Select-String -Path report.txt -Pattern "VERDICT:")) { throw "no verdict in the report" }
          if (Select-String -Path report.txt -Pattern "agt_secret_value_123") { throw "secret leaked" }
          # One file, no DLL dependencies beyond the OS
          dumpbin /dependents $exe | Select-String -Pattern "VCRUNTIME|MSVCP" | ForEach-Object { throw "dynamic CRT: $_" }
      - uses: actions/upload-artifact@v4
        with:
          name: Guardian.exe
          path: tools/guardian-diag/target/i686-pc-windows-msvc/release/Guardian.exe
```
`dumpbin` needs the VS developer shell; if it is not on PATH in the runner, use `ilammy/msvc-dev-cmd@v1` before the step.

- [ ] **Step 2: Manual verification on Windows 10 x64 (and x86 if available)** — record results in the PR description:
  1. Non-admin double-click from a USB stick: window opens, verdict shown, Configuration read-only with "Run as administrator to edit"; unreadable fields say "no access".
  2. "Restart as administrator" → UAC → reopens on the same tab; cancel UAC → old window stays with "Elevation was cancelled".
  3. Admin: add `winword.exe`, Save → confirm → `whitelist.txt` updated, `whitelist.txt.bak-*` present, restart dialog → Restart → Processes shows `winword.exe` "Allowed (whitelist.txt)"; Event Log shows a new "Server address:" start record.
  4. Change `CHECK_INTERVAL` to 15 → Save → Restart → `Synced:` records ~15 s apart.
  5. Change `SERVER_ADDRESS` to another host → the drop-out warning appears; Cancel.
  6. Open `.env` in Notepad between load and Save, edit, save → Guardian's Save shows "changed on disk" → Reload.
  7. Agent in whitelist mode: the window survives (protected name `guardian.exe`).
  8. 32-bit `Guardian.exe` on 64-bit Windows: paths shown as `System32`, edits land in the real folder (check with 64-bit Explorer).
  9. `Guardian.exe report > r.txt` from cmd; `Guardian.exe report` without redirect → message box with the saved path.

- [ ] **Step 3: Documentation**
  - `specs/guardian-diag.md`: the final description — purpose, tabs, what it writes and when, CLI and exit codes, build (`cargo build --release -p guardian --target i686-pc-windows-msvc` from `tools/guardian-diag`), tests (`cargo test --workspace`), the agent-consistency tests and what to do when they fail, the two-`Guardian.exe` situation (installer archive still ships the Go console).
  - Design spec: R12/R17 → exact-name `taskkill /IM` semantics with the substring *attempt* note (from Task 4); R14.3 → `std::fs::rename` + `sync_all` (Task 7 note); R25 → output reaches a redirect/pipe, otherwise saved to `GuardianReports\` with a message box.
  - `specs/whitelist-gui.md`: a status line at the top — "Viewing and editing the agent's config moved to Guardian Diag (`tools/guardian-diag`); this console remains the installer/updater shipped in the cabinet's installer archive."
  - `CLAUDE.md`: add **Guardian Diag** to the component list and Build Commands (`cd tools/guardian-diag && cargo build --release -p guardian --target i686-pc-windows-msvc`; `cargo test --workspace`), the CI line (`diag.yml`), and a Rules bullet: "`tools/guardian-diag/crates/core/src/agent.rs` mirrors the agent's protected list, file names, `.env` keys and log strings; `tests/agent_consistency.rs` enforces it." Extend the existing `.env`-keys rule: "change all three" → also `crates/core/src/agent.rs`.
  - `README.md`: one line under tools.

- [ ] **Step 4: Final checks and commit**

Run: `cd tools/guardian-diag && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all green.
```bash
git add .github/workflows/diag.yml specs CLAUDE.md README.md
git commit -m "docs(diag): document Guardian Diag and run it on Windows in CI"
```
