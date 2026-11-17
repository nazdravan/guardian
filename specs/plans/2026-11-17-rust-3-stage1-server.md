# Guardian 3.0 — Etapa 1: fundația și serverul — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un server Guardian 3.0 funcțional în Rust (librărie + binar) cu API gRPC, canal WebSocket pentru agenți, conturi, săli, profiluri, înrolare și comenzi, plus `guardian-cli`, testat cap-coadă cu un agent simulat.

**Architecture:** Workspace Cargo la rădăcina repo-ului. `proto/` conține contractul (gRPC + cadrele WebSocket), compilat în `guardian-proto` cu `protox` (fără `protoc`). `guardian-core` ține logica fără OS și fără rețea (politica efectivă, decizia de enforcement, chei, coduri, registrul de feature-uri). `guardian-server` combină `tonic` și un handler WebSocket `axum` pe **un singur port TLS**, cu SQLite (`sqlx`, un singur scriitor). `guardian-client` (TLS cu amprentă fixată + token) e folosit de `guardian-cli` acum și de consolă/agent în etapele 2–3.

**Tech Stack:** Rust edition 2024 (MSRV 1.85; local 1.95), tokio, tonic 0.14 + tonic-prost + tonic-prost-build, prost 0.14, protox 0.10, axum 0.8 (WebSocket), sqlx 0.9 (sqlite, fără macro-uri `query!`), rustls 0.23 + tokio-rustls 0.26, rcgen, ed25519-dalek, argon2, sha2, governor, clap 4, tracing, thiserror, uuid. Pentru teste: tokio-tungstenite, tempfile, assert_cmd.

**Spec:** [`specs/2026-11-17-rust-rewrite-design.md`](../2026-11-17-rust-rewrite-design.md) (aprobat 2026-11-17). Etapa 1 = §14 punctul 1. Fundal: [`docs/research/2026-11-17-veyon-analysis.md`](../../docs/research/2026-11-17-veyon-analysis.md).

## Înainte de a începe

- Lucrul se face pe branch-ul **`rust-rewrite`**, creat din `main`, cu spec-ul și analiza Veyon deja commit-ate pe el. Nu folosi `feat` nicăieri (nume de branch, mesaje de commit).
- Codul Go rămâne neatins în etapa 1. Workspace-ul Cargo stă lângă el la rădăcină (`go.mod` și `Cargo.toml` coexistă).
- Mesaje de commit: `<crate sau zonă>: <propoziție>` (de exemplu `core: decide which processes a profile kills`). Fără trailer-e de atribuire, fără linkuri de sesiune.
- Versiunile crate-urilor: cele mai noi care compilează împreună; `Cargo.lock` se commit-ează. Atenție la `ed25519-dalek` / `rand` / `rand_core`: alege combinația pe care `ed25519-dalek` o acceptă pentru `generate` (sau generează seed-ul de 32 de octeți cu `rand::rngs::OsRng`/`getrandom` și construiește cheia din seed).

## Global Constraints

- Licențe permise pentru dependențe: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0 doar dacă `cargo-deny` o cere explicit pentru `webpki-roots`/similar (verificat cu `cargo deny check licenses`). Nicio dependență GPL/AGPL. Nu se copiază cod din Veyon sau RustDesk.
- Platforme: serverul și CLI-ul compilează pe Linux și Windows (x86_64); `guardian-core` și `guardian-proto` compilează și pe `i686-pc-windows-msvc`.
- Fără `protoc` în build: `.proto` se compilează cu `protox` din `build.rs`.
- Lints de workspace: `unsafe_code = "forbid"`, `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic` = `deny` în cod de producție (permise în `#[cfg(test)]` și `tests/`).
- Port implicit: **8443** (gRPC, WebSocket `/agent/ws` și, în etapa 4, HTTPS de fișiere — toate pe același port).
- Coduri de înrolare și de inițializare: **8 caractere** din `ABCDEFGHJKMNPQRSTVWXYZ23456789`, comparate fără diferență de majuscule, ignorând spații și `-`. Cod de agent de unică folosință: TTL **15 minute**. Cod de sală: TTL implicit **86400 s**, folosiri maxime obligatorii (1–500). Mesaj de eroare unic pentru cod necunoscut/folosit/expirat: `invalid or expired enrollment code`.
- Rate limiting: **10 cereri/minut/IP** pentru `Auth.Init`, `Auth.Login`, `Auth.EnrollConsole` și pentru înrolarea pe WebSocket.
- WebSocket: ping la **20 s**, agent offline după **60 s** fără trafic, handshake complet în **10 s**, mesaj maxim **1 MiB**. gRPC: mesaj decodat maxim **4 MiB**.
- SQLite: `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`, `auto_vacuum=INCREMENTAL` (setat la crearea bazei); **un singur scriitor** (pool de scriere cu o conexiune), citiri pe un pool separat read-only (4 conexiuni).
- `last_seen`: scris la conectare, la deconectare și cel mult o dată la **5 minute** cât agentul e conectat; „online" vine din hub, nu din bază.
- Retenție implicită **90 de zile** pentru comenzi încheiate și `audit_log`.
- Profiluri create la prima pornire: **„Liber"** (`free`), **„Lecție"** (`blacklist`, listă goală), **„Examen"** (`whitelist`, listă goală). Profil implicit al școlii: **„Lecție"**.
- Nume de aplicație în profil: caractere `[A-Za-z0-9 ._-]`, lungime **3–64**, normalizate la litere mici, fără spații la capete (regula SEC-08a din `specs/audit-2026-06-12.md`).
- Parole: minim **8** caractere; hash **argon2id** cu parametrii impliciți ai crate-ului `argon2`. Login: `[a-z0-9._-]`, 3–32 caractere. Nume de sală/profil: 1–64 caractere, unice.
- Sesiuni: token opac de 32 de octeți aleatori (base64url fără padding), stocat ca SHA-256 hex, expirare **7 zile** de la emitere.
- Comenzi: TTL implicit **600 s**, maxim **86400 s**.
- Versiunea protocolului agent: `PROTOCOL_VERSION = 1`.

## Review Focus

1. **Același calculator se reconectează cât conexiunea veche e încă înregistrată** (Wi-Fi care cade și revine): conexiunea nouă o înlocuiește pe cea veche, vechea e închisă, iar starea și comenzile merg doar pe cea nouă; calculatorul nu apare offline între timp. → test în Task 11.
2. **Ștergerea unui profil folosit** (activ într-o sală, excepție pe un calculator sau profil implicit): refuzată cu `FAILED_PRECONDITION` și un mesaj care spune unde e folosit; nimic nu rămâne cu o referință moartă. → test în Task 8.
3. **Un profesor scos dintr-o sală în timp ce are `Watch` deschis**: nu mai primește evenimente din sala aceea și nu mai poate trimite comenzi acolo, fără să se reconecteze. → test în Task 14.
4. **Cod de sală folosit în paralel peste numărul maxim** (o sală întreagă pornește installer-ul deodată): exact `max_uses` înrolări reușesc, restul primesc `invalid or expired enrollment code`. → test în Task 10.
5. **Comandă către un calculator offline care revine după expirare**: comanda nu se mai livrează; starea ei per calculator devine `EXPIRED`. → test în Task 13.

---

## File Structure

```
Cargo.toml                          # workspace, lints, profile
deny.toml                           # cargo-deny: licențe, surse, advisories
.github/workflows/rust.yml          # fmt, clippy, test (Linux), build+test (Windows), deny
proto/guardian/v1/common.proto      # Mode, Profile, AgentState, Command, CommandEnvelope, CommandResult
proto/guardian/v1/agent.proto       # AgentFrame / ServerFrame (cadrele WebSocket)
proto/guardian/v1/api.proto         # servicii gRPC Auth, Admin, Control, Watch
crates/guardian-proto/              # build.rs (protox + tonic-prost-build), lib.rs (re-export + FILE_DESCRIPTOR_SET)
crates/guardian-core/src/
  lib.rs
  policy.rs                         # Mode, ProfileId, Profile, AppName, effective_profile_id
  protected.rs                      # listele de procese protejate (Windows, Linux)
  enforce.rs                        # Os, ProcessInfo, Verdict, decide
  keys.rs                           # KeyPair, PublicKey, Signature, AuthContext
  codes.rs                          # generate_code, normalize_code
  feature.rs                        # FeatureId, FeatureSpec, ALL_FEATURES, feature_of, PROTOCOL_VERSION
crates/guardian-server/
  migrations/0001_init.sql
  src/lib.rs                        # start(), ServerConfig, TlsMode, ServerHandle
  src/config.rs
  src/db/mod.rs                     # Db (pool scriere + pool citire), deschidere, pragma-uri
  src/db/{users,sessions,rooms,profiles,settings,computers,codes,consoles,commands,audit}.rs
  src/batch.rs                      # scriitor pe loturi pentru rezultate de comenzi și audit
  src/auth.rs                       # parole, token-uri, Caller, interceptor, rate limit
  src/state.rs                      # AppState partajat (Db, Hub, limiter, config)
  src/hub.rs                        # conexiunile agenților, push de stare, livrare comenzi, evenimente Watch
  src/ws.rs                         # handler /agent/ws: înrolare, handshake, buclă
  src/grpc/{mod,auth,admin,control,watch}.rs
  src/tls.rs                        # cert din fișiere / self-signed (rcgen), amprentă
  src/net.rs                        # un port: TLS + ALPN, gRPC și WebSocket pe același listener
  src/retention.rs                  # curățare zilnică
  src/testing.rs                    # (feature "testing") TestServer, SimAgent
  src/main.rs                       # binar: clap, tracing, Linux/Windows service
  tests/*.rs                        # teste de integrare
crates/guardian-client/src/lib.rs   # connect(), TLS cu amprentă fixată, token în metadata
crates/guardian-cli/src/main.rs     # comenzi clap
crates/guardian-cli/src/{session,commands}.rs
crates/guardian-cli/tests/cli.rs
packaging/server-linux-v3/          # guardian-server.service, install.sh
```

---

### Task 1: Workspace, lints, CI

**Files:**
- Create: `Cargo.toml`, `deny.toml`, `.github/workflows/rust.yml`, `crates/guardian-core/Cargo.toml`, `crates/guardian-core/src/lib.rs`
- Modify: `.gitignore` (adaugă `/target`)

**Interfaces:**
- Produces: workspace cu `members = ["crates/*"]`, `[workspace.package] edition = "2024", rust-version = "1.85", license = "MIT"`, `[workspace.lints]` conform Global Constraints, `[workspace.dependencies]` pentru toate crate-urile din Tech Stack (fiecare crate le ia cu `x.workspace = true`).

- [ ] **Step 1: Creează workspace-ul** cu `guardian-core` ca lib goală care conține un singur test `#[test] fn workspace_builds() {}` și `lints.workspace = true`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-core` — Expected: `1 passed`.
- [ ] **Step 3: Rulează** `cargo clippy --workspace --all-targets -- -D warnings` și `cargo fmt --all --check` — Expected: fără erori.
- [ ] **Step 4: Scrie `deny.toml`** (licențele din Global Constraints, `[sources] unknown-registry = "deny"`, `unknown-git = "deny"`) și rulează `cargo deny check` (instalează cu `cargo install cargo-deny` dacă lipsește) — Expected: `advisories ok, bans ok, licenses ok, sources ok`.
- [ ] **Step 5: Scrie `.github/workflows/rust.yml`**: rulează la push și pull request pe orice branch, cu `paths` pe `crates/**`, `proto/**`, `Cargo.*`, `deny.toml`, workflow-ul însuși. Joburi: `linux` (ubuntu-latest: fmt check, clippy `-D warnings`, `cargo test --workspace --all-features`), `windows` (windows-latest: `cargo build --workspace`, `cargo test -p guardian-core -p guardian-proto`, `rustup target add i686-pc-windows-msvc` + `cargo build -p guardian-core -p guardian-proto --target i686-pc-windows-msvc`), `deny` (`EmbarkStudios/cargo-deny-action`). `release.yml` existent nu se atinge.
- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock deny.toml .gitignore .github/workflows/rust.yml crates/guardian-core
git commit -m "build: add the Rust workspace, lints and CI"
```

---

### Task 2: Contractul protobuf și `guardian-proto`

**Files:**
- Create: `proto/guardian/v1/common.proto`, `proto/guardian/v1/agent.proto`, `proto/guardian/v1/api.proto`, `crates/guardian-proto/Cargo.toml`, `crates/guardian-proto/build.rs`, `crates/guardian-proto/src/lib.rs`
- Test: `crates/guardian-proto/tests/roundtrip.rs`

**Interfaces:**
- Produces: `guardian_proto::v1::*` (pachet `guardian.v1`, cod `prost` + servere și clienți `tonic` pentru `Auth`, `Admin`, `Control`, `Watch`); `guardian_proto::FILE_DESCRIPTOR_SET: &[u8]` (pentru reflection).

Conținutul `.proto` (numele câmpurilor și numerele sunt contract; păstrează-le exact):

`common.proto`:
- `enum Mode { MODE_UNSPECIFIED = 0; MODE_FREE = 1; MODE_BLACKLIST = 2; MODE_WHITELIST = 3; }`
- `message Profile { int64 id = 1; string name = 2; Mode mode = 3; repeated string apps = 4; }`
- `message AgentState { uint64 version = 1; int64 effective_profile_id = 2; repeated Profile profiles = 3; int64 room_id = 4; /* 0 = fără sală */ }`
- `message MessageCmd { string title = 1; string text = 2; }`, `message ProgramStartCmd { string path = 1; repeated string args = 2; }`, `message SiteOpenCmd { string url = 1; }`, `message ShutdownCmd { uint32 countdown_seconds = 1; }`, `message RestartCmd { uint32 countdown_seconds = 1; }`
- `message Command { oneof kind { MessageCmd message = 1; ProgramStartCmd program_start = 2; SiteOpenCmd site_open = 3; ShutdownCmd shutdown = 4; RestartCmd restart = 5; } }` (fișierele se adaugă în etapa 4 ca variante noi)
- `message CommandEnvelope { string id = 1; /* UUID */ int64 expires_at_unix = 2; Command command = 3; }`
- `enum ResultStatus { RESULT_STATUS_UNSPECIFIED = 0; RESULT_PENDING = 1; RESULT_DELIVERED = 2; RESULT_OK = 3; RESULT_FAILED = 4; RESULT_NO_USER = 5; RESULT_UNSUPPORTED = 6; RESULT_EXPIRED = 7; }`
- `message CommandResult { string command_id = 1; ResultStatus status = 2; string detail = 3; }`

`agent.proto` (fiecare mesaj WebSocket binar = exact un cadru codat protobuf):
- `message SystemInfo { string hostname = 1; string os = 2; /* "windows" | "linux" */ string agent_version = 3; repeated string lan_addresses = 4; }`
- `message Enroll { string code = 1; bytes public_key = 2; SystemInfo info = 3; }`
- `message Hello { uint64 computer_id = 1; uint32 protocol_version = 2; repeated string capabilities = 3; /* nume de feature */ SystemInfo info = 4; }`
- `message ChallengeResponse { bytes signature = 1; }`
- `message AgentFrame { oneof msg { Enroll enroll = 1; Hello hello = 2; ChallengeResponse challenge_response = 3; CommandResult command_result = 4; } }`
- `message Enrolled { uint64 computer_id = 1; }`, `message Challenge { bytes nonce = 1; /* 32 octeți */ }`
- `message TrustedConsole { int64 console_id = 1; bytes public_key = 2; }`, `message AuthorizedUser { int64 user_id = 1; string login = 2; }`
- `message Welcome { AgentState state = 1; repeated CommandEnvelope pending = 2; repeated TrustedConsole consoles = 3; repeated AuthorizedUser users = 4; /* admini + profesorii sălii calculatorului */ }`
- `message ServerError { string code = 1; string message = 2; }` cu coduri: `invalid_code`, `unauthorized`, `protocol`, `rate_limited`, `revoked`
- `message ServerFrame { oneof msg { Enrolled enrolled = 1; Challenge challenge = 2; Welcome welcome = 3; AgentState state = 4; CommandEnvelope command = 5; ServerError error = 6; Welcome trust_update = 7; /* doar consoles + users */ } }`

`api.proto` (`import "google/protobuf/empty.proto"` nu e necesar — definește `message Empty {}`):
- `service Auth { rpc Init(InitRequest) returns (LoginResponse); rpc Login(LoginRequest) returns (LoginResponse); rpc Logout(Empty) returns (Empty); rpc ChangePassword(ChangePasswordRequest) returns (Empty); rpc WhoAmI(Empty) returns (User); rpc EnrollConsole(EnrollConsoleRequest) returns (EnrollConsoleResponse); }`
  - `InitRequest { string bootstrap_code = 1; string login = 2; string password = 3; }`, `LoginRequest { string login = 1; string password = 2; }`, `LoginResponse { string token = 1; int64 expires_at_unix = 2; User user = 3; }`, `ChangePasswordRequest { string old_password = 1; string new_password = 2; }`, `EnrollConsoleRequest { string code = 1; bytes public_key = 2; string name = 3; }`, `EnrollConsoleResponse { int64 console_id = 1; }`
- `enum Role { ROLE_UNSPECIFIED = 0; ROLE_ADMIN = 1; ROLE_TEACHER = 2; }`, `message User { int64 id = 1; string login = 2; Role role = 3; bool active = 4; repeated int64 room_ids = 5; }`
- `message Room { int64 id = 1; string name = 2; int64 active_profile_id = 3; }`
- `message Computer { uint64 id = 1; string name = 2; string hostname = 3; string os = 4; string agent_version = 5; int64 room_id = 6; int64 exception_profile_id = 7; repeated string lan_addresses = 8; bool online = 9; int64 last_seen_unix = 10; int64 effective_profile_id = 11; }` (0 = lipsă pentru id-uri)
- `service Admin`: `ListUsers(Empty) returns (UserList)`, `CreateUser(CreateUserRequest{login,password,role}) returns (User)`, `UpdateUser(UpdateUserRequest{int64 id; optional Role role; optional bool active; optional string new_password}) returns (User)`, `SetUserRooms(SetUserRoomsRequest{int64 user_id; repeated int64 room_ids}) returns (User)`, `ListRooms(Empty) returns (RoomList)`, `CreateRoom(CreateRoomRequest{name}) returns (Room)`, `RenameRoom(RenameRoomRequest{id,name}) returns (Room)`, `DeleteRoom(IdRequest) returns (Empty)`, `ListProfiles(Empty) returns (ProfileList)`, `CreateProfile(Profile) returns (Profile)` (id ignorat), `UpdateProfile(Profile) returns (Profile)`, `DeleteProfile(IdRequest) returns (Empty)`, `ListComputers(Empty) returns (ComputerList)`, `UpdateComputer(UpdateComputerRequest{uint64 id; optional string name; optional int64 room_id; optional int64 exception_profile_id}) returns (Computer)` (valoarea 0 la `room_id`/`exception_profile_id` = scoate), `RevokeComputer(ComputerIdRequest{uint64 id}) returns (Empty)`, `CreateEnrollCode(CreateEnrollCodeRequest{EnrollCodeKind kind; int64 room_id; uint32 max_uses; uint32 ttl_seconds}) returns (EnrollCode{string code; int64 expires_at_unix; uint32 max_uses; EnrollCodeKind kind; int64 room_id})`, `ListConsoles(Empty) returns (ConsoleList)`, `RevokeConsole(IdRequest) returns (Empty)`, `GetSettings(Empty) returns (Settings{int64 default_profile_id; uint32 retention_days})`, `UpdateSettings(Settings) returns (Settings)`.
  - `enum EnrollCodeKind { ENROLL_CODE_KIND_UNSPECIFIED = 0; ENROLL_AGENT = 1; ENROLL_CONSOLE = 2; }`; `message IdRequest { int64 id = 1; }`; listele: `UserList { repeated User users = 1; }` etc.
- `service Control`: `SetRoomProfile(SetRoomProfileRequest{int64 room_id; int64 profile_id}) returns (Room)`, `SendCommand(SendCommandRequest{oneof target { int64 room_id = 1; ComputerIds computers = 2; } Command command = 3; uint32 ttl_seconds = 4;}) returns (SendCommandResponse{string command_id})`, `GetCommand(GetCommandRequest{string command_id}) returns (CommandStatus{string command_id; Command command; int64 created_at_unix; int64 expires_at_unix; repeated ComputerResult results})`; `ComputerIds { repeated uint64 ids = 1; }`, `ComputerResult { uint64 computer_id = 1; ResultStatus status = 2; string detail = 3; }`.
- `service Watch { rpc Watch(Empty) returns (stream WatchEvent); }`; `message WatchEvent { oneof event { Snapshot snapshot = 1; Computer computer = 2; /* online/offline/actualizat */ Room room = 3; ComputerResultEvent command_result = 4; uint64 computer_removed = 5; int64 room_removed = 6; } }`; `Snapshot { repeated Room rooms = 1; repeated Computer computers = 2; repeated Profile profiles = 3; }`; `ComputerResultEvent { string command_id = 1; ComputerResult result = 2; }`.

- [ ] **Step 1: Scrie testul** `crates/guardian-proto/tests/roundtrip.rs`:

```rust
use guardian_proto::v1::{agent_frame, server_frame, AgentFrame, Enroll, ServerFrame, Welcome, AgentState, Profile, Mode};
use prost::Message;

#[test]
fn agent_frame_roundtrips() {
    let f = AgentFrame { msg: Some(agent_frame::Msg::Enroll(Enroll { code: "ABCD2345".into(), public_key: vec![7; 32], info: None })) };
    let back = AgentFrame::decode(f.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back, f);
}

#[test]
fn welcome_carries_profiles() {
    let p = Profile { id: 2, name: "Lecție".into(), mode: Mode::Blacklist as i32, apps: vec!["chrome".into()] };
    let f = ServerFrame { msg: Some(server_frame::Msg::Welcome(Welcome { state: Some(AgentState { version: 1, effective_profile_id: 2, profiles: vec![p], room_id: 0 }), ..Default::default() })) };
    let back = ServerFrame::decode(f.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back, f);
}

#[test]
fn descriptor_set_is_embedded() {
    assert!(!guardian_proto::FILE_DESCRIPTOR_SET.is_empty());
}
```

- [ ] **Step 2: Rulează** `cargo test -p guardian-proto` — Expected: FAIL (crate-ul nu există).
- [ ] **Step 3: Implementează** `build.rs`: `protox::compile(["guardian/v1/common.proto","guardian/v1/agent.proto","guardian/v1/api.proto"], ["../../proto"])` → `FileDescriptorSet`; scrie-l în `OUT_DIR/guardian_descriptor.bin`; generează cu `tonic_prost_build::configure().build_server(true).build_client(true).compile_fds(fds)`. `lib.rs`: `pub mod v1 { tonic::include_proto!("guardian.v1"); }` și `pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guardian_descriptor.bin"));`. Adaugă `cargo:rerun-if-changed=../../proto`.
- [ ] **Step 4: Rulează** `cargo test -p guardian-proto` — Expected: `3 passed`.
- [ ] **Step 5: Commit**

```bash
git add proto crates/guardian-proto Cargo.toml Cargo.lock
git commit -m "proto: define the gRPC API and the agent WebSocket frames"
```

---

### Task 3: Politica — nume de aplicații, profiluri, profil efectiv

**Files:**
- Create: `crates/guardian-core/src/policy.rs`
- Modify: `crates/guardian-core/src/lib.rs`, `crates/guardian-core/Cargo.toml` (dependență `guardian-proto`, `thiserror`)

**Interfaces:**
- Produces:
  - `pub enum Mode { Free, Blacklist, Whitelist }` cu `impl TryFrom<guardian_proto::v1::Mode>` (UNSPECIFIED → eroare) și `impl From<Mode> for guardian_proto::v1::Mode`
  - `pub struct ProfileId(pub i64)` (Copy, Eq, Hash, Ord)
  - `pub struct AppName(String)` cu `pub fn parse(raw: &str) -> Result<AppName, AppNameError>`, `pub fn as_str(&self) -> &str`
  - `pub enum AppNameError { Length, Charset }`
  - `pub struct Profile { pub id: ProfileId, pub name: String, pub mode: Mode, pub apps: BTreeSet<AppName> }` cu `impl TryFrom<&guardian_proto::v1::Profile>` și `pub fn to_proto(&self) -> guardian_proto::v1::Profile`
  - `pub fn effective_profile_id(exception: Option<ProfileId>, room_active: Option<ProfileId>, school_default: ProfileId) -> ProfileId`

- [ ] **Step 1: Scrie testele** (în `policy.rs`, `#[cfg(test)] mod tests`):

```rust
#[test] fn app_name_is_trimmed_and_lowercased() { assert_eq!(AppName::parse("  Chrome.EXE ").unwrap().as_str(), "chrome.exe"); }
#[test] fn app_name_rejects_paths_and_wildcards() {
    assert_eq!(AppName::parse(r"C:\x\chrome.exe"), Err(AppNameError::Charset));
    assert_eq!(AppName::parse("chr*me"), Err(AppNameError::Charset));
    assert_eq!(AppName::parse("/usr/bin/vlc"), Err(AppNameError::Charset));
}
#[test] fn app_name_length_is_3_to_64() {
    assert_eq!(AppName::parse("qq"), Err(AppNameError::Length));
    assert!(AppName::parse("vlc").is_ok());
    assert!(AppName::parse(&"a".repeat(64)).is_ok());
    assert_eq!(AppName::parse(&"a".repeat(65)), Err(AppNameError::Length));
}
#[test] fn exception_wins_then_room_then_default() {
    let (e, r, d) = (ProfileId(9), ProfileId(5), ProfileId(2));
    assert_eq!(effective_profile_id(Some(e), Some(r), d), e);
    assert_eq!(effective_profile_id(None, Some(r), d), r);
    assert_eq!(effective_profile_id(None, None, d), d);
}
#[test] fn unspecified_mode_is_rejected() {
    assert!(Mode::try_from(guardian_proto::v1::Mode::Unspecified).is_err());
}
```

- [ ] **Step 2: Rulează** `cargo test -p guardian-core policy` — Expected: FAIL (tipuri nedefinite).
- [ ] **Step 3: Implementează** tipurile de mai sus în `policy.rs`; lungimea se verifică după `trim`, setul de caractere pe textul trim-uit (`[A-Za-z0-9 ._-]`).
- [ ] **Step 4: Rulează** `cargo test -p guardian-core policy` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-core Cargo.lock
git commit -m "core: model profiles and pick a computer's effective profile"
```

---

### Task 4: Procese protejate și decizia de enforcement

**Files:**
- Create: `crates/guardian-core/src/protected.rs`, `crates/guardian-core/src/enforce.rs`
- Modify: `crates/guardian-core/src/lib.rs`

**Interfaces:**
- Consumes: `Mode`, `Profile`, `AppName` (Task 3).
- Produces:
  - `pub enum Os { Windows, Linux }`
  - `pub fn is_protected(os: Os, name: &str) -> bool` (comparație exactă pe numele în litere mici)
  - `pub struct ProcessInfo { pub pid: u32, pub name: String, pub exe_path: Option<std::path::PathBuf> }`
  - `pub enum Verdict { Allow, Kill }`
  - `pub fn decide(os: Os, profile: &Profile, local_whitelist: &BTreeSet<AppName>, p: &ProcessInfo) -> Verdict`

Valorile listelor (în `protected.rs`):
- **Windows** — exact lista din `isSystemProcess` din `agent/main.go` (de la `"system"` până la `"sshd.exe"`), fără `"procsentinel-agent64.exe"`, `"procsentinel-agent32.exe"`, `"guardian.exe"`, plus: `"guardian-agent.exe"`, `"guardian-worker.exe"`, `"guardian-setup.exe"`, `"guardian-console.exe"`.
- **Linux** — `"init"`, `"systemd"`, `"systemd-journald"`, `"systemd-logind"`, `"systemd-udevd"`, `"systemd-resolved"`, `"systemd-networkd"`, `"systemd-timesyncd"`, `"dbus-daemon"`, `"dbus-broker"`, `"polkitd"`, `"networkmanager"`, `"wpa_supplicant"`, `"sshd"`, `"cron"`, `"crond"`, `"login"`, `"agetty"`, `"sh"`, `"bash"`, `"zsh"`, `"xorg"`, `"xwayland"`, `"gnome-shell"`, `"gnome-session-binary"`, `"kwin_wayland"`, `"kwin_x11"`, `"plasmashell"`, `"sddm"`, `"gdm"`, `"gdm3"`, `"lightdm"`, `"pipewire"`, `"wireplumber"`, `"pulseaudio"`, `"guardian-agent"`, `"guardian-worker"`, `"guardian-setup"`.

Reguli de decizie (algoritmul, pe care testele îl fixează):
- nume normalizat = `p.name.trim().to_lowercase()`; un element `e` din listă se potrivește dacă `e == nume` sau `e == nume fără sufixul ".exe"`;
- protejat dacă `is_protected(os, nume)` sau (pe Linux) `exe_path == None` (fire de kernel);
- `Free` → `Allow`; `Blacklist` → `Kill` dacă se potrivește cu lista profilului și nu e protejat; `Whitelist` → `Kill` dacă nu se potrivește nici cu lista profilului, nici cu `local_whitelist`, și nu e protejat.

- [ ] **Step 1: Scrie testele** (în `enforce.rs`):

```rust
fn prof(mode: Mode, apps: &[&str]) -> Profile { /* Profile cu id 1, nume "t", apps parse-uite */ }
fn proc_(name: &str) -> ProcessInfo { ProcessInfo { pid: 100, name: name.into(), exe_path: Some("/x".into()) } }

#[test] fn free_kills_nothing() { assert_eq!(decide(Os::Windows, &prof(Mode::Free, &[]), &Default::default(), &proc_("chrome.exe")), Verdict::Allow); }
#[test] fn blacklist_matches_with_or_without_exe() {
    let p = prof(Mode::Blacklist, &["chrome"]);
    assert_eq!(decide(Os::Windows, &p, &Default::default(), &proc_("Chrome.exe")), Verdict::Kill);
    assert_eq!(decide(Os::Windows, &p, &Default::default(), &proc_("chromedriver.exe")), Verdict::Allow);
}
#[test] fn blacklist_never_kills_protected() {
    assert_eq!(decide(Os::Windows, &prof(Mode::Blacklist, &["explorer"]), &Default::default(), &proc_("explorer.exe")), Verdict::Allow);
}
#[test] fn whitelist_kills_unlisted_but_not_protected_or_local() {
    let p = prof(Mode::Whitelist, &["word.exe"]);
    let local: BTreeSet<_> = [AppName::parse("printer-tool.exe").unwrap()].into();
    assert_eq!(decide(Os::Windows, &p, &local, &proc_("chrome.exe")), Verdict::Kill);
    assert_eq!(decide(Os::Windows, &p, &local, &proc_("WORD.EXE")), Verdict::Allow);
    assert_eq!(decide(Os::Windows, &p, &local, &proc_("printer-tool.exe")), Verdict::Allow);
    assert_eq!(decide(Os::Windows, &p, &local, &proc_("svchost.exe")), Verdict::Allow);
    assert_eq!(decide(Os::Windows, &p, &local, &proc_("guardian-agent.exe")), Verdict::Allow);
}
#[test] fn linux_kernel_threads_are_never_killed() {
    let k = ProcessInfo { pid: 2, name: "kthreadd".into(), exe_path: None };
    assert_eq!(decide(Os::Linux, &prof(Mode::Whitelist, &[]), &Default::default(), &k), Verdict::Allow);
}
#[test] fn linux_desktop_survives_whitelist() {
    assert_eq!(decide(Os::Linux, &prof(Mode::Whitelist, &[]), &Default::default(), &proc_("gnome-shell")), Verdict::Allow);
}
#[test] fn protected_lists_are_lowercase() {
    for n in protected::WINDOWS.iter().chain(protected::LINUX) { assert_eq!(*n, n.to_lowercase()); }
}
```

- [ ] **Step 2: Rulează** `cargo test -p guardian-core enforce` — Expected: FAIL.
- [ ] **Step 3: Implementează** `protected.rs` (`pub const WINDOWS: &[&str]`, `pub const LINUX: &[&str]`, `is_protected`) și `enforce.rs` după regulile de mai sus.
- [ ] **Step 4: Rulează** `cargo test -p guardian-core` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-core
git commit -m "core: decide which processes a profile kills"
```

---

### Task 5: Chei, coduri, registrul de feature-uri

**Files:**
- Create: `crates/guardian-core/src/keys.rs`, `crates/guardian-core/src/codes.rs`, `crates/guardian-core/src/feature.rs`
- Modify: `crates/guardian-core/src/lib.rs`, `crates/guardian-core/Cargo.toml` (`ed25519-dalek`, `rand`/`getrandom`, `uuid`, `zeroize`)

**Interfaces:**
- Produces:
  - `pub enum AuthContext { Agent, Console }` — etichete: `b"guardian/v1/agent-auth"`, `b"guardian/v1/console-auth"`
  - `pub struct KeyPair` cu `generate() -> KeyPair`, `from_seed(seed: &[u8; 32]) -> KeyPair`, `seed(&self) -> zeroize::Zeroizing<[u8; 32]>`, `public(&self) -> PublicKey`, `sign_challenge(&self, ctx: AuthContext, subject: u64, nonce: &[u8; 32]) -> Signature`
  - `pub struct PublicKey([u8; 32])` cu `from_bytes(&[u8]) -> Result<PublicKey, KeyError>`, `as_bytes(&self) -> &[u8; 32]`, `verify_challenge(&self, ctx: AuthContext, subject: u64, nonce: &[u8; 32], sig: &Signature) -> Result<(), KeyError>`
  - `pub struct Signature([u8; 64])` cu `from_bytes(&[u8]) -> Result<Signature, KeyError>`, `to_bytes(&self) -> [u8; 64]`
  - `pub enum KeyError { Malformed, BadSignature }`
  - Mesajul semnat = `eticheta || nonce (32) || subject.to_le_bytes() (8)`. Pentru o consolă, `subject` = `console_id`; la înrolare (când id-ul nu există încă) nu se semnează nimic.
  - `pub const CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRSTVWXYZ23456789"; pub const CODE_LEN: usize = 8; pub fn generate_code() -> String; pub fn normalize_code(raw: &str) -> String` (majuscule, fără spații și `-`)
  - `pub struct FeatureId(pub uuid::Uuid)`, `pub struct FeatureSpec { pub id: FeatureId, pub name: &'static str, pub allowed_offline: bool }`, `pub const ALL_FEATURES: &[FeatureSpec]`, `pub fn by_name(name: &str) -> Option<&'static FeatureSpec>`, `pub fn feature_of(kind: &guardian_proto::v1::command::Kind) -> &'static FeatureSpec`, `pub const PROTOCOL_VERSION: u32 = 1`

Registrul (UUID-urile sunt definitive, nu se mai schimbă):

| name | UUID | allowed_offline |
|---|---|---|
| `message` | `6f612c55-6c31-4b31-969b-9fbcbe7fd340` | true |
| `program.start` | `4a182ff4-acc0-406c-8939-596a31618bdc` | true |
| `site.open` | `99c80159-9d0f-4b3d-b0f7-b0510a89ec11` | true |
| `power.shutdown` | `99d2affe-9035-4e2b-96d2-2c89c6ef8b53` | true |
| `power.restart` | `6d7c62ea-e047-46fe-8720-73f2671c9c55` | true |
| `files.push` | `ad82cfb6-0ca9-41ff-8e41-98f9cc4b98cd` | false |
| `files.collect` | `daaf0b70-e7d9-4259-bc9f-1967bf287699` | false |

(`files.*` sunt rezervate pentru etapa 4; nu au încă variantă în `Command`.)

- [ ] **Step 1: Scrie testele**:

```rust
// keys.rs
#[test] fn signed_challenge_verifies() {
    let k = KeyPair::generate(); let n = [3u8; 32];
    let s = k.sign_challenge(AuthContext::Agent, 7, &n);
    assert!(k.public().verify_challenge(AuthContext::Agent, 7, &n, &s).is_ok());
}
#[test] fn signature_is_bound_to_context_subject_and_nonce() {
    let k = KeyPair::generate(); let n = [3u8; 32];
    let s = k.sign_challenge(AuthContext::Agent, 7, &n);
    let pk = k.public();
    assert_eq!(pk.verify_challenge(AuthContext::Console, 7, &n, &s), Err(KeyError::BadSignature));
    assert_eq!(pk.verify_challenge(AuthContext::Agent, 8, &n, &s), Err(KeyError::BadSignature));
    assert_eq!(pk.verify_challenge(AuthContext::Agent, 7, &[4u8; 32], &s), Err(KeyError::BadSignature));
    assert_eq!(KeyPair::generate().public().verify_challenge(AuthContext::Agent, 7, &n, &s), Err(KeyError::BadSignature));
}
#[test] fn seed_roundtrip_keeps_identity() {
    let k = KeyPair::generate(); let k2 = KeyPair::from_seed(&k.seed());
    assert_eq!(k.public(), k2.public());
}
#[test] fn malformed_bytes_are_rejected() {
    assert_eq!(PublicKey::from_bytes(&[0u8; 31]), Err(KeyError::Malformed));
    assert_eq!(Signature::from_bytes(&[0u8; 63]), Err(KeyError::Malformed));
}
// codes.rs
#[test] fn codes_use_the_alphabet() { let c = generate_code(); assert_eq!(c.len(), 8); assert!(c.chars().all(|ch| CODE_ALPHABET.contains(ch))); }
#[test] fn normalize_ignores_case_spaces_and_dashes() { assert_eq!(normalize_code(" abcd-2345 "), "ABCD2345"); }
// feature.rs
#[test] fn feature_names_and_ids_are_unique() { /* HashSet pe name și pe id, lungime == ALL_FEATURES.len() */ }
#[test] fn command_kinds_map_to_features() {
    use guardian_proto::v1::{command::Kind, ShutdownCmd};
    assert_eq!(feature_of(&Kind::Shutdown(ShutdownCmd { countdown_seconds: 60 })).name, "power.shutdown");
}
```

- [ ] **Step 2: Rulează** `cargo test -p guardian-core` — Expected: FAIL pe testele noi.
- [ ] **Step 3: Implementează** cele trei module; `KeyPair` nu derivă `Debug` cu seed-ul (un `Debug` manual care afișează doar cheia publică).
- [ ] **Step 4: Rulează** `cargo test -p guardian-core` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-core Cargo.lock
git commit -m "core: add ed25519 challenge keys, enrollment codes and the feature registry"
```

---

### Task 6: Baza de date a serverului

**Files:**
- Create: `crates/guardian-server/Cargo.toml` (feature `testing`), `crates/guardian-server/migrations/0001_init.sql`, `crates/guardian-server/src/lib.rs` (doar `pub mod db; pub mod batch;` deocamdată), `src/db/mod.rs`, `src/batch.rs`
- Test: `crates/guardian-server/tests/db.rs`

**Interfaces:**
- Produces:
  - `pub struct Db { pub read: SqlitePool, pub write: SqlitePool }` cu `pub async fn open(path: &Path) -> Result<Db, DbError>` (creează fișierul, setează pragma-urile din Global Constraints, `write` cu `max_connections(1)`, `read` read-only cu `max_connections(4)`, rulează `sqlx::migrate!("./migrations")` pe `write`)
  - `pub fn now_unix() -> i64`
  - `pub struct Batcher` cu `pub fn spawn(db: Db) -> Batcher`, `pub fn command_result(&self, command_id: Uuid, computer_id: u64, status: ResultStatus, detail: String)`, `pub fn audit(&self, entry: AuditEntry)`, `pub async fn flush(&self)`; scrie la fiecare **500 ms** sau la **100** de elemente, într-o singură tranzacție pe lot
  - `pub struct AuditEntry { pub actor_user_id: Option<i64>, pub action: String, pub target: String, pub detail: String, pub source: AuditSource }`, `pub enum AuditSource { Server, Local }`

Schema (`0001_init.sql`; coloanele sunt contract pentru taskurile următoare, toate timpii în secunde Unix `INTEGER`):
- `users(id INTEGER PRIMARY KEY, login TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, role TEXT NOT NULL CHECK(role IN ('admin','teacher')), active INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL)`
- `sessions(token_hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, expires_at INTEGER NOT NULL)`
- `profiles(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, mode TEXT NOT NULL CHECK(mode IN ('free','blacklist','whitelist')))`, `profile_apps(profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, app TEXT NOT NULL, PRIMARY KEY(profile_id, app))`
- `rooms(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, active_profile_id INTEGER NOT NULL REFERENCES profiles(id))`
- `user_rooms(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, PRIMARY KEY(user_id, room_id))`
- `settings(id INTEGER PRIMARY KEY CHECK(id = 1), default_profile_id INTEGER NOT NULL REFERENCES profiles(id), retention_days INTEGER NOT NULL DEFAULT 90, state_version INTEGER NOT NULL DEFAULT 1)`
- `computers(id INTEGER PRIMARY KEY, public_key BLOB NOT NULL UNIQUE, name TEXT NOT NULL DEFAULT '', hostname TEXT NOT NULL DEFAULT '', os TEXT NOT NULL DEFAULT '', agent_version TEXT NOT NULL DEFAULT '', room_id INTEGER REFERENCES rooms(id) ON DELETE SET NULL, exception_profile_id INTEGER REFERENCES profiles(id), lan_addresses TEXT NOT NULL DEFAULT '[]', enrolled_at INTEGER NOT NULL, last_seen INTEGER)`
- `consoles(id INTEGER PRIMARY KEY, public_key BLOB NOT NULL UNIQUE, name TEXT NOT NULL, created_at INTEGER NOT NULL, revoked_at INTEGER)`
- `enroll_codes(code TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('agent','console')), room_id INTEGER REFERENCES rooms(id) ON DELETE CASCADE, uses_left INTEGER NOT NULL, max_uses INTEGER NOT NULL, expires_at INTEGER NOT NULL, created_by INTEGER REFERENCES users(id))`
- `commands(id TEXT PRIMARY KEY, payload BLOB NOT NULL, created_by INTEGER REFERENCES users(id), created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL)`, `command_targets(command_id TEXT NOT NULL REFERENCES commands(id) ON DELETE CASCADE, computer_id INTEGER NOT NULL, status TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL, PRIMARY KEY(command_id, computer_id))` (`payload` = `CommandEnvelope` codat protobuf; `status` = numele variantei `ResultStatus` fără prefix: `pending`, `delivered`, `ok`, `failed`, `no_user`, `unsupported`, `expired`)
- `audit_log(id INTEGER PRIMARY KEY, at INTEGER NOT NULL, actor_user_id INTEGER, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL, source TEXT NOT NULL CHECK(source IN ('server','local')))`
- Indexuri: `command_targets(computer_id, status)`, `audit_log(at)`, `commands(expires_at)`, `sessions(expires_at)`.
- `PRAGMA foreign_keys=ON` pe fiecare conexiune.

Seed-ul (profilurile „Liber"/„Lecție"/„Examen" și rândul din `settings`) **nu** e în migrare: îl face `start()` în Task 7, ca numele să fie într-un singur loc în cod.

- [ ] **Step 1: Scrie testele** `tests/db.rs`:

```rust
#[tokio::test] async fn opens_fresh_db_with_wal_and_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("g.db")).await.unwrap();
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode").fetch_one(&db.read).await.unwrap();
    assert_eq!(mode, "wal");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('users','profiles','rooms','computers','commands','audit_log')").fetch_one(&db.read).await.unwrap();
    assert_eq!(n, 6);
}
#[tokio::test] async fn reopening_runs_no_new_migrations_and_keeps_data() { /* open, insert user, drop, open again, user still there */ }
#[tokio::test] async fn concurrent_writes_never_fail_busy() {
    // 200 task-uri tokio care inserează câte un rând în audit_log prin db.write → toate Ok, count == 200
}
#[tokio::test] async fn read_pool_is_read_only() {
    // un INSERT pe db.read eșuează
}
#[tokio::test] async fn batcher_flushes_audit_in_batches() {
    // 250 × batcher.audit(...), batcher.flush().await → count(audit_log) == 250
}
```

- [ ] **Step 2: Rulează** `cargo test -p guardian-server --test db` — Expected: FAIL.
- [ ] **Step 3: Implementează** `Db::open`, `now_unix`, migrarea și `Batcher` (task tokio cu `mpsc` și `tokio::time::interval(500ms)`; `flush` trimite un mesaj cu `oneshot` și așteaptă golirea).
- [ ] **Step 4: Rulează** `cargo test -p guardian-server --test db` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server Cargo.toml Cargo.lock
git commit -m "server: open SQLite with one writer and add the schema"
```

---

### Task 7: Scheletul serverului, prima pornire și serviciul `Auth`

**Files:**
- Create: `src/config.rs`, `src/state.rs`, `src/auth.rs`, `src/db/users.rs`, `src/db/sessions.rs`, `src/db/profiles.rs` (doar `seed_defaults`), `src/db/settings.rs`, `src/grpc/mod.rs`, `src/grpc/auth.rs`, `src/net.rs` (varianta fără TLS), `src/testing.rs`
- Modify: `src/lib.rs`
- Test: `crates/guardian-server/tests/auth.rs`

**Interfaces:**
- Consumes: `Db`, `Batcher` (Task 6); `generate_code`, `normalize_code` (Task 5).
- Produces:
  - `pub struct ServerConfig { pub data_dir: PathBuf, pub listen: SocketAddr /* implicit 0.0.0.0:8443 */, pub tls: TlsMode, pub handshake_timeout: Duration /* 10 s */, pub idle_timeout: Duration /* 60 s */, pub ping_interval: Duration /* 20 s */, pub expiry_interval: Duration /* 10 s */, pub retention_interval: Duration /* 24 h */ }` cu `pub fn new(data_dir: PathBuf, tls: TlsMode) -> ServerConfig` (implicitele din comentarii; câmpurile de timp există ca testele să le poată scurta — folosite din Task 11, 13, 15), `pub enum TlsMode { Files { cert: PathBuf, key: PathBuf }, SelfSigned, InsecurePlaintext }` (TLS se implementează în Task 16; aici doar `InsecurePlaintext` funcționează, celelalte întorc eroare „not yet")
  - `pub async fn start(config: ServerConfig) -> Result<ServerHandle, StartError>`: deschide `data_dir/guardian.db`, face seed-ul (o dată: cele trei profiluri + `settings` cu profilul implicit „Lecție"), generează **codul de inițializare** dacă nu există niciun user (în memorie, unul per proces, scris în log cu `tracing::warn!` ca `bootstrap code: XXXXXXXX`), pornește listener-ul
  - `pub struct ServerHandle` cu `local_addr(&self) -> SocketAddr`, `bootstrap_code(&self) -> Option<String>`, `cert_sha256(&self) -> Option<String>`, `async fn shutdown(self)`
  - `pub struct Caller { pub user_id: i64, pub login: String, pub role: Role, pub room_ids: BTreeSet<i64> }` cu `pub fn require_admin(&self) -> Result<(), tonic::Status>`, `pub fn can_control_room(&self, room_id: i64) -> bool` (admin = orice sală)
  - `pub async fn caller_from_request<T>(state: &AppState, req: &tonic::Request<T>) -> Result<Caller, tonic::Status>`: citește `authorization: Bearer <token>`, caută `sha256(token)` în `sessions` neexpirate, încarcă userul **activ** și sălile lui la fiecare apel (fără cache — un profesor scos dintr-o sală pierde dreptul imediat)
  - `pub fn hash_password(p: &str) -> Result<String, AuthError>`, `pub fn verify_password(p: &str, hash: &str) -> bool`
  - `pub struct RateLimiter` cu `pub fn check(&self, ip: IpAddr) -> Result<(), tonic::Status>` (10/min/IP, `RESOURCE_EXHAUSTED`)
  - `pub struct AppState { pub db: Db, pub batcher: Batcher, pub limiter: RateLimiter, pub bootstrap: Mutex<Option<String>>, … }` (Hub se adaugă în Task 11)
  - În `testing.rs` (feature `testing`): `pub struct TestServer { pub handle: ServerHandle, pub dir: tempfile::TempDir }` cu `pub async fn start() -> TestServer` (plaintext pe `127.0.0.1:0`, timpi scurtați: handshake 1 s, idle 2 s, ping 500 ms, expirare 100 ms), `pub async fn start_with(f: impl FnOnce(&mut ServerConfig)) -> TestServer`, `pub fn url(&self) -> String` (`http://…` sau `https://…`), `pub fn ws_url(&self) -> String` (`ws://…/agent/ws` sau `wss://…`), `pub fn pin(&self) -> Option<String>`, `pub async fn grpc(&self) -> tonic::transport::Channel`, `pub async fn admin_token(&self) -> String` (face `Init` cu `admin`/`admin-password` și întoarce token-ul), `pub async fn teacher_token(&self, login: &str, rooms: &[i64]) -> String` (definit în Task 9)

`Auth` (comportament):
- `Init`: dacă există deja un user → `FAILED_PRECONDITION`; cod greșit → `PERMISSION_DENIED`; altfel creează admin activ, consumă codul (`bootstrap` devine `None`), întoarce sesiune. Rate limit.
- `Login`: login necunoscut, parolă greșită și user inactiv → același `UNAUTHENTICATED` „invalid login or password". Rate limit.
- `Logout`: șterge sesiunea curentă. `WhoAmI`: userul curent cu sălile. `ChangePassword`: cere parola veche corectă; după schimbare **șterge celelalte sesiuni** ale userului.
- `EnrollConsole`: implementat în Task 10.

- [ ] **Step 1: Scrie testele** `tests/auth.rs` (folosesc `TestServer` și clientul `tonic` generat, cu header-ul pus manual):

```rust
#[tokio::test] async fn fresh_server_has_bootstrap_code_and_seeded_profiles() {
    let s = TestServer::start().await;
    assert_eq!(s.handle.bootstrap_code().map(|c| c.len()), Some(8));
    // ListProfiles cu token de admin după init: nume "Liber","Lecție","Examen"; GetSettings.default_profile_id == id-ul "Lecție"
}
#[tokio::test] async fn init_works_once_with_the_right_code() {
    // cod greșit → PermissionDenied; cod bun → token; al doilea Init → FailedPrecondition; bootstrap_code() == None
}
#[tokio::test] async fn login_errors_do_not_reveal_which_part_was_wrong() {
    // user inexistent și parolă greșită → același cod UNAUTHENTICATED și același mesaj
}
#[tokio::test] async fn calls_without_token_are_rejected() { /* WhoAmI fără header → Unauthenticated */ }
#[tokio::test] async fn change_password_kills_other_sessions() {
    // două login-uri → t1, t2; ChangePassword cu t1; WhoAmI cu t2 → Unauthenticated; cu t1 → Ok
}
#[tokio::test] async fn login_is_rate_limited_per_ip() { /* al 11-lea Login în aceeași minută → ResourceExhausted */ }
#[tokio::test] async fn restart_keeps_users_and_does_not_reseed() {
    // init, shutdown, start pe același data_dir → bootstrap_code() == None, tot 3 profiluri
}
```

- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test auth` — Expected: FAIL.
- [ ] **Step 3: Implementează** modulele listate. Fiecare serviciu gRPC are `max_decoding_message_size(4 * 1024 * 1024)`. `net.rs` servește `tonic::service::Routes` convertit în router `axum` (`into_axum_router`), ca în Task 11 să se poată adăuga `/agent/ws` pe același router; IP-ul clientului vine din `ConnectInfo`/`remote_addr` al cererii. Reflection gRPC (`tonic-reflection` cu `FILE_DESCRIPTOR_SET`) e activă.
- [ ] **Step 4: Rulează** testele — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server Cargo.lock
git commit -m "server: start up, seed profiles, and add login with a bootstrap code"
```

---

### Task 8: `Admin` — profiluri, săli, setări

**Files:**
- Create: `src/db/rooms.rs`
- Modify: `src/db/profiles.rs`, `src/db/settings.rs`, `src/grpc/admin.rs` (nou), `src/grpc/mod.rs`
- Test: `crates/guardian-server/tests/admin_profiles.rs`

**Interfaces:**
- Consumes: `Caller::require_admin`, `AppName::parse`, `Mode`.
- Produces (folosite de Task 12–14):
  - `db::profiles::{list(db) -> Vec<Profile>, get(db, ProfileId) -> Option<Profile>, create, update, delete}` cu `Profile` din `guardian-core`
  - `db::rooms::{list, get, create, rename, delete, set_active_profile(db, room_id, ProfileId)}`
  - `db::settings::{get(db) -> Settings, update}`; `db::settings::bump_state_version(tx) -> u64` (mărește `state_version` în aceeași tranzacție cu orice schimbare care afectează politica agenților)
  - un hook `AppState::policy_changed(Scope)` cu `pub enum Scope { All, Room(i64), Computer(u64) }` — în acest task e un no-op; Task 12 îl leagă de Hub

Comportament:
- Toate RPC-urile `Admin` cer `admin` (altfel `PERMISSION_DENIED`).
- `CreateProfile`/`UpdateProfile`: nume 1–64, unic (`ALREADY_EXISTS`); mod nespecificat → `INVALID_ARGUMENT`; fiecare aplicație trece prin `AppName::parse` (eroare → `INVALID_ARGUMENT` cu numele greșit în mesaj); duplicatele după normalizare se elimină. `UpdateProfile` → `policy_changed(All)`.
- `DeleteProfile` folosit ca profil activ al unei săli, excepție pe un calculator sau profil implicit → `FAILED_PRECONDITION` cu mesaj care numește folosirea (`used by room "Sala 12"`, `used as an exception by 2 computers`, `is the school default`).
- `CreateRoom`: profil activ inițial = profilul implicit al școlii. `DeleteRoom`: calculatoarele sălii rămân fără sală (`ON DELETE SET NULL`) → `policy_changed(All)`.
- `UpdateSettings`: `default_profile_id` trebuie să existe; `retention_days` 1–3650 → `policy_changed(All)`.
- Fiecare schimbare scrie un `AuditEntry` (`action` = numele RPC-ului, `target` = id/nume).

- [ ] **Step 1: Scrie testele**: CRUD profil cu aplicații normalizate (`" Chrome.EXE "` → `"chrome.exe"`); nume duplicat → `AlreadyExists`; aplicație invalidă `"C:\\x"` → `InvalidArgument`; **Review Focus 2**: `delete_profile_in_use_is_refused` — trei cazuri (activ într-o sală, excepție — introdus direct prin SQL în test până la Task 10, profil implicit), fiecare `FailedPrecondition`, profilul încă există; cameră nouă primește profilul implicit; profesor → `PermissionDenied` pe `CreateRoom`; `UpdateSettings` cu profil inexistent → `InvalidArgument`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test admin_profiles` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: manage profiles, rooms and school settings"
```

---

### Task 9: `Admin` — utilizatori și alocarea la săli

**Files:**
- Modify: `src/db/users.rs`, `src/grpc/admin.rs`, `src/testing.rs` (`teacher_token`)
- Test: `crates/guardian-server/tests/admin_users.rs`

**Interfaces:**
- Produces: `db::users::{list, create, update, set_rooms(db, user_id, &[i64]), rooms_of(db, user_id) -> BTreeSet<i64>}`; `TestServer::teacher_token(login, rooms)`; un eveniment intern `AppState::user_rooms_changed(user_id)` (no-op aici; Task 14 îl folosește pentru `Watch`).

Comportament (fiecare schimbare scrie un `AuditEntry`, ca în Task 8): login `[a-z0-9._-]` 3–32 (`INVALID_ARGUMENT`), unic (`ALREADY_EXISTS`); parolă ≥ 8; `UpdateUser` cu `active=false` sau `new_password` **șterge toate sesiunile** userului; un admin nu se poate dezactiva sau retrograda pe sine dacă e ultimul admin activ (`FAILED_PRECONDITION`); `SetUserRooms` cu o sală inexistentă → `INVALID_ARGUMENT` și nimic schimbat.

- [ ] **Step 1: Scrie testele**: creare profesor + login; dezactivare → token-ul existent dă `Unauthenticated`; reset parolă → sesiunile vechi mor; ultimul admin nu se poate retrograda; `SetUserRooms` atomic la sală inexistentă; `WhoAmI` arată sălile.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test admin_users` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: manage teacher accounts and their rooms"
```

---

### Task 10: Coduri de înrolare, calculatoare, console

**Files:**
- Create: `src/db/codes.rs`, `src/db/computers.rs`, `src/db/consoles.rs`
- Modify: `src/grpc/admin.rs`, `src/grpc/auth.rs` (`EnrollConsole`)
- Test: `crates/guardian-server/tests/enroll_codes.rs`

**Interfaces:**
- Consumes: `generate_code`, `normalize_code`, `PublicKey::from_bytes`.
- Produces:
  - `db::codes::create(db, kind: CodeKind, room_id: Option<i64>, max_uses: u32, ttl_secs: u32, by: i64) -> EnrollCode`
  - `db::codes::consume(db, raw: &str, kind: CodeKind) -> Result<Option<i64> /* room_id */, CodeError::Invalid>` — **atomic**: `UPDATE enroll_codes SET uses_left = uses_left - 1 WHERE code = ? AND kind = ? AND uses_left > 0 AND expires_at > ? RETURNING room_id`
  - `pub enum CodeKind { Agent, Console }`
  - `db::computers::{insert(db, &PublicKey, &SystemInfo, room_id: Option<i64>) -> u64, get(db, u64), by_key, list, update(name/room/exception), delete, touch_last_seen(db, u64, i64), set_system_info}`
  - `db::consoles::{insert(db, &PublicKey, name) -> i64, list, revoke(db, id), trusted(db) -> Vec<(i64, PublicKey)>}`

Comportament:
- `CreateEnrollCode` `ENROLL_AGENT` fără sală: TTL forțat **900 s**, `max_uses` forțat **1**. Cu sală: `ttl_seconds` 60–604800 (implicit 86400 dacă 0), `max_uses` 1–500 (obligatoriu). `ENROLL_CONSOLE`: TTL 900 s, 1 folosire, fără sală. Codul se generează până nu există coliziune.
- `EnrollConsole`: rate limit; consumă un cod de tip consolă; cheie publică invalidă → `INVALID_ARGUMENT`; cheie deja folosită → `ALREADY_EXISTS`; eroare de cod → `PERMISSION_DENIED` cu mesajul `invalid or expired enrollment code`. Schimbare → `AppState::trust_changed()` (no-op aici; Task 12 îl leagă de Hub).
- `UpdateComputer`: sală/profil inexistent → `INVALID_ARGUMENT`; schimbare de sală/excepție → `policy_changed(Computer(id))`. `RevokeComputer` → șterge rândul și `AppState::computer_revoked(id)` (no-op aici; Task 11 închide conexiunea). `RevokeConsole` → `revoked_at`, `trust_changed()`.

- [ ] **Step 1: Scrie testele**: cod de agent simplu se poate folosi o dată (a doua oară `Invalid`); cod expirat (TTL manipulat în DB) → `Invalid`; normalizarea `" abcd-2345 "`; tipul greșit (cod de consolă folosit ca agent) → `Invalid`; **Review Focus 4**: `room_code_allows_exactly_max_uses_in_parallel` — cod de sală cu `max_uses = 5`, 30 de `consume` concurente → exact 5 `Ok(Some(room))`, 25 `Invalid`; `EnrollConsole` cu cod bun → console în `ListConsoles`; `UpdateComputer` cu sală inexistentă → `InvalidArgument`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test enroll_codes` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: issue enrollment codes and register computers and consoles"
```

---

### Task 11: Hub-ul și handshake-ul WebSocket

**Files:**
- Create: `src/hub.rs`, `src/ws.rs`
- Modify: `src/net.rs` (ruta `/agent/ws` pe același router), `src/state.rs`, `src/testing.rs` (`SimAgent`)
- Test: `crates/guardian-server/tests/agent_ws.rs`

**Interfaces:**
- Consumes: `db::codes::consume`, `db::computers`, `db::consoles::trusted`, `KeyPair`/`PublicKey::verify_challenge`, `effective_profile_id`, `PROTOCOL_VERSION`.
- Produces:
  - `pub struct Hub` cu `pub fn register(&self, id: u64, caps: BTreeSet<String>, tx: mpsc::Sender<ServerFrame>) -> ConnectionToken`, `pub fn unregister(&self, id: u64, token: ConnectionToken)` (nu face nimic dacă token-ul e al unei conexiuni deja înlocuite), `pub fn is_online(&self, id: u64) -> bool`, `pub fn send(&self, id: u64, frame: ServerFrame) -> bool`, `pub fn disconnect(&self, id: u64)`, `pub fn capabilities(&self, id: u64) -> Option<BTreeSet<String>>`, `pub fn online_ids(&self) -> Vec<u64>`, `pub fn events(&self) -> broadcast::Receiver<HubEvent>`
  - `pub enum HubEvent { ComputerChanged(u64), ComputerRemoved(u64), RoomChanged(i64), RoomRemoved(i64), CommandResult { command_id: Uuid, computer_id: u64 }, UserRoomsChanged(i64) }`
  - `pub async fn build_agent_state(db: &Db, computer_id: u64) -> Result<AgentState, DbError>` (profil efectiv cu `effective_profile_id`, **toate** profilurile, `room_id`, `version` = `settings.state_version`)
  - `pub async fn build_trust(db: &Db, computer_id: u64) -> (Vec<TrustedConsole>, Vec<AuthorizedUser>)` (consolele neretrase; userii activi admin + profesorii sălii calculatorului)
  - `SimAgent` (feature `testing`): `pub async fn enroll(server: &TestServer, code: &str, caps: &[&str]) -> Result<SimAgent, ServerError>` (după `Enrolled` trimite singur `Hello` și semnează provocarea), `pub async fn connect(server: &TestServer, id: u64, key: &KeyPair, caps: &[&str]) -> Result<SimAgent, ServerError>` (ambele folosesc `server.ws_url()` și `server.pin()`, deci merg neschimbate și după TLS din Task 16), `pub fn welcome(&self) -> &Welcome`, `pub async fn next_frame(&mut self, timeout: Duration) -> Option<ServerFrame>`, `pub async fn send_result(&mut self, r: CommandResult)`, `pub fn id(&self) -> u64`, `pub fn key(&self) -> &KeyPair`, `pub async fn close(self)`

Protocolul (server, `ws.rs`):
1. Upgrade cu `max_message_size` 1 MiB. Primul cadru trebuie să sosească, iar handshake-ul să se încheie, în **10 s** (altfel închidere).
2. Primul cadru `Enroll`: rate limit IP; `consume(code, Agent)`; cheie publică validă și necunoscută (cheie existentă → `ServerError{code:"invalid_code"}` — nu dezvăluie că cheia există); `insert` cu sala din cod; răspunde `Enrolled{computer_id}`, apoi continuă ca la pasul 3 **fără să ceară `Hello`** (agentul trimite totuși `Hello` imediat după `Enrolled`; serverul îl acceptă ca pas 3).
3. Cadru `Hello`: `protocol_version` ≠ 1 → `ServerError{code:"protocol"}` + închidere; id necunoscut → `ServerError{code:"unauthorized"}` + închidere; altfel `Challenge{nonce: 32 octeți aleatori}`.
4. `ChallengeResponse`: verificare cu `AuthContext::Agent`, `subject = computer_id`; eșec → `unauthorized` + închidere.
5. Succes: actualizează `SystemInfo` și `last_seen`, `Hub::register` (o conexiune existentă pentru același id e închisă), trimite `Welcome{state, pending, consoles, users}` (`pending` = comenzile nexpirate cu status `pending` pentru acest calculator — Task 13 le umple; aici listă goală), emite `HubEvent::ComputerChanged(id)`.
6. Bucla: ping WebSocket la 20 s; fără niciun cadru primit 60 s → închidere; `CommandResult` → Task 13; la închidere `unregister`, `last_seen`, `ComputerChanged`. Cât e conectat, `last_seen` se scrie cel mult la 5 minute.
7. `AppState::computer_revoked(id)` → `Hub::send(id, ServerError{code:"revoked"})` + `disconnect(id)` + `HubEvent::ComputerRemoved(id)`.

- [ ] **Step 1: Scrie testele** `tests/agent_ws.rs`:
  - `enroll_with_room_code_puts_computer_in_room_and_welcomes_it` — `Welcome.state.room_id` = sala codului, `effective_profile_id` = profilul activ al sălii, `profiles.len() == 3`; `ListComputers` arată `online: true`.
  - `hello_with_wrong_key_is_rejected` — `connect` cu altă cheie → `ServerError{code:"unauthorized"}`.
  - `unknown_protocol_version_is_rejected`.
  - `handshake_times_out_after_10s` — conexiune care nu trimite nimic e închisă (folosește `tokio::time::pause` sau un timeout configurabil din `ServerConfig` pentru teste — adaugă `pub handshake_timeout: Duration` și `pub idle_timeout: Duration` în `ServerConfig` cu implicitele 10 s / 60 s).
  - **Review Focus 1**: `reconnect_replaces_old_connection` — conectează A, apoi încă o dată cu aceeași cheie (B); A primește închidere; `is_online` rămâne `true` tot timpul; o stare nouă ajunge doar la B; închiderea lui A nu marchează calculatorul offline.
  - `revoke_disconnects_and_blocks_reconnect` — `RevokeComputer` → agentul primește `ServerError{code:"revoked"}`; `connect` din nou → `unauthorized`.
  - `same_public_key_cannot_enroll_twice`.
  - `welcome_lists_trusted_consoles_and_room_teachers` — după `EnrollConsole` și `SetUserRooms`, `Welcome.consoles.len()==1`, `users` conține adminul și profesorul sălii, dar nu un profesor al altei săli.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test agent_ws` — Expected: FAIL.
- [ ] **Step 3: Implementează** `hub.rs` (`Mutex<HashMap<u64, Conn>>` + `broadcast::channel(1024)`), `ws.rs`, ruta în `net.rs`, `SimAgent` cu `tokio-tungstenite`.
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server Cargo.lock
git commit -m "server: authenticate agents over WebSocket with ed25519 challenges"
```

---

### Task 12: Împingerea stării la schimbări de politică și de încredere

**Files:**
- Modify: `src/state.rs` (`policy_changed`, `trust_changed`), `src/grpc/admin.rs`, `src/grpc/control.rs` (nou: `SetRoomProfile`), `src/grpc/mod.rs`
- Test: `crates/guardian-server/tests/state_push.rs`

**Interfaces:**
- Consumes: `Hub`, `build_agent_state`, `build_trust`, `Caller::can_control_room`.
- Produces: `AppState::policy_changed(Scope)` — calculează și trimite `ServerFrame::State` fiecărui calculator online din scop (`All` = toate online, `Room(r)` = calculatoarele sălii, `Computer(c)`); `version` crește (`bump_state_version`) o singură dată per schimbare. `AppState::trust_changed()` — trimite `ServerFrame::TrustUpdate` (un `Welcome` cu doar `consoles` și `users`) tuturor calculatoarelor online. `SetUserRooms` și schimbări de rol/activ declanșează și ele `trust_changed()`.

`Control.SetRoomProfile`: admin sau profesorul sălii (altfel `PERMISSION_DENIED`); sală sau profil inexistent → `NOT_FOUND`; setează profilul activ, audit, `policy_changed(Room(r))`, `HubEvent::RoomChanged(r)`.

- [ ] **Step 1: Scrie testele**:
  - `teacher_switching_room_profile_pushes_state_to_room_only` — două săli cu câte un agent; profesorul sălii 1 trece pe „Examen” → agentul din sala 1 primește `State` cu `effective_profile_id` = „Examen” și `version` mai mare; agentul din sala 2 nu primește nimic în 500 ms.
  - `teacher_cannot_switch_other_room` → `PermissionDenied`, nimic trimis.
  - `exception_overrides_room_profile` — `UpdateComputer{exception_profile_id: Liber}` → agentul primește `State` cu „Liber”; scoaterea excepției (0) → revine la profilul sălii.
  - `editing_profile_apps_pushes_to_everyone_online`.
  - `moving_computer_between_rooms_pushes_new_room_state`.
  - `revoking_console_sends_trust_update`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test state_push` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: push profile and trust changes to connected agents"
```

---

### Task 13: Comenzi — trimitere, livrare, coadă, expirare, rezultate

**Files:**
- Create: `src/db/commands.rs`
- Modify: `src/grpc/control.rs` (`SendCommand`, `GetCommand`), `src/ws.rs` (`CommandResult`, `pending` în `Welcome`), `src/hub.rs`
- Test: `crates/guardian-server/tests/commands.rs`

**Interfaces:**
- Consumes: `feature_of`, `Batcher::command_result`, `Hub::send`, `Hub::capabilities`.
- Produces: `db::commands::{create(db, &CommandEnvelope, by: i64, targets: &[u64]) , pending_for(db, computer_id, now) -> Vec<CommandEnvelope>, status(db, Uuid) -> Option<CommandStatus>, expire_due(db, now) -> Vec<(Uuid, u64)>}`.

Comportament:
- `SendCommand`: `command` lipsă sau variantă nespecificată → `INVALID_ARGUMENT`; validări pe argumente: `MessageCmd.title` 0–100 și `text` 1–2000 caractere; `ProgramStartCmd.path` nevid ≤ 1024, ≤ 32 de argumente; `SiteOpenCmd.url` cu schema `http`/`https`, ≤ 2048; `countdown_seconds` 0–3600. `ttl_seconds` 0 → 600, > 86400 → `INVALID_ARGUMENT`.
- Țintă: sală (toate calculatoarele ei) sau listă de id-uri; profesorul doar în sălile lui — un calculator dintr-o sală străină sau fără sală → `PERMISSION_DENIED` pentru toată cererea; listă goală → `INVALID_ARGUMENT`.
- Pentru fiecare țintă: dacă e online și `capabilities` conține `feature_of(kind).name` → trimite `ServerFrame::Command`, status `delivered`; online fără capabilitate → `unsupported` (nu se trimite); offline → `pending`. Răspunde imediat cu `command_id`.
- La `Welcome` (Task 11, pasul 5): `pending` = `pending_for(...)` filtrat pe capabilități (fără capabilitate → `unsupported`); cele trimise devin `delivered`.
- `CommandResult` de la agent: doar pentru o comandă care îl țintește pe acel calculator (altfel ignorat cu log); statusurile acceptate de la agent: `ok`, `failed`, `no_user`, `unsupported`; scris prin `Batcher`; `HubEvent::CommandResult`.
- Un task periodic (la 10 s) marchează `expired` țintele `pending`/`delivered` cu `expires_at` trecut și emite `HubEvent::CommandResult`. Agentul care revine după expirare nu primește comanda.
- Audit pentru fiecare `SendCommand` (`action = "SendCommand"`, `target` = sala sau lista, `detail` = numele feature-ului).

- [ ] **Step 1: Scrie testele**:
  - `command_reaches_online_agent_and_result_is_recorded` — agent cu `caps = ["message"]`; `SendCommand(message)` → agentul primește `Command`; trimite `CommandResult{ok}`; `GetCommand` (după `batcher.flush`) → `ok`.
  - `agent_without_capability_gets_unsupported_without_delivery`.
  - `offline_agent_gets_command_on_reconnect` — status `pending`, după `connect` comanda e în `Welcome.pending`, status `delivered`.
  - **Review Focus 5**: `expired_command_is_not_delivered_later` — `ttl_seconds = 1`, agent offline; după expirare (task-ul de expirare cu interval configurabil în `ServerConfig::expiry_interval`, implicit 10 s, pus la 100 ms în test) status `expired`; la `connect` `Welcome.pending` e gol.
  - `teacher_cannot_target_computer_outside_rooms` → `PermissionDenied`, nimic creat.
  - `invalid_arguments_are_rejected` — `SiteOpenCmd{url:"file:///etc/passwd"}` → `InvalidArgument`; `text` gol → `InvalidArgument`.
  - `result_for_foreign_command_is_ignored`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test commands` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: send commands to agents, queue them while offline, and expire them"
```

---

### Task 14: `Watch` — starea live pe streaming

**Files:**
- Create: `src/grpc/watch.rs`
- Modify: `src/grpc/mod.rs`, `src/state.rs` (`user_rooms_changed` emite `HubEvent::UserRoomsChanged`)
- Test: `crates/guardian-server/tests/watch.rs`

**Interfaces:**
- Consumes: `Hub::events`, `caller_from_request`, `db::rooms::list`, `db::computers::list`, `db::profiles::list`.
- Produces: `Watch.Watch` — primul eveniment `Snapshot` (sălile vizibile, calculatoarele lor cu `online` din Hub și `effective_profile_id`, toate profilurile); apoi `Computer`, `Room`, `ComputerResultEvent`, `computer_removed`, `room_removed` filtrate pe sălile vizibile. Admin vede tot, inclusiv calculatoarele fără sală.

Comportament: setul de săli vizibile al stream-ului se recitește din bază la `HubEvent::UserRoomsChanged(user_id)` al userului stream-ului; dacă userul a fost dezactivat sau sesiunea a expirat, stream-ul se încheie cu `UNAUTHENTICATED`. Dacă receiver-ul `broadcast` rămâne în urmă (`Lagged`), stream-ul trimite un `Snapshot` nou în loc să piardă evenimente.

- [ ] **Step 1: Scrie testele**:
  - `snapshot_then_online_event` — profesor cu sala 1 deschide `Watch`; primește `Snapshot` fără calculatoare online; agentul din sala 1 se conectează → eveniment `Computer{online:true}`.
  - `teacher_does_not_see_other_rooms`.
  - **Review Focus 3**: `removed_from_room_stops_events_and_commands` — profesorul are `Watch` deschis pe sala 1; adminul îi scoate sala; un agent din sala 1 se conectează → profesorul nu primește evenimentul (500 ms); `SendCommand` spre sala 1 cu token-ul lui → `PermissionDenied`.
  - `command_results_are_streamed`.
  - `disabled_user_stream_ends` — dezactivare → stream-ul se termină cu `Unauthenticated`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test watch` — Expected: FAIL.
- [ ] **Step 3: Implementează** (stream `tokio_stream` din `broadcast::Receiver`, filtrare per eveniment).
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: stream live room and computer state to consoles"
```

---

### Task 15: Retenție și curățare

**Files:**
- Create: `src/retention.rs`
- Modify: `src/lib.rs` (pornește task-ul)
- Test: `crates/guardian-server/tests/retention.rs`

**Interfaces:**
- Produces: `pub async fn run_once(db: &Db, now: i64) -> RetentionReport { commands: u64, audit: u64, codes: u64, sessions: u64 }`; task la **24 h** (și o dată la 1 minut după pornire), interval în `ServerConfig::retention_interval`.

Comportament: șterge comenzile la care toate țintele sunt încheiate (`ok`/`failed`/`no_user`/`unsupported`/`expired`) și `created_at < now - retention_days`; `audit_log` cu `at < now - retention_days`; codurile de înrolare expirate sau epuizate; sesiunile expirate; apoi `PRAGMA incremental_vacuum`. Comenzile cu ținte încă `pending`/`delivered` nu se șterg.

- [ ] **Step 1: Scrie testele**: rânduri vechi șterse, cele recente păstrate; comanda veche cu o țintă `pending` neexpirată rămâne; `retention_days` din setări e respectat (setat la 1 în test).
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test retention` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server
git commit -m "server: prune old commands, audit entries, codes and sessions"
```

---

### Task 16: TLS pe un singur port și `guardian-client`

**Files:**
- Create: `src/tls.rs`, `crates/guardian-client/Cargo.toml`, `crates/guardian-client/src/lib.rs`
- Modify: `src/net.rs`, `src/lib.rs`, `src/testing.rs` (`TestServer::start_tls()`; `ws_url`/`pin`/`grpc` folosesc TLS și amprenta)
- Test: `crates/guardian-server/tests/tls.rs`, `crates/guardian-client/tests/pin.rs`

**Interfaces:**
- Produces:
  - `TlsMode::Files` (PEM cert + cheie), `TlsMode::SelfSigned` — la prima pornire generează cu `rcgen` un certificat pentru `localhost`, hostname-ul mașinii și `127.0.0.1`, valabil 10 ani, salvat în `data_dir/tls/cert.pem` + `key.pem` (cheia cu drepturi 0600 pe Unix), refolosit la pornirile următoare
  - `ServerHandle::cert_sha256()` = SHA-256 hex (litere mici, fără separatori) al certificatului DER servit; `None` doar la `InsecurePlaintext`
  - `net.rs`: un singur `TcpListener`; TLS cu ALPN `h2` și `http/1.1`; `hyper-util` auto (HTTP/1.1 pentru upgrade-ul WebSocket, HTTP/2 pentru gRPC) peste același router
  - `guardian_client::rustls_config(pin_sha256: Option<&str>) -> Result<Arc<rustls::ClientConfig>, ClientError>` — fără pin: rădăcinile sistemului (`rustls-native-certs` sau `rustls-platform-verifier`); cu pin: acceptă **doar** certificatul cu acel SHA-256 (verifier propriu, ignoră lanțul și numele)
  - `guardian_client::connect(server_url: &str, pin_sha256: Option<&str>) -> Result<tonic::transport::Channel, ClientError>` (`https://` cu TLS, `http://` doar dacă URL-ul e explicit `http`)
  - `guardian_client::Authed<T>` / `with_token(channel, token) -> InterceptedService<Channel, TokenInterceptor>` care pune `authorization: Bearer <token>`
  - `guardian_client::ws_connector(pin_sha256: Option<&str>) -> tokio_tungstenite::Connector` (pentru agent în etapa 2 și pentru `SimAgent`)

- [ ] **Step 1: Scrie testele**:
  - `self_signed_cert_is_stable_across_restarts` — două porniri pe același `data_dir` dau aceeași amprentă.
  - `grpc_and_websocket_share_the_tls_port` — `TestServer::start_tls()`; `Login` prin `guardian_client::connect(url, Some(pin))` reușește; un `SimAgent` se înrolează pe `wss://` la același port.
  - `wrong_pin_is_refused` — amprentă greșită → eroare de conexiune, nu răspuns.
  - `no_pin_rejects_self_signed` — fără pin, certificatul self-signed e refuzat.
  - `files_mode_serves_given_cert` — cert generat în test cu `rcgen`, scris în fișiere, `TlsMode::Files` → amprenta = SHA-256 al acelui cert.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test tls` și `cargo test -p guardian-client` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** `cargo test --workspace --all-features` — Expected: PASS (toate testele de până acum rămân verzi pe plaintext).
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server crates/guardian-client Cargo.toml Cargo.lock
git commit -m "server: serve gRPC and WebSocket on one TLS port, and add a pinning client"
```

---

### Task 17: Binarul serverului, systemd și serviciul Windows

**Files:**
- Create: `crates/guardian-server/src/main.rs`, `crates/guardian-server/src/service_windows.rs` (`#[cfg(windows)]`), `packaging/server-linux-v3/guardian-server.service`, `packaging/server-linux-v3/install.sh`
- Test: `crates/guardian-server/tests/binary.rs`

**Interfaces:**
- Produces: binarul `guardian-server` cu argumente `clap` (fiecare și din variabilă de mediu):
  - `--data-dir` / `GUARDIAN_DATA_DIR` (implicit: Linux `/var/lib/guardian`, Windows `%ProgramData%\Guardian Server`)
  - `--listen` / `GUARDIAN_LISTEN` (implicit `0.0.0.0:8443`)
  - `--tls-cert` + `--tls-key` / `GUARDIAN_TLS_CERT` + `GUARDIAN_TLS_KEY` (→ `TlsMode::Files`); fără ele → `SelfSigned`
  - `--insecure-plaintext` / `GUARDIAN_INSECURE_PLAINTEXT=1` (→ `InsecurePlaintext`, cu `warn!` la pornire)
  - `--log` / `GUARDIAN_LOG` (filtru `tracing`, implicit `info`)
  - subcomenzi Windows: `service install|uninstall|run` (cu `windows-service`; numele serviciului `GuardianServer`, pornire automată, regulă de firewall `netsh advfirewall firewall add rule name="Guardian Server" dir=in action=allow protocol=TCP localport=<port>` la install, ștearsă la uninstall)
- La pornire scrie în log, pe linii separate și exact cu aceste prefixe: `listening on <addr>` (adresa reală, deci și cu portul ales de sistem la `:0`), `certificate sha256: <hex>` și, dacă există, `bootstrap code: <cod>`. SIGINT/SIGTERM (și oprirea serviciului Windows) → `ServerHandle::shutdown()`.
- `guardian-server.service`: `User=guardian`, `StateDirectory=guardian`, `ExecStart=/usr/local/bin/guardian-server --data-dir /var/lib/guardian`, `Restart=always`, `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `AmbientCapabilities=CAP_NET_BIND_SERVICE` (pentru cine schimbă portul pe 443).
- `install.sh`: creează userul `guardian` (sistem, fără shell), copiază binarul, instalează și pornește unitatea, apoi afișează din `journalctl -u guardian-server` amprenta și codul de inițializare. Rulat de două ori nu strică nimic.

- [ ] **Step 1: Scrie testul** `tests/binary.rs`: pornește binarul (`env!("CARGO_BIN_EXE_guardian-server")`) cu `--data-dir <tmp> --listen 127.0.0.1:0` și TLS self-signed; citește din ieșire liniile `listening on`, `certificate sha256:` și `bootstrap code:`; face `Init` prin `guardian_client` cu pin; trimite SIGTERM (pe Unix) și verifică ieșirea cu cod 0 în 5 s.
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test binary` — Expected: FAIL.
- [ ] **Step 3: Implementează** `main.rs` și `service_windows.rs`; verifică `cargo build -p guardian-server --target x86_64-pc-windows-msvc` în jobul Windows din CI (local pe macOS: `cargo check -p guardian-server --target x86_64-pc-windows-gnu`).
- [ ] **Step 4: Rulează** testul — Expected: PASS; `shellcheck packaging/server-linux-v3/install.sh` — Expected: fără erori.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server packaging/server-linux-v3 Cargo.lock
git commit -m "server: add the guardian-server binary with systemd and Windows service setup"
```

---

### Task 18: `guardian-cli`

**Files:**
- Create: `crates/guardian-cli/Cargo.toml`, `src/main.rs`, `src/session.rs`, `src/commands.rs`
- Test: `crates/guardian-cli/tests/cli.rs` (dev-dependency `guardian-server` cu feature `testing`, `assert_cmd`, `predicates`)

**Interfaces:**
- Consumes: `guardian_client::{connect, with_token}`, clienții generați.
- Produces: binarul `guardian-cli`. Opțiuni globale: `--server <URL>` / `GUARDIAN_SERVER`, `--cert-sha256 <HEX>` / `GUARDIAN_CERT_SHA256`, `--config <PATH>` (implicit fișierul de sesiune din directorul de config al userului: `guardian/cli.toml`, drepturi 0600 pe Unix; conține `server`, `cert_sha256`, `token`). Ieșire text pentru oameni; `--json` pentru scripturi pe comenzile `list`/`show`.
  - `init --code C --login L` (parola citită de pe stdin sau prompt, niciodată din argumente), `login --login L`, `logout`, `whoami`, `passwd`
  - `user list|add <login> --role admin|teacher|set-role <login> <role>|disable <login>|enable <login>|reset-password <login>|rooms <login> <room>...`
  - `room list|add <name>|rename <room> <name>|rm <room>|profile <room> <profile>`
  - `profile list|add <name> --mode free|blacklist|whitelist|mode <profile> <mode>|apps <profile> <app>...|add-app <profile> <app>...|rm-app <profile> <app>...|rm <profile>`
  - `computer list|rename <id> <name>|move <id> <room|-> |except <id> <profile|->|revoke <id>`
  - `code agent [--room R --uses N --ttl SECONDS]|code console`
  - `console list|revoke <id>`
  - `send (--room R | --computer ID...) [--ttl S] message --text T [--title X] | program <path> [-- args...] | site <url> | shutdown [--countdown S] | restart [--countdown S]` → afișează id-ul comenzii
  - `command show <id>`, `watch` (afișează evenimentele pe linii până la Ctrl+C)
  - `settings show|set [--default-profile P] [--retention-days N]`
  - Săli și profiluri se pot da după nume sau id. Codul de ieșire ≠ 0 la orice eroare gRPC, cu mesajul serverului pe stderr.

- [ ] **Step 1: Scrie testele** `tests/cli.rs` (fiecare pornește un `TestServer` plaintext și rulează binarul cu `--server` și `--config` într-un dir temporar):
  - `init_login_and_whoami` — `init` cu codul din `TestServer` și parola pe stdin → `whoami` afișează `admin`.
  - `profile_and_room_flow` — `profile add Desen --mode whitelist`, `profile apps Desen paint.exe`, `room add "Sala 12"`, `room profile "Sala 12" Desen`, `room list --json` conține `Sala 12` cu profilul lui `Desen`.
  - `send_message_to_room_reports_command_id` — un `SimAgent` înrolat în sală cu `code agent --room`; `send --room "Sala 12" message --text Salut` → stdout conține un UUID; agentul primește comanda.
  - `errors_exit_nonzero` — `room profile "Nu există" Lecție` → cod de ieșire ≠ 0, stderr conține mesajul serverului.
  - `session_file_is_private` (doar Unix) — drepturile fișierului `cli.toml` sunt `0o600`.
- [ ] **Step 2: Rulează** `cargo test -p guardian-cli` — Expected: FAIL.
- [ ] **Step 3: Implementează.**
- [ ] **Step 4: Rulează** — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-cli Cargo.lock
git commit -m "cli: add guardian-cli for administration and classroom commands"
```

---

### Task 19: Testul cap-coadă și documentația

**Files:**
- Create: `crates/guardian-server/tests/e2e.rs`, `specs/server-v3.md`
- Modify: `AGENTS.md` (secțiune nouă „Guardian 3.0 (Rust, în lucru)"), `specs/2026-11-17-rust-rewrite-design.md` (doar §9.4: `peers` omis, vezi mai jos)

**Interfaces:**
- Consumes: tot ce e mai sus.

- [ ] **Step 1: Scrie testul** `tests/e2e.rs` — `full_classroom_flow` pe TLS self-signed cu pin:
  1. `Init` admin; creează sala „Sala 12” și profesorul `ion` cu sala; cod de sală cu `max_uses = 3`;
  2. trei `SimAgent` se înrolează (`caps = ["message","power.shutdown"]`) — toți în „Sala 12”, `Welcome` cu profilul „Lecție”;
  3. `ion` deschide `Watch` → `Snapshot` cu 3 calculatoare online;
  4. `ion` trece sala pe „Examen” → cei 3 agenți primesc `State` cu „Examen”; `Watch` primește `Room`;
  5. `ion` trimite `ShutdownCmd{countdown_seconds: 60}` sălii → toți primesc comanda; doi răspund `ok`, unul `no_user`; `GetCommand` arată exact asta; `Watch` a primit 3 `ComputerResultEvent`;
  6. adminul revocă un calculator → agentul primește `revoked`, `Watch` primește `computer_removed`;
  7. un agent se deconectează → `Watch` primește `Computer{online:false}` în ≤ 1 s (închidere curată).
- [ ] **Step 2: Rulează** `cargo test -p guardian-server --features testing --test e2e` — Expected: PASS (dacă pică, repară codul din taskul vinovat, nu testul).
- [ ] **Step 3: Scrie documentația**:
  - `specs/server-v3.md`: componente și module, schema, protocolul WebSocket (pașii din Task 11), maparea RPC → rol, porturi și TLS (amprentă), argumentele binarului, comportamentele de siguranță (fail-secure, revocare, expirare comenzi, retenție).
  - `AGENTS.md`: comenzi `cargo build --workspace`, `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`; harta crate-urilor; regula că `.proto` e contractul și `guardian-core` e singura sursă a logicii de politică; mențiunea că codul Go rămâne până la 3.0.
  - Spec, §9.4: lista `peers` din `Welcome` a fost omisă (niciun consumator în etapele 1–3); notează asta cu data.
- [ ] **Step 4: Rulează** verificarea finală: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && cargo deny check` — Expected: totul verde.
- [ ] **Step 5: Commit**

```bash
git add crates/guardian-server/tests/e2e.rs specs/server-v3.md AGENTS.md specs/2026-11-17-rust-rewrite-design.md
git commit -m "server: cover the full classroom flow end to end and document the 3.0 server"
```
