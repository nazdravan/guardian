# Guardian 3.0 — Etapa 2: agentul — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un agent Guardian 3.0 real pe Linux și Windows (32/64 de biți): se înrolează, ține conexiunea WebSocket cu serverul din etapa 1, salvează starea, omoară procesele după profil cu detectare hibridă (notificări + reconciliere + rezervă la 1 s), pornește un worker schelet în sesiunea userului și se instalează ca serviciu.

**Architecture:** `guardian-platform` ascunde OS-ul în spatele trait-ului `Platform` (listare/omorâre procese, sursa de evenimente de procese, sesiunea activă și pornirea unui proces în ea, informații de sistem, rădăcinile de încredere); implementări `linux/` (procfs, proc connector prin netlink cu `nix`, `loginctl`) și `windows/` (crate-ul `windows`, ETW prin `ferrisetw`, WTS), plus `FakePlatform` pentru teste. `guardian-agent` (librărie + binar) leagă: magazinul de fișiere, motorul de enforcement (decizia rămâne în `guardian-core::enforce::decide`), conexiunea WS (cu `guardian-client`), managerul de worker. `guardian-worker` e un binar subțire care se conectează la agent prin IPC local (socket unix / named pipe, cadre protobuf cu lungime) și ține legătura; funcțiile vizibile (mesaj, countdown) vin în etapa 4.

**Tech Stack:** cele din etapa 1, plus `nix` (Linux: semnale, netlink), `windows` 0.62 (Toolhelp, procese, WTS, named pipe), `ferrisetw` 1.2 (ETW), `windows-service` 0.8, `if-addrs` (adrese LAN), `serde` + `toml` (config). Teste: `guardian-server` cu feature `testing` (`TestServer`), `tempfile`.

**Spec:** [`specs/2026-11-17-rust-rewrite-design.md`](../2026-11-17-rust-rewrite-design.md) §8.1, §8.2, §8.4, §10, §14 punctul 2. Serverul cu care vorbește agentul: [`specs/server-v3.md`](../server-v3.md) (protocolul WebSocket, pașii 1–7). Etapa 1: [`2026-11-17-rust-3-stage1-server.md`](2026-11-17-rust-3-stage1-server.md).

## Înainte de a începe

- Se lucrează pe branch-ul **`rust-rewrite`** (etapa 1 e acolo). Fără `feat` în nume și mesaje; commit-uri `<crate sau zonă>: <propoziție>`, fără trailer-e de atribuire.
- Pe macOS (mașina proprietarului) codul Linux și Windows nu rulează. Linux: `podman machine start`, apoi testele într-un container `rust:1.95` cu repo-ul montat (comanda exactă în Task 4). Windows: `cargo check --target x86_64-pc-windows-gnu` și `--target i686-pc-windows-gnu` local (rustup + mingw sunt instalate: `RUSTC=$HOME/.cargo/bin/rustc ~/.cargo/bin/rustup run stable cargo check …`); rularea reală se face în jobul Windows din CI (Task 15).
- Nu există un Windows fizic la îndemână: verificarea manuală pe Windows (serviciu, worker în sesiune) e o listă de pași în `specs/agent-v3.md`, nu un pas al planului.

## Global Constraints

- MSRV **1.88** (workspace). Nu se adaugă crate-uri care cer mai mult (de aceea fără `zbus` 6 și `sysinfo` 0.40).
- Licențe: doar cele din `deny.toml` (fără GPL, fără MPL); `cargo deny check` trece la fiecare task care adaugă dependențe.
- `unsafe`: interzis (`#![forbid(unsafe_code)]`) în `guardian-agent`, `guardian-worker` și în toată logica din `guardian-platform`; singurele locuri cu `#[allow(unsafe_code)]` sunt modulele de FFI `guardian-platform/src/windows/ffi_*.rs`, fiecare bloc `unsafe` cu un comentariu `// SAFETY:`. Pe Linux nu există `unsafe` (totul prin `nix` și `std`).
- Fără `unwrap`/`expect`/`panic` în cod de producție (regulile de lint ale workspace-ului).
- Ținte: `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `i686-pc-windows-msvc` (agent + worker pe 32 și 64 de biți). Pe alte OS-uri (macOS) workspace-ul compilează: `guardian-platform` are o implementare `unsupported` care întoarce `PlatformError::Unsupported`.
- Reconectare: backoff exponențial cu jitter, de la **1 s** la maxim **60 s**; după o conexiune autentificată, backoff-ul revine la 1 s.
- **Offline** = fără conexiune autentificată de cel puțin **60 s** (doar se raportează în etapa 2; modul local e etapa 3).
- Reconciliere completă: la pornire, la fiecare stare nouă aplicată și la fiecare **10 s**. Rezervă: dacă sursa de evenimente nu pornește sau cade, scanare completă la **1 s**, cu reîncercarea sursei la **60 s**, și o linie `warn!` la fiecare trecere între moduri.
- Fără stare salvată: se aplică profilul `Lecție` (`blacklist`, listă goală) cu id 0 — nu omoară nimic, nu e `free`.
- O stare primită se aplică doar dacă `version` e **strict mai mare** decât cea curentă (`specs/server-v3.md`, pasul 5).
- Comenzile se deduplică după `id` (serverul retrimite comenzile `delivered` la reconectare); agentul ține ultimele **1000** de id-uri văzute, salvate pe disc.
- În etapa 2 agentul anunță `capabilities = []` (funcțiile vin în etapa 4). O comandă primită totuși (sau expirată) primește `CommandResult{status: RESULT_UNSUPPORTED}` o singură dată per id.
- Fișiere și drepturi:
  - Linux: config `/etc/guardian-agent/agent.toml` (0600, root), date `/var/lib/guardian-agent/` (0700, root), binare `/opt/guardian-agent/`, socket worker `/run/guardian-agent/worker.sock`.
  - Windows: binare `%ProgramFiles%\Guardian\` (agentul de 64 de biți pe Windows de 64 de biți), config + date `%ProgramData%\Guardian Agent\` cu ACL protejat (SYSTEM + Administrators, ca în `service_windows.rs` al serverului), pipe `\\.\pipe\guardian-agent-worker`.
  - Fișiere din directorul de date: `agent.key` (seed ed25519, 32 de octeți), `state.pb` (`StoredState`), `seen.txt` (id-uri de comenzi), `whitelist.txt` (local, un nume pe linie, `#` = comentariu), `agent.log` (doar la serviciul Windows).
- Toate scrierile de fișiere de stare: fișier temporar în același director + `sync_all` + `rename`.
- Numele binarelor: `guardian-agent`, `guardian-worker` (`.exe` pe Windows) — sunt deja în listele protejate din `guardian-core::protected`.
- **În afara etapei 2** (din `Platform` din spec §10.5): oprire/restart (vin cu funcțiile din etapa 4), firewall pentru portul local 11500 (etapa 3), canalul local Noise (etapa 3), afișarea a orice în worker (etapa 4).
- Dependențe noi permise: `nix`, `windows` 0.62, `ferrisetw`, `if-addrs`, `async-trait`, `serde`/`toml` (deja în workspace). Pe Linux worker-ul primește doar `uid`/`gid` ale userului (grupurile suplimentare ar cere `pre_exec`, adică `unsafe`).

## Review Focus

1. **Un program redenumit cu un nume protejat, într-un director în care elevul poate scrie** (`Downloads\svchost.exe`, `C:\Windows\Temp\explorer.exe`, `~/bash`): în Examen e omorât; doar copiile din rădăcinile de încredere ale sistemului rămân protejate. → teste în Task 1 (decizia) și Task 4/6 (rădăcinile fiecărui OS).
2. **PID refolosit între listare și omorâre**: agentul nu omoară niciodată alt proces decât cel pe care l-a judecat (numele, calea și marcajul de pornire trebuie să coincidă la momentul omorârii). → test în Task 4 (Linux, marcaj vechi → refuz); pe Windows aceeași regulă, verificată prin handle în Task 6.
3. **Serverul nu răspunde la pornirea calculatorului** (sau agentul repornește cât e offline): agentul aplică imediat starea salvată, înainte de orice încercare de conectare; fără stare salvată aplică „Lecție" gol. → test în Task 11.
4. **Fișierul de stare corupt sau trunchiat** (curent tăiat): agentul nu cade, scrie un `warn!`, pornește cu „Lecție" gol și suprascrie fișierul la prima stare primită. → test în Task 8.
5. **Sursa de evenimente cade în timpul rulării** (sesiune ETW oprită, netlink `ENOBUFS`): agentul trece pe scanare la 1 s fără să lase să treacă un proces interzis mai mult de ~1 s, apoi revine la evenimente când sursa repornește. → test în Task 9.

---

## File Structure

```
proto/guardian/v1/agent.proto        # + StoredState
proto/guardian/v1/worker.proto       # cadrele IPC agent ↔ worker
crates/guardian-core/src/enforce.rs  # decide(.., &TrustedRoots, ..); ProcessInfo.start_marker
crates/guardian-core/src/trusted.rs  # TrustedRoots
crates/guardian-platform/
  src/lib.rs                         # trait Platform, ProcessEventSource, UserSession, PlatformError, current()
  src/fake.rs                        # (feature "testing") FakePlatform, FakeEvents
  src/unsupported.rs                 # alte OS-uri
  src/ipc.rs                         # cadre cu lungime + IpcListener/IpcStream (socket unix / named pipe, PID-ul peer-ului)
  src/linux/{mod,procs,events,session,sysinfo}.rs
  src/windows/{mod,procs,events,session,sysinfo}.rs
  src/windows/ffi_procs.rs, ffi_session.rs, ffi_pipe.rs   # singurele locuri cu unsafe
crates/guardian-agent/
  src/lib.rs                         # Agent::run, AgentHandle
  src/config.rs                      # AgentConfig, AgentPaths
  src/store.rs                       # atomic_write, KeyStore, StateStore, SeenIds, load_whitelist
  src/engine.rs                      # Engine (aplicarea politicii) + supervizorul hibrid
  src/conn.rs                        # enroll(), sesiunea WS, backoff
  src/worker.rs                      # WorkerManager (IPC server, pornire în sesiune)
  src/main.rs                        # clap: enroll, run, status, service …
  src/service_windows.rs
  tests/*.rs
crates/guardian-worker/src/{lib,main}.rs  # schelet: run(endpoint) — conectare IPC, hello, ping/pong
packaging/agent-linux-v3/            # guardian-agent.service, install.sh, uninstall.sh
specs/agent-v3.md
```

---

### Task 1: Rădăcini de încredere în decizia de enforcement

**Files:**
- Create: `crates/guardian-core/src/trusted.rs`
- Modify: `crates/guardian-core/src/enforce.rs`, `crates/guardian-core/src/lib.rs`
- Modify: `specs/2026-11-17-rust-rewrite-design.md` (nota din §10.1 devine „rezolvat în etapa 2" cu data)

**Interfaces:**
- Produces:
  - `pub struct TrustedRoots { pub recursive: Vec<PathBuf>, pub direct_files: Vec<PathBuf>, pub excluded: Vec<PathBuf> }` cu `pub fn contains(&self, os: Os, exe: &Path) -> bool` și `pub fn none() -> TrustedRoots`
    - `recursive`: orice fișier de sub director; `direct_files`: doar fișiere aflate direct în director (pentru `C:\Windows\explorer.exe`); `excluded`: subdirectoare (recursiv) scoase din încredere.
    - Comparația: pe Windows fără diferență de majuscule, cu `/` și `\` echivalente; pe Linux exactă pe componente. Componentele `..` din `exe` → `false`.
  - `ProcessInfo` primește `pub start_marker: u64` (Linux: `starttime` din `/proc/<pid>/stat`; Windows: `CreationTime` ca FILETIME în u64; 0 = necunoscut).
  - `pub fn decide(os: Os, profile: &Profile, local_whitelist: &BTreeSet<AppName>, trusted: &TrustedRoots, p: &ProcessInfo) -> Verdict`

Regula de protecție nouă (înlocuiește „protejat dacă numele e în listă"):
- `exe_path = Some(path)`: protejat ⇔ numele (sau numele fără `.exe`) e în lista OS-ului **și** `trusted.contains(os, path)`.
- `exe_path = None`: pe Linux protejat (fir de kernel); pe Windows protejat doar dacă numele e în listă (pseudo-procesele `system`, `registry`, `secure system`, `system idle process` și procesele protejate pe care nu le putem deschide).
- Restul regulilor (`free`/`blacklist`/`whitelist`, `whitelist.txt` local) rămân neschimbate.

- [ ] **Step 1: Actualizează testele existente din `enforce.rs`** (helper-ul `proc_` primește `exe_path` sub o rădăcină de încredere de test și `start_marker: 0`; toate apelurile `decide` primesc `&roots()` cu `roots() = TrustedRoots { recursive: vec!["/x".into()], .. }`) și **adaugă**:

```rust
#[test] fn renamed_binary_outside_trusted_roots_is_killed_in_exam() {
    let p = prof(Mode::Whitelist, &[]);
    let fake = ProcessInfo { pid: 9, name: "svchost.exe".into(), exe_path: Some(r"C:\Users\elev\Downloads\svchost.exe".into()), start_marker: 1 };
    assert_eq!(decide(Os::Windows, &p, &Default::default(), &win_roots(), &fake), Verdict::Kill);
    let real = ProcessInfo { exe_path: Some(r"C:\Windows\System32\svchost.exe".into()), ..fake.clone() };
    assert_eq!(decide(Os::Windows, &p, &Default::default(), &win_roots(), &real), Verdict::Allow);
}
#[test] fn excluded_writable_dir_is_not_trusted() {
    let p = prof(Mode::Whitelist, &[]);
    let t = ProcessInfo { pid: 9, name: "explorer.exe".into(), exe_path: Some(r"c:/windows/temp/explorer.exe".into()), start_marker: 1 };
    assert_eq!(decide(Os::Windows, &p, &Default::default(), &win_roots(), &t), Verdict::Kill);
    let real = ProcessInfo { exe_path: Some(r"C:\WINDOWS\explorer.exe".into()), ..t };
    assert_eq!(decide(Os::Windows, &p, &Default::default(), &win_roots(), &real), Verdict::Allow);
}
#[test] fn linux_home_bash_is_killed_system_bash_is_not() {
    let p = prof(Mode::Whitelist, &[]);
    let roots = TrustedRoots { recursive: vec!["/usr".into(), "/bin".into()], ..TrustedRoots::none() };
    let home = ProcessInfo { pid: 7, name: "bash".into(), exe_path: Some("/home/elev/bash".into()), start_marker: 1 };
    assert_eq!(decide(Os::Linux, &p, &Default::default(), &roots, &home), Verdict::Kill);
    let sys = ProcessInfo { exe_path: Some("/usr/bin/bash".into()), ..home.clone() };
    assert_eq!(decide(Os::Linux, &p, &Default::default(), &roots, &sys), Verdict::Allow);
    let dotdot = ProcessInfo { exe_path: Some("/usr/../home/elev/bash".into()), ..home };
    assert_eq!(decide(Os::Linux, &p, &Default::default(), &roots, &dotdot), Verdict::Kill);
}
#[test] fn windows_pseudo_process_without_path_is_protected() {
    let sys = ProcessInfo { pid: 4, name: "System".into(), exe_path: None, start_marker: 0 };
    assert_eq!(decide(Os::Windows, &prof(Mode::Whitelist, &[]), &Default::default(), &TrustedRoots::none(), &sys), Verdict::Allow);
}
```
  cu `win_roots()` = `recursive: [C:\Windows\System32]`, `direct_files: [C:\Windows]`, `excluded: [C:\Windows\Temp]`.

- [ ] **Step 2: Rulează** `cargo test -p guardian-core` — Expected: FAIL (compilare: `TrustedRoots`, `start_marker` lipsesc).
- [ ] **Step 3: Implementează** `trusted.rs` și noua regulă din `decide`. Normalizarea pe Windows: litere mici, `/` → `\`, fără `\` final; pe Linux `Path::components()`.
- [ ] **Step 4: Rulează** `cargo test -p guardian-core` și `cargo build --workspace --all-features` (serverul folosește `ProcessInfo`? nu — confirmă că doar `guardian-core` se schimbă) — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-core specs/2026-11-17-rust-rewrite-design.md
git commit -m "core: protect system process names only under trusted roots"
```

---

### Task 2: Cadrele noi din proto

**Files:**
- Modify: `proto/guardian/v1/agent.proto`
- Create: `proto/guardian/v1/worker.proto`
- Modify: `crates/guardian-proto/build.rs` (adaugă `worker.proto`)
- Test: `crates/guardian-proto/tests/roundtrip.rs`

**Interfaces:**
- Produces (numele și numerele sunt contract):
  - în `agent.proto`: `message StoredState { AgentState state = 1; repeated TrustedConsole consoles = 2; repeated AuthorizedUser users = 3; int64 saved_at_unix = 4; uint64 computer_id = 5; }`
  - `worker.proto`:
    - `message WorkerHello { uint32 pid = 1; string session_id = 2; string user = 3; string worker_version = 4; }`
    - `message Ping { uint64 seq = 1; }`, `message Pong { uint64 seq = 1; }`
    - `message WorkerFrame { oneof msg { WorkerHello hello = 1; Pong pong = 2; } }` (worker → agent)
    - `message AgentToWorker { oneof msg { Ping ping = 1; } }` (agent → worker; etapa 4 adaugă variante)
  - Transport IPC: fiecare cadru = lungime `u32` big-endian + octeții protobuf; maxim **64 KiB**.

- [ ] **Step 1: Adaugă testul** `stored_state_and_worker_frames_roundtrip` în `roundtrip.rs` (un `StoredState` cu o stare, un `TrustedConsole`, un `AuthorizedUser`, `computer_id: 7`; un `WorkerFrame::Hello`; encode → decode → egal).
- [ ] **Step 2: Rulează** `cargo test -p guardian-proto` — Expected: FAIL (tipuri lipsă).
- [ ] **Step 3: Implementează** mesajele și `worker.proto` în `build.rs`.
- [ ] **Step 4: Rulează** `cargo test -p guardian-proto` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add proto crates/guardian-proto
git commit -m "proto: add the agent's stored state and the worker IPC frames"
```

---

### Task 3: `guardian-platform` — trait-ul, platforma falsă, stub-ul

**Files:**
- Create: `crates/guardian-platform/Cargo.toml` (feature `testing`), `src/lib.rs`, `src/fake.rs`, `src/unsupported.rs`
- Test: `crates/guardian-platform/tests/fake.rs`

**Interfaces:**
- Consumes: `guardian_core::enforce::{Os, ProcessInfo}`, `guardian_core::trusted::TrustedRoots`, `guardian_proto::v1::SystemInfo`.
- Produces:
  - `pub enum PlatformError { Unsupported, NotFound, AccessDenied, Changed /* PID refolosit */, Io(std::io::Error), Os(String) }` (thiserror)
  - `pub struct ProcessStart { pub pid: u32 }` — evenimentul minimal; detaliile se citesc cu `process_info(pid)`
  - `pub struct UserSession { pub id: String, pub user: String, pub uid: Option<u32>, pub env: Vec<(String, String)> }`
  - `#[async_trait] pub trait ProcessEventSource: Send { async fn next(&mut self) -> Result<ProcessStart, PlatformError>; }` — o eroare înseamnă „sursa a căzut" (sau, pentru `PlatformError::Changed`, „s-au pierdut evenimente, reconciliază")
  - `pub trait Platform: Send + Sync + 'static`:
    - `fn os(&self) -> Os`
    - `fn list_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError>`
    - `fn process_info(&self, pid: u32) -> Result<ProcessInfo, PlatformError>`
    - `fn kill(&self, p: &ProcessInfo) -> Result<(), PlatformError>` — omoară doar dacă procesul cu `p.pid` are încă același `name`, `exe_path` și `start_marker` (altfel `Changed`)
    - `fn process_events(&self) -> Result<Box<dyn ProcessEventSource>, PlatformError>`
    - `fn trusted_roots(&self) -> TrustedRoots`
    - `fn system_info(&self, agent_version: &str) -> SystemInfo` (hostname, `os` = `"linux"`/`"windows"`, versiune, adrese IPv4/IPv6 LAN fără loopback și link-local)
    - `fn active_session(&self) -> Result<Option<UserSession>, PlatformError>`
    - `fn spawn_in_session(&self, s: &UserSession, exe: &Path, args: &[String]) -> Result<u32, PlatformError>`
  - `pub fn current() -> Arc<dyn Platform>` (Linux / Windows / `Unsupported`)
  - `fake` (feature `testing`): `pub struct FakePlatform` cu `new(os: Os) -> Arc<FakePlatform>`, `spawn(&self, name: &str, exe: &str) -> u32` (adaugă un proces și emite `ProcessStart` pe sursă), `remove(&self, pid)`, `is_running(&self, pid) -> bool`, `kills(&self) -> Vec<ProcessInfo>`, `fail_events(&self, on: bool)` (sursa întoarce eroare / `process_events()` eșuează), `set_session(&self, Option<UserSession>)`, `spawned_in_session(&self) -> Vec<(String, PathBuf)>`, `set_roots(&self, TrustedRoots)`.

- [ ] **Step 1: Scrie testele** `tests/fake.rs`:
  - `fake_kill_checks_identity` — `spawn("game.exe", r"C:\g\game.exe")` → `kill(&info)` ok, `is_running == false`; un `ProcessInfo` cu `start_marker` diferit pentru un PID existent → `Err(Changed)`.
  - `fake_events_report_spawns` — `process_events()?.next().await` întoarce PID-ul procesului pornit după abonare.
  - `fake_events_can_fail` — `fail_events(true)` → `process_events()` întoarce `Err`.
  - `current_platform_reports_its_os` — `#[cfg(target_os = "linux")]` `current().os() == Os::Linux`; `#[cfg(windows)]` `Os::Windows`; pe alte OS-uri `current().list_processes()` → `Err(Unsupported)`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-platform --features testing` — Expected: FAIL.
- [ ] **Step 3: Implementează** trait-ul, `FakePlatform` (stare în `Mutex`, sursa de evenimente = `tokio::sync::mpsc`), `Unsupported`, `current()` cu `#[cfg]`. Modulele `linux` și `windows` sunt create goale (întorc `Unsupported`) și se umplu în Task 4–7 și 12.
- [ ] **Step 4: Rulează** testele — Expected: PASS. `cargo check --target x86_64-pc-windows-gnu -p guardian-platform` — Expected: compilează.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-platform Cargo.toml Cargo.lock
git commit -m "platform: define the platform trait with a fake for tests"
```

---

### Task 4: Linux — procese, omorâre sigură, informații de sistem

**Files:**
- Create/Modify: `crates/guardian-platform/src/linux/{mod,procs,sysinfo}.rs`
- Test: `crates/guardian-platform/tests/linux.rs` (`#![cfg(target_os = "linux")]`)

**Interfaces:**
- Consumes: Task 3.
- Produces: implementarea Linux pentru `list_processes`, `process_info`, `kill`, `trusted_roots`, `system_info`.

Reguli:
- `list_processes`: fiecare director numeric din `/proc`; `name` = `comm` (din `/proc/<pid>/stat`, între paranteze; câmpul poate conține spații și `)`, se ia până la **ultima** `)`), `exe_path` = `read_link("/proc/<pid>/exe")` (eroare → `None`; sufixul ` (deleted)` se taie), `start_marker` = câmpul 22 (`starttime`). Procesele care dispar în timpul citirii se sar.
- `kill`: recitește `stat` și `exe`; dacă `comm`, `exe` sau `starttime` diferă → `Err(Changed)`; altfel `nix::sys::signal::kill(Pid, SIGKILL)`; `ESRCH` → `NotFound`, `EPERM` → `AccessDenied`.
- `trusted_roots`: `recursive` = `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/opt/guardian-agent`; `excluded` = gol.
- `system_info`: hostname (`gethostname`), `if-addrs` fără loopback și link-local (`fe80::/10`, `169.254/16`).

- [ ] **Step 1: Scrie testele** `tests/linux.rs`:
  - `lists_this_process` — PID-ul curent e în listă, cu `exe_path == std::env::current_exe()` și `start_marker > 0`.
  - `kills_a_spawned_process_by_identity` — copiază `/bin/sleep` într-un `tempdir` ca `guardian-test-game`, îl pornește cu `30`, îl găsește în listă după PID, `kill` → procesul se termină în 2 s (`child.wait()`).
  - `refuses_to_kill_on_stale_start_marker` (**Review Focus 2**) — același proces, `ProcessInfo` cu `start_marker + 1` → `Err(Changed)`, procesul încă rulează.
  - `comm_with_parenthesis_is_parsed` — test unitar pe funcția de parsare `parse_stat("123 (a) b)) R 1 …")` → `comm == "a) b)"`, `starttime` corect.
  - `system_info_has_hostname_and_os` — `os == "linux"`, `hostname` nevid, nicio adresă `127.*`/`::1`.
- [ ] **Step 2: Rulează** testele în Linux: `podman machine start` apoi
  `podman run --rm -v "$PWD":/w -w /w -e CARGO_TARGET_DIR=/w/target/linux docker.io/library/rust:1.95 cargo test -p guardian-platform --features testing --test linux` — Expected: FAIL (`Unsupported`).
- [ ] **Step 3: Implementează** modulele Linux (fără `unsafe`; `nix` cu feature-urile `signal`, `socket`, `uio`, `user`).
- [ ] **Step 4: Rulează** aceeași comandă — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-platform Cargo.toml Cargo.lock
git commit -m "platform: list and kill Linux processes by identity"
```

---

### Task 5: Linux — evenimente de proces prin proc connector

**Files:**
- Create: `crates/guardian-platform/src/linux/events.rs`
- Test: `crates/guardian-platform/tests/linux.rs`

**Interfaces:**
- Produces: `process_events()` pe Linux = o sursă care deschide `socket(AF_NETLINK, SOCK_DGRAM, NETLINK_CONNECTOR)`, se leagă la grupul `CN_IDX_PROC` (1), trimite `PROC_CN_MCAST_LISTEN` și emite `ProcessStart{pid}` pentru evenimentele `PROC_EVENT_EXEC` (0x2). Citirea e într-un thread dedicat care trimite pe un `mpsc` (socket-ul e blocant); `next()` citește din canal.

Reguli:
- Mesajul de abonare: `nlmsghdr` (16 octeți, `NLMSG_DONE`) + `cn_msg` (`idx=1`, `val=1`, seq, ack, len=4, flags) + `u32` op = 1; octeții se compun manual (little-endian/ordinea host), fără `unsafe`.
- Parsarea: `nlmsghdr` → `cn_msg` (20 de octeți) → `proc_event` (`what: u32`, `cpu: u32`, `timestamp: u64`, apoi pentru EXEC `process_pid: u32`, `process_tgid: u32`); se emite `tgid`.
- Fără `CAP_NET_ADMIN` (`EPERM`) → `process_events()` întoarce `Err(AccessDenied)` (agentul trece singur pe scanare).
- `ENOBUFS` la citire → `next()` întoarce `Err(PlatformError::Changed)` (evenimente pierdute) și sursa rămâne utilizabilă.

- [ ] **Step 1: Scrie testele**:
  - `parses_exec_event` — test unitar pe `parse_proc_event(&bytes)` cu un buffer construit în test (what = 2, tgid = 4242) → `Some(4242)`; `what = 1` (FORK) → `None`; buffer scurt → `None`.
  - `subscribe_message_layout` — `subscribe_msg(seq)` are 40 de octeți, `nlmsg_len == 40`, `cn idx/val == 1/1`, op == 1.
  - `exec_events_are_reported_when_root` — `#[ignore]` (rulează doar cu `--ignored` ca root): pornește sursa, rulează `/bin/true`, primește PID-ul în 2 s.
  - `non_root_gets_access_denied` — dacă `geteuid() != 0`, `process_events()` → `Err(AccessDenied)`.
- [ ] **Step 2: Rulează** în container (comanda din Task 4, cu `--test linux`) — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** în container, apoi ca root: `podman run --rm --cap-add NET_ADMIN … cargo test -p guardian-platform --features testing --test linux -- --include-ignored` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-platform
git commit -m "platform: receive Linux exec events from the proc connector"
```

---

### Task 6: Windows — procese, omorâre prin handle, informații de sistem

**Files:**
- Create: `crates/guardian-platform/src/windows/{mod,procs,sysinfo}.rs`, `src/windows/ffi_procs.rs`
- Test: `crates/guardian-platform/tests/windows.rs` (`#![cfg(windows)]`)

**Interfaces:**
- Produces: implementarea Windows pentru `list_processes`, `process_info`, `kill`, `trusted_roots`, `system_info`.

Reguli:
- Listare: `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` → nume (`szExeFile`) și PID; pentru fiecare, `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW` (cale Win32) și `GetProcessTimes` (`CreationTime` → `start_marker`); dacă deschiderea eșuează, `exe_path = None`, `start_marker = 0`.
- `kill`: `OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION)`, apoi **pe același handle** se recitesc calea și `CreationTime`; diferă → `Changed`; altfel `TerminateProcess(h, 1)`. `ERROR_ACCESS_DENIED` → `AccessDenied`, `ERROR_INVALID_PARAMETER` la deschidere → `NotFound`. Handle-urile se închid printr-un wrapper RAII.
- `trusted_roots` (din `%SystemRoot%`, `%ProgramFiles%`, `%ProgramData%`, citite din mediu, cu valorile implicite `C:\Windows`, `C:\Program Files`, `C:\ProgramData`):
  - `recursive`: `%SystemRoot%\System32`, `%SystemRoot%\SysWOW64`, `%SystemRoot%\SystemApps`, `%SystemRoot%\ImmersiveControlPanel`, `%ProgramFiles%\WindowsApps`, `%ProgramFiles%\PowerShell`, `%ProgramFiles%\Windows Defender`, `%ProgramData%\Microsoft\Windows Defender\Platform`, `%ProgramFiles%\Guardian`
  - `direct_files`: `%SystemRoot%`
  - `excluded` (directoare în care userii obișnuiți pot scrie): `%SystemRoot%\System32\spool`, `%SystemRoot%\System32\Tasks`, `%SystemRoot%\System32\com\dmp`, `%SystemRoot%\System32\Microsoft\Crypto`, `%SystemRoot%\SysWOW64\Tasks`, `%SystemRoot%\SysWOW64\com\dmp`, `%SystemRoot%\Temp`, `%SystemRoot%\Tracing`
- `system_info`: hostname, `os = "windows"`, adrese prin `if-addrs`.
- Tot FFI-ul stă în `ffi_procs.rs` (`#![allow(unsafe_code)]` la nivel de modul, câte un `// SAFETY:` pe bloc); restul modulului e cod sigur.

- [ ] **Step 1: Scrie testele** `tests/windows.rs`:
  - `lists_this_process` — PID-ul curent, cale = `current_exe()` (comparație fără majuscule), `start_marker > 0`.
  - `kills_a_spawned_process_by_identity` — copiază `C:\Windows\System32\PING.EXE` în `tempdir` ca `guardian-test-game.exe`, pornit cu `-n 30 127.0.0.1`, `kill` → se termină în 2 s.
  - `refuses_to_kill_on_stale_start_marker` — `start_marker + 1` → `Err(Changed)`, procesul rulează.
  - `trusted_roots_cover_system32_but_not_temp` — `contains(Os::Windows, C:\Windows\System32\svchost.exe)` și `!contains(C:\Windows\Temp\svchost.exe)`.
- [ ] **Step 2: Verifică local** `cargo check --target x86_64-pc-windows-gnu -p guardian-platform --tests` și `--target i686-pc-windows-gnu` (comanda rustup din „Înainte de a începe") — Expected: compilează după implementare; testele rulează în CI (Task 15).
- [ ] **Step 3: Implementează** (crate-ul `windows` 0.62 cu feature-urile `Win32_Foundation`, `Win32_System_Diagnostics_ToolHelp`, `Win32_System_Threading`, `Win32_System_ProcessStatus`).
- [ ] **Step 4: Rulează** check-urile pe ambele ținte — Expected: fără erori și fără warning-uri (`cargo clippy` cu aceeași țintă, dacă clippy-driver funcționează cu toolchain-ul rustup; altfel doar `check`).
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-platform Cargo.toml Cargo.lock
git commit -m "platform: list and kill Windows processes through verified handles"
```

---

### Task 7: Windows — evenimente de proces prin ETW

**Files:**
- Create: `crates/guardian-platform/src/windows/events.rs`
- Test: `crates/guardian-platform/tests/windows.rs`

**Interfaces:**
- Produces: `process_events()` pe Windows = o sesiune ETW user-mode (`ferrisetw::UserTrace`) cu providerul **Microsoft-Windows-Kernel-Process** (`22FB2CD6-0E7B-422B-A0C7-2FAD1FD0E716`), keyword `WINEVENT_KEYWORD_PROCESS` (`0x10`), event id **1** (ProcessStart), câmpul `ProcessID` → `ProcessStart{pid}`. Numele sesiunii: `GuardianAgentProcessTrace`; o sesiune rămasă de la o rulare anterioară cu același nume se oprește înainte de pornire. Callback-ul ETW trimite pe un `mpsc`; oprirea sesiunii (drop) sau eroarea → `next()` întoarce `Err`.
- Fără drepturi de administrator → `Err(AccessDenied)`.

- [ ] **Step 1: Scrie testul** `etw_reports_new_processes` — `#[ignore]` local (în CI Windows rulează cu `--include-ignored`, runner-ul e admin): pornește sursa, apoi `cmd /c exit`, primește PID-ul copilului în 3 s.
- [ ] **Step 2: Verifică local** `cargo check` pe ambele ținte Windows — Expected: FAIL înainte de implementare (`events` lipsește).
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Verifică local** din nou — Expected: compilează; testul rulează în CI.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-platform Cargo.toml Cargo.lock
git commit -m "platform: receive Windows process-start events through ETW"
```

---

### Task 8: `guardian-agent` — config și magazinul de fișiere

**Files:**
- Create: `crates/guardian-agent/Cargo.toml`, `src/lib.rs` (doar module), `src/config.rs`, `src/store.rs`
- Test: `crates/guardian-agent/tests/store.rs`

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize)] pub struct AgentConfig { pub server: String /* wss://host:port */, pub cert_sha256: Option<String>, pub computer_id: Option<u64> }` cu `load(path) -> Result<AgentConfig, StoreError>`, `save(&self, path) -> Result<(), StoreError>` (atomic, 0600 pe Unix)
  - `pub struct AgentPaths { pub config: PathBuf, pub data: PathBuf }` cu `system_default() -> AgentPaths` (valorile din Global Constraints) și `under(dir: &Path) -> AgentPaths` (teste: `dir/agent.toml`, `dir/data/`); metode `key()`, `state()`, `seen()`, `whitelist()`, `log()`
  - `pub fn atomic_write(path: &Path, data: &[u8], private: bool) -> std::io::Result<()>`
  - `pub fn load_or_create_key(path: &Path) -> Result<KeyPair, StoreError>` (32 de octeți; altă lungime → eroare, **nu** se regenerează — schimbarea cheii ar rupe identitatea)
  - `pub fn load_state(path) -> Option<StoredState>` (lipsă sau coruptă → `None`, corupția cu `warn!`); `pub fn save_state(path, &StoredState) -> Result<(), StoreError>`
  - `pub struct SeenIds` cu `load(path) -> SeenIds`, `insert(&mut self, id: &str) -> bool` (`false` dacă exista), `save(&self, path)`; păstrează ultimele **1000**, cele mai vechi ies primele
  - `pub fn load_whitelist(path) -> BTreeSet<AppName>` (lipsă → gol; linii goale și `#` ignorate; nume invalide sărite cu `warn!`)
  - `pub fn default_state() -> StoredState` — `AgentState{version: 0, effective_profile_id: 0, room_id: 0, profiles: [Profile{id: 0, name: "Lecție", mode: BLACKLIST, apps: []}]}`

- [ ] **Step 1: Scrie testele** `tests/store.rs`:
  - `config_roundtrips_and_is_private` — save/load egal; pe Unix mod `0o600`.
  - `key_is_created_once_and_kept` — două apeluri → aceeași cheie publică; fișier de 31 de octeți → `Err`, fișierul rămâne neatins.
  - `state_roundtrips` — save/load egal.
  - `corrupt_state_is_ignored` (**Review Focus 4**) — `state.pb` cu octeți aleatori și unul trunchiat (primii 5 octeți dintr-o stare validă) → `load_state == None`, fără panică.
  - `atomic_write_leaves_no_temp_files` — după 50 de scrieri, directorul conține doar fișierul țintă.
  - `seen_ids_dedupe_and_cap` — `insert("a") == true`, `insert("a") == false`; după 1001 id-uri, primul a ieșit; save/load păstrează ordinea.
  - `whitelist_parsing` — `"# x\n\nprinter-tool.exe\nC:\\bad\n  Word.EXE \n"` → `{"printer-tool.exe", "word.exe"}`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-agent --test store` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-agent Cargo.toml Cargo.lock
git commit -m "agent: store config, key, state and seen commands atomically"
```

---

### Task 9: Motorul de enforcement și supervizorul hibrid

**Files:**
- Create: `crates/guardian-agent/src/engine.rs`
- Test: `crates/guardian-agent/tests/engine.rs`

**Interfaces:**
- Consumes: `decide`, `TrustedRoots`, `Platform`, `ProcessEventSource`, `load_whitelist`.
- Produces:
  - `pub struct Policy { pub profile: Profile, pub version: u64 }` cu `pub fn from_state(&AgentState) -> Policy` (profilul cu `effective_profile_id`; dacă nu se găsește în listă → profilul implicit „Lecție" gol, cu `warn!`)
  - `pub struct EngineConfig { pub reconcile_every: Duration /* 10 s */, pub fallback_poll: Duration /* 1 s */, pub retry_events_every: Duration /* 60 s */ }` cu `Default`
  - `pub struct Engine` cu `pub fn spawn(platform: Arc<dyn Platform>, policy: Policy, whitelist_path: PathBuf, cfg: EngineConfig) -> Engine`, `pub fn set_policy(&self, p: Policy)` (declanșează o reconciliere imediată), `pub fn mode(&self) -> EngineMode`, `pub fn stats(&self) -> EngineStats { kills: u64, reconciles: u64 }`, `pub async fn shutdown(self)`
  - `pub enum EngineMode { Events, Polling }`

Comportament:
- La pornire: reconciliere completă, apoi încearcă `process_events()`; reușită → `Events`, eșec → `Polling` (scanare la `fallback_poll`) cu `warn!("process events unavailable ({e}); scanning every 1s")`, și reîncearcă sursa la `retry_events_every`.
- Pe `Events`: pentru fiecare `ProcessStart{pid}` → `process_info(pid)` → `decide` → `kill` dacă `Kill`. Eroarea `Changed` a sursei → reconciliere imediată, rămâne pe `Events`. Orice altă eroare → `Polling` cu `warn!`.
- Reconciliere: `list_processes` → `decide` pe fiecare → `kill`; la fiecare `reconcile_every`, la `set_policy` și la schimbarea fișierului `whitelist.txt` (verificat după `mtime` la fiecare reconciliere).
- Procesul agentului însuși și worker-ul nu se omoară niciodată, chiar dacă decizia ar spune `Kill` (verificare după PID-ul propriu și PID-ul worker-ului curent, setat cu `pub fn set_worker_pid(&self, Option<u32>)`).
- `kill` care întoarce `Changed`/`NotFound` nu e o eroare (`debug!`); `AccessDenied` → `warn!` o singură dată per (nume, cale).
- Platformă nesuportată (`list_processes` → `Unsupported`, de exemplu pe macOS la dezvoltare): un singur `warn!("process control is not supported on this OS")`, motorul rămâne inactiv (fără buclă de reîncercări), iar restul agentului funcționează.

- [ ] **Step 1: Scrie testele** `tests/engine.rs` (cu `FakePlatform`, `EngineConfig` scurtat: reconcile 200 ms, fallback 50 ms, retry 300 ms):
  - `event_triggers_kill_in_blacklist` — profil `blacklist ["game.exe"]`; `fake.spawn("game.exe", r"C:\g\game.exe")` → omorât în 200 ms; `notepad.exe` rămâne.
  - `reconcile_kills_processes_started_before` — proces pornit înainte de `Engine::spawn` → omorât la pornire.
  - `set_policy_reapplies_immediately` — `free` → nimic omorât; `set_policy(whitelist [])` → procesele nesigure omorâte în 100 ms, cele din rădăcinile de încredere (`set_roots`) cu nume protejat rămân.
  - `falls_back_to_polling_and_recovers` (**Review Focus 5**) — `fail_events(true)` înainte de pornire → `mode() == Polling`; un proces interzis e omorât în ≤ 150 ms; `fail_events(false)` → după ≤ 500 ms `mode() == Events`.
  - `source_dying_midway_switches_to_polling` — pornit pe `Events`; `fail_events(true)` → `Polling`, procesul interzis pornit după cădere e totuși omorât.
  - `local_whitelist_counts_in_exam_and_reloads` — `whitelist []`, `whitelist.txt` scris în timpul rulării cu `printer-tool.exe` → un proces `printer-tool.exe` pornit după reîncărcare rămâne.
  - `unsupported_platform_stays_quiet` — o platformă de test care întoarce `Unsupported` la tot → `Engine` pornește, `stats().reconciles` nu crește peste 1 în 500 ms.
  - `never_kills_self_or_worker` — un proces fals cu PID-ul agentului (`std::process::id()`) și unul cu PID-ul setat prin `set_worker_pid` nu apar în `kills()`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-agent --test engine` — Expected: FAIL.
- [ ] **Step 3: Implementează** (un task tokio; `process_info`/`list_processes`/`kill` sunt sincrone → `spawn_blocking`).
- [ ] **Step 4: Rulează** de 3 ori la rând — Expected: PASS de fiecare dată.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-agent
git commit -m "agent: enforce profiles from process events with a polling fallback"
```

---

### Task 10: Conexiunea cu serverul

**Files:**
- Create: `crates/guardian-agent/src/conn.rs`
- Modify: `crates/guardian-server/src/testing.rs` (`TestServer::restart_same_port()`)
- Test: `crates/guardian-agent/tests/conn.rs` (dev-dependency `guardian-server` cu `testing`)

**Interfaces:**
- Consumes: `guardian_client::ws_connector`, `KeyPair::sign_challenge`, `AgentConfig`, `SeenIds`, `PROTOCOL_VERSION`, cadrele din `agent.proto`.
- Produces:
  - `pub async fn enroll(server: &str, pin: Option<&str>, code: &str, key: &KeyPair, info: SystemInfo) -> Result<u64, ConnError>` (deschide WS, trimite `Enroll`, primește `Enrolled`, închide; nu trimite `Hello`)
  - `pub enum ConnError { Rejected { code: String, message: String }, Io(String), Protocol(String) }` — `Rejected` cu codul serverului (`invalid_code`, `unauthorized`, `revoked`, `protocol`, `rate_limited`)
  - `pub enum ConnEvent { Online { welcome: Welcome }, State(AgentState), Trust(Welcome), Command(CommandEnvelope), Offline { reason: String }, Revoked }`
  - `pub struct Connection` cu `pub fn spawn(cfg: ConnConfig, key: KeyPair, info: SystemInfo, events: mpsc::Sender<ConnEvent>) -> Connection`, `pub fn send_result(&self, r: CommandResult)`, `pub fn online_since(&self) -> Option<Instant>`, `pub fn offline_for(&self) -> Option<Duration>`, `pub async fn shutdown(self)`
  - `pub struct ConnConfig { pub server: String, pub pin: Option<String>, pub computer_id: u64, pub capabilities: Vec<String>, pub backoff_min: Duration /* 1 s */, pub backoff_max: Duration /* 60 s */ }`

Comportament:
- Bucla: conectare → `Hello{computer_id, PROTOCOL_VERSION, capabilities, info}` → semnează `Challenge` cu `AuthContext::Agent` → `Welcome` → `ConnEvent::Online`. Cadrele următoare: `State` → `ConnEvent::State`, `TrustUpdate` → `ConnEvent::Trust`, `Command` → `ConnEvent::Command`, `ServerError{revoked}` → `ConnEvent::Revoked` și bucla **se oprește** (nu se mai reconectează). `ServerError{unauthorized}` → `Offline` și reconectare cu backoff (cheia poate fi rotită de admin în etapa 3 — nu e terminal).
- Backoff: `min(backoff_max, backoff_min * 2^n)` cu jitter uniform ±20 %; resetat după `Welcome`.
- Răspunde la ping (tungstenite o face când citim) și trimite propriul ping la 20 s; fără niciun cadru de la server 60 s → închide și reconectează.
- Decizia despre `version`, deduplicarea și răspunsul `unsupported` **nu** sunt aici (le face `Agent`, Task 11): conexiunea doar transportă.

- [ ] **Step 1: Scrie testele** `tests/conn.rs`:
  - `enrolls_and_goes_online` — `TestServer::start()`, cod de sală; `enroll(..)` → id; `Connection::spawn` → `Online` cu `welcome.state.room_id` = sala; `ListComputers` arată `online: true`.
  - `bad_code_is_rejected` — `enroll` cu `"ABCD2345"` → `Rejected{code: "invalid_code", ..}`.
  - `reconnects_after_server_restart` — `Online`, apoi `s.restart_same_port()` → `Offline` apoi din nou `Online` în ≤ 5 s (backoff_min 100 ms în test).
  - `revocation_stops_reconnecting` — `RevokeComputer` → `Revoked`; în următoarele 2 s nu mai apare niciun `Online`.
  - `state_push_is_forwarded` — `SetRoomProfile` → `ConnEvent::State` cu profilul nou.
  - `works_over_tls_with_pin` — `TestServer::start_tls()`, `wss://` cu pin → `Online`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-agent --test conn` — Expected: FAIL.
- [ ] **Step 3: Implementează** `conn.rs` și `TestServer::restart_same_port()` (oprește serverul și pornește pe același `local_addr`, cu reîncercări de bind timp de 2 s).
- [ ] **Step 4: Rulează** — Expected: PASS; `cargo test -p guardian-server --all-features` rămâne verde.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-agent crates/guardian-server/src/testing.rs Cargo.lock
git commit -m "agent: enroll and keep the WebSocket connection with backoff"
```

---

### Task 11: `Agent` — starea, comenzile și funcționarea offline

**Files:**
- Modify: `crates/guardian-agent/src/lib.rs`
- Test: `crates/guardian-agent/tests/agent.rs`

**Interfaces:**
- Consumes: Task 8–10.
- Produces:
  - `pub struct AgentOptions { pub paths: AgentPaths, pub platform: Arc<dyn Platform>, pub engine: EngineConfig, pub backoff_min: Duration }` (Task 12 adaugă `worker: Option<WorkerOptions>`)
  - `pub async fn run(opts: AgentOptions) -> Result<AgentHandle, AgentError>` — cere config cu `computer_id` (altfel `AgentError::NotEnrolled`)
  - `pub struct AgentHandle` cu `status(&self) -> AgentStatus`, `async fn shutdown(self)`
  - `pub struct AgentStatus { pub online: bool, pub offline_for: Option<Duration>, pub state_version: u64, pub profile_id: i64, pub profile_name: String, pub engine_mode: EngineMode, pub kills: u64, pub revoked: bool }` (Task 12 adaugă `worker_connected: bool`)
  - `pub const CAPABILITIES: &[&str] = &[];`

Comportament:
1. Pornire: încarcă starea salvată (sau `default_state()`), pornește `Engine` cu ea **înainte** de conexiune, apoi `Connection`.
2. `Online{welcome}` și `State(st)`: aplică doar dacă `st.version > curentă` (sau starea curentă e cea implicită, versiunea 0); salvează `StoredState` (cu `consoles`, `users`, `computer_id`), apoi `engine.set_policy`. `Trust(w)`: actualizează și salvează `consoles`/`users`, fără să atingă politica.
3. `Command(env)`: dacă `SeenIds::insert(env.id)` e `false` → ignorat; altfel `send_result(CommandResult{command_id, status: RESULT_UNSUPPORTED, detail: "not supported by this agent version"})` și `SeenIds::save`. (Comenzile din `welcome.pending` trec pe același drum.)
4. `Revoked`: `warn!`, `status().revoked = true`, politica rămâne ultima aplicată (fail-secure).
5. `status().online` = `true` între `Online` și `Offline`; `offline_for` = timpul de la ultima pierdere a conexiunii autentificate (sau de la pornire).

- [ ] **Step 1: Scrie testele** `tests/agent.rs` (`FakePlatform`, `TestServer`, dir temporar cu `AgentPaths::under`):
  - `enrolled_agent_applies_room_profile_and_kills` — înrolare în sală; adminul pune în „Lecție" `game.exe`; agentul pornit; `fake.spawn("game.exe", …)` → omorât în 1 s; `state.pb` conține versiunea curentă.
  - `boots_offline_with_saved_state` (**Review Focus 3**) — rulează o dată online (starea salvată, profil blacklist cu `game.exe`), `shutdown`; serverul oprit; agentul repornit cu aceleași fișiere → `status().online == false`, `game.exe` omorât în 1 s.
  - `first_boot_without_server_uses_default_lesson` — config cu un `computer_id` și server inexistent (`wss://127.0.0.1:1`), fără `state.pb` → `profile_name == "Lecție"`, nimic omorât.
  - `older_state_is_ignored` — după `Online` (versiunea v), testul trimite prin `s.handle.state().hub.send(id, State{version: v - 1, effective = Liber})` → `status().profile_id` rămâne neschimbat; un `State{version: v + 1}` se aplică.
  - `resent_command_is_answered_once` — testul trimite de două ori același `Command{id: X}` prin `hub.send` → serverul vede exact **un** `CommandResult` (`GetCommand` după `batcher.flush` arată `unsupported`; numărul de rezultate primite de server se verifică printr-un `Watch` care vede exact un `ComputerResultEvent` non-`delivered` pentru X). După repornirea agentului, același id trimis din nou → tot niciun al doilea rezultat.
  - `revoked_agent_keeps_last_policy` — `RevokeComputer` → `status().revoked`, `game.exe` încă omorât.
- [ ] **Step 2: Rulează** `cargo test -p guardian-agent --test agent` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS (de 3 ori la rând).
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-agent
git commit -m "agent: apply server state, dedupe commands and keep enforcing offline"
```

---

### Task 12: Worker-ul — IPC, binarul schelet și managerul

**Files:**
- Create: `crates/guardian-platform/src/ipc.rs`, `src/windows/ffi_pipe.rs`, `src/linux/session.rs`, `src/windows/session.rs`, `src/windows/ffi_session.rs`
- Create: `crates/guardian-agent/src/worker.rs`; Modify: `crates/guardian-agent/src/lib.rs` (`AgentOptions.worker`, `AgentStatus.worker_connected`)
- Create: `crates/guardian-worker/Cargo.toml`, `src/lib.rs`, `src/main.rs`
- Test: `crates/guardian-platform/tests/ipc.rs`, `crates/guardian-agent/tests/worker.rs`, `crates/guardian-worker/tests/peer.rs`

**Interfaces:**
- Consumes: `Platform::{active_session, spawn_in_session}`, `worker.proto`, `Engine::set_worker_pid`.
- Produces:
  - `guardian_platform::ipc` (dependență comună a agentului și a worker-ului; `guardian-worker` nu depinde de `guardian-agent`):
    - `pub enum IpcEndpoint { Unix(PathBuf), Pipe(String) }` cu `system_default()` (`/run/guardian-agent/worker.sock` / `\\.\pipe\guardian-agent-worker`)
    - `pub async fn write_frame<W: AsyncWrite + Unpin, M: prost::Message>(w: &mut W, m: &M) -> io::Result<()>` și `pub async fn read_frame<R: AsyncRead + Unpin, M: prost::Message + Default>(r: &mut R) -> io::Result<M>` (lungime u32 BE, maxim 64 KiB, altfel `InvalidData`)
    - `pub struct IpcListener` cu `bind(&IpcEndpoint) -> io::Result<IpcListener>`, `accept(&self) -> io::Result<(IpcStream, u32 /* PID-ul peer-ului */)>`; `pub async fn connect(&IpcEndpoint) -> io::Result<IpcStream>`; `IpcStream: AsyncRead + AsyncWrite + Unpin + Send`
    - Unix: directorul socket-ului 0711, socket-ul 0666 (worker-ul rulează ca user), PID din `UnixStream::peer_cred()`; un socket vechi rămas de la o rulare anterioară se șterge la `bind`.
    - Windows: `tokio::net::windows::named_pipe::ServerOptions` cu `first_pipe_instance(true)` la prima instanță; PID din `GetNamedPipeClientProcessId` (`ffi_pipe.rs`).
  - `guardian_worker::run(endpoint: IpcEndpoint) -> Result<(), WorkerError>`: se conectează (reîncercări 10 s, apoi `WorkerError::Connect`), trimite `WorkerHello{pid, session_id: env GUARDIAN_SESSION, user: env USER/USERNAME, worker_version}`, răspunde `Pong{seq}` la fiecare `Ping{seq}`, întoarce `Ok` când agentul închide. `main.rs`: `guardian-worker --endpoint <path|pipe>`, cod 0 / 2.
  - `guardian_agent::worker`: `pub struct WorkerOptions { pub exe: PathBuf, pub endpoint: IpcEndpoint, pub check_every: Duration /* 2 s */, pub ping_every: Duration /* 5 s */ }`; `pub struct WorkerManager` cu `spawn(platform: Arc<dyn Platform>, opts: WorkerOptions, engine: Engine) -> WorkerManager`, `connected(&self) -> bool`, `pid(&self) -> Option<u32>`, `async fn shutdown(self)`. `Engine` devine `Clone` (handle) pentru asta.
  - Platformă, `active_session()`:
    - Linux: `loginctl list-sessions --no-legend`, apoi `loginctl show-session <id> -p Active -p Remote -p Type -p Class -p Name -p User -p Display`; prima sesiune cu `Active=yes`, `Remote=no`, `Class=user`, `Type` ∈ {`x11`, `wayland`}; `env` = `XDG_RUNTIME_DIR=/run/user/<uid>`, `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/<uid>/bus`, `DISPLAY` (dacă e setat), `WAYLAND_DISPLAY=wayland-0` (pentru Wayland), `GUARDIAN_SESSION=<id>`, `USER=<nume>`. Parsarea e în `pub fn parse_show_session(text: &str) -> Option<UserSession>`.
    - Windows: `WTSGetActiveConsoleSessionId`; `0xFFFFFFFF` sau user gol (`WTSQuerySessionInformationW(WTSUserName)`) → `None`.
  - `spawn_in_session`:
    - Linux: `std::process::Command` cu `uid`/`gid` (`CommandExt`; gid-ul principal din `nix::unistd::User::from_uid`), `env_clear()` + `env`, `current_dir("/")`, stdin/stdout/stderr la `/dev/null`; întoarce PID-ul.
    - Windows: `WTSQueryUserToken` → `DuplicateTokenEx(TokenPrimary)` → `CreateEnvironmentBlock` → `CreateProcessAsUserW` cu `lpDesktop = winsta0\default`, `CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW`; FFI doar în `ffi_session.rs`.

Comportament `WorkerManager`:
- La fiecare `check_every`: fără sesiune activă și worker conectat → închide conexiunea (worker-ul iese singur). Cu sesiune activă și (niciun worker sau altă sesiune decât la pornirea lui) → `spawn_in_session(exe, ["--endpoint", <endpoint>])` și ține minte PID-ul.
- Acceptă doar conexiunea al cărei PID de peer e PID-ul întors de `spawn_in_session`; orice altă conexiune se închide imediat cu `warn!` (spec §10.4). Primul cadru trebuie să fie `WorkerHello` în 5 s.
- `Ping` la `ping_every`; fără `Pong` în `2 × ping_every` → închide; la verificarea următoare se pornește alt worker.
- `engine.set_worker_pid(...)` la fiecare schimbare; `AgentStatus.worker_connected` vine de aici.

- [ ] **Step 1: Scrie testele**:
  - `crates/guardian-platform/tests/ipc.rs`: `frames_roundtrip_and_reject_oversize` (peste `tokio::io::duplex`; prefix de 70 000 → `InvalidData`); `#[cfg(unix)] listener_reports_peer_pid` (`bind` într-un `tempdir`, `connect` din același proces → PID == `std::process::id()`); `#[cfg(unix)] stale_socket_is_replaced` (un fișier rămas la cale nu împiedică `bind`).
  - `crates/guardian-worker/tests/peer.rs` (`#![cfg(unix)]`, aici `CARGO_BIN_EXE_guardian-worker` există): `real_worker_process_says_hello` — `IpcListener` într-un `tempdir`, pornește binarul cu `--endpoint`, `accept` → PID-ul peer-ului == PID-ul copilului, primul cadru `WorkerHello` cu același PID, `Ping{7}` → `Pong{7}`; închiderea conexiunii → copilul iese cu cod 0 în 2 s.
  - `crates/guardian-agent/tests/worker.rs` (`#![cfg(unix)]`, `FakePlatform` al cărui `spawn_in_session` pornește `guardian_worker::run(endpoint)` într-un task tokio și întoarce PID-ul configurat prin `set_spawn_pid`):
    - `manager_spawns_worker_in_session_and_accepts_it` — `set_session(Some(..))`, `set_spawn_pid(std::process::id())` → `connected()` în 1 s, `pid() == Some(std::process::id())`.
    - `foreign_peer_is_rejected` — `set_spawn_pid(999_999)` (worker-ul din task are PID-ul testului) → conexiunea e închisă, `connected()` rămâne `false` 1 s.
    - `no_session_means_no_worker` — `set_session(None)` → `spawned_in_session()` gol după 3 verificări.
    - `dead_worker_is_restarted` — task-ul worker-ului e oprit → în ≤ `2 × check_every` `spawned_in_session().len() == 2`.
  - `crates/guardian-platform/src/linux/session.rs`, test unitar `parses_wayland_session` (`#[cfg(target_os = "linux")]`) pe un text `loginctl show-session` cu `Active=yes`, `Type=wayland`, `User=1000` → `env` conține `WAYLAND_DISPLAY=wayland-0` și `XDG_RUNTIME_DIR=/run/user/1000`.
  - `FakePlatform` primește `set_spawn_pid(u32)` și un hook `on_spawn(Box<dyn Fn(&UserSession, &Path, &[String]) + Send + Sync>)` (Task 3 se extinde aici).
- [ ] **Step 2: Rulează** `cargo test -p guardian-platform --features testing --test ipc`, `cargo test -p guardian-worker`, `cargo test -p guardian-agent --test worker` — Expected: FAIL.
- [ ] **Step 3: Implementează** IPC, binarul, managerul și sesiunile pe ambele OS-uri.
- [ ] **Step 4: Rulează** testele pe macOS, în containerul Linux, și `cargo check` pe ambele ținte Windows — Expected: PASS / compilează.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-platform crates/guardian-agent crates/guardian-worker Cargo.toml Cargo.lock
git commit -m "agent: start the worker in the user session and talk to it over local IPC"
```

---

### Task 13: Binarul `guardian-agent`, serviciul Windows și instalarea pe Linux

**Files:**
- Create: `crates/guardian-agent/src/main.rs`, `src/service_windows.rs`, `packaging/agent-linux-v3/guardian-agent.service`, `install.sh`, `uninstall.sh`
- Test: `crates/guardian-agent/tests/binary.rs`

**Interfaces:**
- Produces: binarul `guardian-agent` (clap, fiecare opțiune și din env `GUARDIAN_AGENT_*`):
  - opțiuni globale: `--config <PATH>` (implicit `AgentPaths::system_default().config`), `--data-dir <DIR>`, `--log <FILTER>` (implicit `info`)
  - `enroll --server <wss://host:port> --code <C> [--cert-sha256 <HEX>]` — generează/încarcă cheia, apelează `conn::enroll` cu `platform.system_info(..)`, scrie configul cu `computer_id`; dacă e deja înrolat → eroare „already enrolled (computer N); delete agent.key and the config to re-enroll" și cod 1
  - `run` — rulează în prim-plan până la SIGINT/SIGTERM (Ctrl+C pe Windows)
  - `status` — citește configul, `state.pb`, `seen.txt` și afișează: server, `computer_id`, versiunea și profilul salvat, numărul de id-uri văzute (fără conexiune la agentul care rulează — IPC de status vine în etapa 3)
  - Windows: `service install|uninstall|run` — serviciul `GuardianAgent`, „Guardian Agent", pornire automată, LocalSystem, recuperare: repornire după 5 s (de 3 ori); `install` aplică ACL-ul protejat pe `%ProgramData%\Guardian Agent`; ca serviciu, log în `agent.log`; `Running` raportat după ce starea salvată e aplicată
- `guardian-agent.service`: `ExecStart=/opt/guardian-agent/guardian-agent run`, `Restart=always`, `RestartSec=2`, rulează ca root (trebuie să omoare procesele oricui și să citească netlink), `StateDirectory=guardian-agent` (0700), `ConfigurationDirectory=guardian-agent` (0700), `RuntimeDirectory=guardian-agent` (0711), `ProtectHome=read-only`, `PrivateTmp=yes`, `NoNewPrivileges=yes`, `ProtectSystem=full`. `CAP_NET_ADMIN` + `CAP_KILL` + `CAP_SETUID` + `CAP_SETGID` în `CapabilityBoundingSet` (pornirea worker-ului ca user).
- `install.sh <wss://server:port> <CODE> [CERT_SHA256]` (root): copiază `guardian-agent` și `guardian-worker` în `/opt/guardian-agent/`, rulează `enroll` dacă nu e deja înrolat, instalează și pornește unitatea; rulat de două ori nu reînrolează. `uninstall.sh` oprește și șterge unitatea și binarele; păstrează `/etc/guardian-agent` și `/var/lib/guardian-agent` dacă nu primește `--purge`.

- [ ] **Step 1: Scrie testul** `tests/binary.rs` (`#![cfg(unix)]`): pornește un `TestServer` plaintext, creează un cod de sală, rulează `guardian-agent --config <tmp>/agent.toml --data-dir <tmp>/data enroll --server ws://… --code …` → cod 0 și `computer_id` în config; a doua rulare `enroll` → cod 1 cu „already enrolled"; `run` pornit ca proces → `ListComputers` arată `online` în 5 s; `status` afișează `computer_id`; SIGTERM → ieșire cu cod 0 în 5 s. Pe macOS platforma e `Unsupported`: `run` pornește oricum (enforcement-ul scrie `warn!` și rămâne inactiv, conexiunea funcționează) — comportament documentat pentru dezvoltare.
- [ ] **Step 2: Rulează** `cargo test -p guardian-agent --test binary` — Expected: FAIL.
- [ ] **Step 3: Implementează** binarul, serviciul Windows (după modelul `crates/guardian-server/src/service_windows.rs`) și fișierele din `packaging/agent-linux-v3/`. Regula „platformă nesuportată": `Engine` cu o platformă care întoarce `Unsupported` scrie `warn!("process control is not supported on this OS")` o dată și nu reîncearcă.
- [ ] **Step 4: Rulează** testul pe macOS și în containerul Linux; `cargo check` pe ambele ținte Windows; `sh -n packaging/agent-linux-v3/*.sh` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-agent packaging/agent-linux-v3 Cargo.lock
git commit -m "agent: add the guardian-agent binary, the Windows service and Linux packaging"
```

---

### Task 14: Prima utilizare reală — server + agent + CLI pe Linux

**Files:**
- Create: `crates/guardian-agent/tests/real_linux.rs` (`#![cfg(target_os = "linux")]`)

**Interfaces:**
- Consumes: binarele `guardian-server`, `guardian-agent`, `guardian-cli`. `CARGO_BIN_EXE_*` nu trece între crate-uri, deci testul ia binarele din directorul dat de `GUARDIAN_BIN_DIR` (implicit `<workspace>/target/debug`, calculat din `CARGO_MANIFEST_DIR`); dacă lipsește unul, testul eșuează cu mesajul `run: cargo build --workspace --bins`.

- [ ] **Step 1: Scrie testul** `server_agent_cli_on_real_linux`:
  1. pornește `guardian-server --data-dir <tmp> --listen 127.0.0.1:0` (TLS self-signed), citește adresa, amprenta și codul de inițializare din stderr;
  2. `guardian-cli --server https://… --cert-sha256 … init` (parola pe stdin), `room add "Sala 1"`, `profile apps Lecție guardian-test-game`, `code agent --room "Sala 1" --uses 1`;
  3. `guardian-agent enroll --server wss://… --cert-sha256 … --code …` și `guardian-agent run` (ca user curent, fără root: proc connector refuzat → `Polling`, ceea ce testează rezerva pe un sistem real);
  4. copiază `/bin/sleep` ca `<tmp>/guardian-test-game` și îl pornește cu `60` → procesul moare în ≤ 3 s;
  5. `guardian-cli room profile "Sala 1" Liber` → un nou `guardian-test-game` trăiește 3 s;
  6. oprește serverul → un nou `guardian-test-game` e totuși omorât (după ce testul a pus înapoi „Lecție" înainte de oprire și a așteptat 1 s);
  7. SIGTERM la agent și server → ambele ies cu 0.
- [ ] **Step 2: Rulează** în container: `cargo build --workspace --bins && cargo test -p guardian-agent --test real_linux` — Expected: FAIL înainte ca toate piesele să fie legate, PASS după.
- [ ] **Step 3: Repară** ce iese la iveală în codul taskului vinovat (nu în test).
- [ ] **Step 4: Rulează** din nou de 3 ori — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-agent/tests/real_linux.rs
git commit -m "agent: cover a real server, agent and CLI run on Linux"
```

---

### Task 15: CI pentru agent (Linux root, Windows 32/64 cu ETW)

**Files:**
- Modify: `.github/workflows/rust.yml`

**Interfaces:**
- Produces, în plus față de joburile existente:
  - `linux`: după `cargo test --workspace --all-features`, `cargo build --workspace --bins` + `cargo test -p guardian-agent --test real_linux`, și testele de platformă ca root: `sudo -E env "PATH=$PATH" cargo test -p guardian-platform --features testing --test linux -- --include-ignored`.
  - `windows` (windows-latest, admin): `cargo test -p guardian-core -p guardian-proto -p guardian-platform --features guardian-platform/testing -- --include-ignored` (include ETW), `cargo test -p guardian-agent` (testele cu `TestServer`), apoi `cargo build --release -p guardian-agent -p guardian-worker --target x86_64-pc-windows-msvc` și `--target i686-pc-windows-msvc`; artefactele (cele 4 `.exe`) se urcă cu `actions/upload-artifact` sub numele `agent-windows`.
  - `paths` includ și `packaging/agent-linux-v3/**`.

- [ ] **Step 1: Scrie modificările workflow-ului.**
- [ ] **Step 2: Verifică** sintaxa local cu `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/rust.yml'))"` — Expected: fără erori. Rularea reală are loc la primul push al branch-ului (decizia proprietarului; până atunci, pasul rămâne neverificat și se spune asta în raport).
- [ ] **Step 3: Commit**

```bash
git add .github/workflows/rust.yml
git commit -m "build: test the agent on Linux as root and on Windows with ETW, and build 32/64-bit binaries"
```

---

### Task 16: Documentația

**Files:**
- Create: `specs/agent-v3.md`
- Modify: `AGENTS.md` (secțiunea Guardian 3.0: crate-urile noi, comenzile de test în container, etapa 2 gata), `specs/2026-11-17-rust-rewrite-design.md` (§14: notă cu data că etapa 2 e făcută, cu trimitere la `agent-v3.md`), `~/Projects/PROJECTS.md` (secțiunea guardian, data)

- [ ] **Step 1: Scrie** `specs/agent-v3.md`: crate-uri și module; fișierele și drepturile (Global Constraints); pornirea (stare salvată → engine → conexiune); regulile de stare (`version`), deduplicarea comenzilor, `capabilities` goale; detectarea hibridă (moduri, intervale, ce se întâmplă la cădere); rădăcinile de încredere pe fiecare OS și de ce; worker-ul (IPC, verificarea PID-ului, sesiunea activă, repornire); binarul (`enroll`, `run`, `status`, `service`); instalarea pe Linux (`install.sh`) și pe Windows (`service install` din `%ProgramFiles%\Guardian`); **lista de verificare manuală pe Windows** (instalare serviciu 32 și 64 de biți, ETW activ în `agent.log`, worker pornit în sesiunea userului după logare și mutat la schimbarea userului, un program din blacklist omorât, unul redenumit `svchost.exe` în Downloads omorât în Examen, reboot fără rețea → starea salvată aplicată).
- [ ] **Step 2: Rulează** verificarea finală: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && cargo deny check`, plus testele Linux în container și `cargo check` pe ambele ținte Windows — Expected: totul verde.
- [ ] **Step 3: Commit**

```bash
git add specs/agent-v3.md AGENTS.md specs/2026-11-17-rust-rewrite-design.md
git commit -m "agent: document the 3.0 agent and its manual Windows checks"
```
