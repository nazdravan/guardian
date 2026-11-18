# Guardian 3.0 — Etapa 3b: modul de rezervă local — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Când pică legătura cu serverul, profesorul schimbă profilul sălii direct din consolă, prin rețeaua școlii, cu excepția profilurilor „free”. La revenirea serverului schimbarea locală se anulează, iar jurnalul acțiunilor locale ajunge în `audit_log` cu sursa `local`.

**Architecture:**
- Un crate nou, `guardian-local`, conține canalul criptat agent ↔ consolă:
  - Noise IK peste TCP, cu cheile ed25519 existente convertite în X25519;
  - cadre cu lungimea în față;
  - mesajele de descoperire UDP, semnate.
- **Regulile** (cine are voie să schimbe ce, când e agentul „offline”, ce profil se aplică sub o schimbare locală) stau în `guardian-core::local`.
- **Agentul:**
  - ascultă pe portul 11500 (TCP + UDP);
  - răspunde doar după 60 s fără server;
  - ține schimbarea locală și jurnalul în `local.pb`;
  - trimite jurnalul serverului la reconectare.
- **Consola** (doar în modul remote):
  - se înrolează ca „consolă”;
  - păstrează un cache local: sălile, calculatoarele cu cheile și adresele lor, profilurile, verificatorii argon2id ai profesorilor care s-au logat pe acel PC;
  - când serverul nu răspunde, lucrează direct cu agenții și revine singură pe server când acesta răspunde din nou.

**Tech Stack:**
- **Nou:** `snow` 0.10 (Noise, `Noise_IK_25519_ChaChaPoly_BLAKE2s`).
- **Existente:**
  - `ed25519-dalek` 3: `SigningKey::to_scalar_bytes`, `VerifyingKey::to_montgomery`;
  - `argon2` 0.6, `tokio`, `prost`, `if-addrs`;
  - `guardian-server` ca librărie de test (`TestServer`), `guardian-agent` ca librărie;
  - `FakePlatform`, `egui_kittest`.

**Spec:**
- [`specs/2026-11-17-rust-rewrite-design.md`](../2026-11-17-rust-rewrite-design.md): §11 (tot), §8.2 (`allowed_offline`), §8.3 (excepția are prioritate), §9.6 (în modul încorporat rezerva nu intră în joc), §10.2 (offline = 60 s), §14 punctul 3.
- Ce există deja: [`server-v3.md`](../server-v3.md), [`agent-v3.md`](../agent-v3.md), [`console-v3.md`](../console-v3.md).
- Serverul din etapa 1 are deja:
  - `Auth.EnrollConsole`, cu un cod `ENROLL_CONSOLE`, fără sesiune;
  - `Admin.ListConsoles` / `RevokeConsole`;
  - `Welcome.consoles` / `Welcome.users`, plus `trust_update` la fiecare schimbare;
  - `AuditSource::Local`.

## Înainte de a începe

- **Branch:** `rust-rewrite`, peste 3a (`14eba6f`). Fără `feat` în nume și mesaje. Commit-urile sunt `<crate sau zonă>: <propoziție>`, fără trailer-e de atribuire. Nimic nu se împinge.
- **Teste:**
  - tot ce nu e `#[cfg(windows)]` rulează pe macOS și în containerul Linux din etapa 2 (`CARGO_TARGET_DIR=/w/target/linux`);
  - Windows: `cargo clippy --target x86_64-pc-windows-gnu` și `i686-pc-windows-gnu`.
- **Limba:** UI-ul și mesajele agentului sunt în engleză.

## Deciziile planului (ale mele, pe care spec-ul nu le fixează)

1. **Crate nou `guardian-local`:** canalul e folosit și de agent, și de consolă. `guardian-client` e doar partea de client gRPC/WS, iar `guardian-core` rămâne fără rețea.
2. **Cheile Noise sunt cheile ed25519 existente, convertite** (conversia standard Ed25519 → X25519, ca în libsodium). Agentul și consola au deja câte o singură identitate, pe care serverul o cunoaște. Nu apare o a doua pereche de chei care ar trebui distribuită.
3. **Local se poate face, în 3b, doar schimbarea profilului.** Mesajul, oprirea și pornirea de programe nu sunt implementate în agent până în etapa 4 (`CAPABILITIES` e gol). Protocolul local are deja varianta `CommandEnvelope`; agentul răspunde la ea cu `RESULT_UNSUPPORTED`, iar consola nu are butoane pentru ea. Fiecare funcție din etapa 4 o activează local dacă are `allowed_offline`.
4. **Fără rezervă în modul încorporat:** acolo serverul rulează chiar în consolă (§9.6). Fereastra „Local fallback” există doar în modul remote.
5. **Cheia și cache-ul consolei stau lângă `console.toml`** (`%APPDATA%\Guardian\`, 0600), deci câte o înrolare pentru fiecare cont Windows.
   - Spec-ul cere ca cheia să țină de PC, nu de profesor. Asta rămâne adevărat față de *conturile Guardian*: mai mulți profesori folosesc aceeași înrolare.
   - O cheie în `%ProgramData%`, citibilă de toți userii PC-ului, ar putea fi furată de un elev care se loghează pe PC-ul profesorului.
   - Cost dacă e greșit: pe un PC cu conturi Windows separate pentru fiecare profesor, fiecare cont se înrolează o dată.
6. **Un singur port pentru toată școala:**
   - `local_port` în `agent.toml` (implicit 11500, `0` = oprit) și `fallback_port` în `console.toml` (implicit 11500); trebuie să coincidă;
   - nu se schimbă schema serverului;
   - răspunsul de descoperire poartă oricum portul.
7. **Agentul verifică el însuși utilizatorul:** `user_id` + `login` trebuie să fie în `AuthorizedUser`, lista primită de la server (administratorii plus profesorii sălii). Consola doar afirmă identitatea, după ce a verificat parola local.
   - Un profesor scos din sală sau dezactivat înainte de pană e refuzat de agent, chiar dacă verificatorul lui a rămas în cache-ul consolei.
   - Verificatorii nu expiră.
8. **Excepția are prioritate și offline:**
   - `AgentState` primește `exception_profile_id`;
   - un calculator cu excepție refuză schimbarea sălii cu `has_exception`;
   - fără câmpul ăsta, catedra pusă pe „Liber” ar fi blocată de o schimbare locală a sălii.
9. **Schimbarea locală supraviețuiește repornirii agentului** (`local.pb`). Ea se anulează doar când revine serverul (§11.5), nu când calculatorul repornește în timpul penei. E varianta fail-secure: un „Examen” pus local rămâne „Examen”.
10. **Jurnalul local:**
    - conține doar acțiunile acceptate, maximum 1000 de intrări (cele mai vechi se pierd, cu avertisment în log);
    - se trimite după `Welcome`;
    - se șterge doar după `LocalAuditAck`;
    - dacă ack-ul se pierde, intrările se retrimit, deci pot apărea dubluri în `audit_log` (acceptat).

## Global Constraints

- **Licențe:** cele din `deny.toml`. `snow` și dependențele lui (`chacha20poly1305`, `blake2`, `curve25519-dalek`, `rand_core`) sunt MIT/Apache. `cargo deny check` trece în Task 2.
- **`unsafe`:** `#![forbid(unsafe_code)]` în `guardian-local` și în toate modulele noi.
- **Fără `unwrap`/`expect`/`panic` în producție.** Orice octet primit din rețea (cadru Noise, protobuf, datagramă UDP) poate fi gunoi: rezultatul e o eroare sau tăcere, niciodată o panică.
- **`.proto` e contractul.** Câmpurile noi primesc numere noi, iar cele existente nu se schimbă.
- **Valori fixe (spec):**
  - offline = **60 s** fără conexiune autentificată (§10.2);
  - port **11500** TCP + UDP (§11.3);
  - profilurile `free` nu se pot pune local (§11.4);
  - la revenire, starea serverului câștigă (§11.5).
- **Limitele canalului (ale mele):**
  - cadru Noise maximum 65 535 octeți (limita protocolului);
  - handshake maximum **5 s**;
  - conexiune inactivă închisă după **30 s**;
  - maximum **16** conexiuni simultane pe agent;
  - datagramele UDP peste 1 KiB sunt ignorate;
  - maximum **50** de răspunsuri de descoperire pe secundă.
- **Textele refuzurilor (cod → mesaj), trimise de agent în `LocalRefused`:**

  | code | message |
  |---|---|
  | `online` | `This computer is connected to the server; use the server.` |
  | `revoked` | `This computer was removed from the server.` |
  | `unauthorized` | `This user may not control this computer.` |
  | `not_in_room` | `This computer is not in that room.` |
  | `has_exception` | `This computer has its own profile.` |
  | `unknown_profile` | `This profile is not known on this computer.` |
  | `free_profile` | `A free profile cannot be set in fallback mode.` |
  | `bad_request` | `Malformed request.` |

## Review Focus

1. **Adresa IP din cache s-a schimbat (DHCP), iar calculatorul trebuie totuși găsit.** Se trimite broadcast UDP; răspunsul semnat corect se acceptă, unul semnat cu altă cheie se ignoră. Test în Task 7: `stale_address_is_found_by_discovery`.
2. **Un elev deschide multe conexiuni pe 11500 și nu le termină.** Profesorul trebuie să intre totuși (timeout de handshake, maximum 16 conexiuni). Test în Task 5: `half_open_connections_do_not_lock_out_the_console`.
3. **Serverul revine în timpul modului de rezervă cu aceeași versiune de stare.** Agentul tot anulează schimbarea locală, iar consola iese singură din modul de rezervă. Teste: Task 4 `override_is_undone_when_server_returns_with_same_version`, Task 8 `fallback_ends_when_server_returns`.
4. **Calculatorul repornește în timpul penei.** Schimbarea locală și jurnalul rămân și ajung la server. Test în Task 4: `override_and_journal_survive_restart`.
5. **Profesor scos din sală înainte de pană, cu verificatorul încă în consolă.** Agentul refuză cu `unauthorized`. Test în Task 4: `removed_teacher_is_refused_by_the_agent`.

---

### Task 1: Contractul — `local.proto`, câmpurile noi și regulile din `guardian-core`

**Files:**
- Create: `proto/guardian/v1/local.proto`
- Modify: `proto/guardian/v1/{agent,common,api}.proto`, `crates/guardian-proto/build.rs` (adaugă `local.proto` în listă)
- Modify: `crates/guardian-core/src/keys.rs`, `crates/guardian-core/src/lib.rs`
- Create: `crates/guardian-core/src/local.rs`
- Test: unit tests în `keys.rs` și `local.rs`

**Interfaces:**
- Produces, `local.proto` (`package guardian.v1`, importă `common.proto` și `api.proto`):
  ```proto
  message LocalActor { int64 user_id = 1; string login = 2; }
  message LocalSetProfile { int64 room_id = 1; int64 profile_id = 2; }
  message StatusRequest {}
  message LocalRequest {
    string id = 1;                 // UUID, echoed in the reply
    LocalActor actor = 2;
    oneof kind { StatusRequest status = 3; LocalSetProfile set_profile = 4; CommandEnvelope command = 5; }
  }
  message LocalStatus {
    uint64 computer_id = 1; int64 room_id = 2; int64 effective_profile_id = 3;
    bool local_override = 4; uint64 state_version = 5; repeated string capabilities = 6;
  }
  message LocalRefused { string code = 1; string message = 2; }
  message LocalReply {
    string request_id = 1;
    oneof kind { LocalStatus status = 2; LocalRefused refused = 3; CommandResult command_result = 4; }
  }
  message DiscoverRequest { int64 console_id = 1; bytes nonce = 2; bytes signature = 3; }   // nonce: 16 bytes
  message DiscoverReply { uint64 computer_id = 1; bytes nonce = 2; uint32 port = 3; bool online = 4; bytes signature = 5; }
  message LocalAuditEntry {
    uint64 seq = 1; int64 at_unix = 2; int64 console_id = 3; int64 user_id = 4;
    string login = 5; string action = 6; string detail = 7;
  }
  message LocalOverride { int64 room_id = 1; int64 profile_id = 2; int64 set_at_unix = 3; int64 user_id = 4; }
  // Agent: local.pb
  message LocalStore { uint64 computer_id = 1; LocalOverride override = 2; repeated LocalAuditEntry journal = 3; uint64 next_seq = 4; }
  // Console: fallback.pb
  message CachedUser { int64 user_id = 1; string login = 2; Role role = 3; repeated int64 room_ids = 4; string verifier = 5; }
  message CachedComputer {
    uint64 id = 1; string name = 2; int64 room_id = 3; bytes public_key = 4;
    repeated string lan_addresses = 5; int64 exception_profile_id = 6;
  }
  message ConsoleCache {
    string server = 1; int64 console_id = 2; repeated CachedUser users = 3;
    repeated Room rooms = 4; repeated CachedComputer computers = 5; repeated Profile profiles = 6;
  }
  ```
- Produces, câmpuri noi:
  - `AgentState.exception_profile_id = 5` (în `common.proto`; 0 = fără excepție);
  - `Computer.public_key = 12` (`bytes`, în `api.proto`);
  - `AgentFrame.local_audit = 5` cu `message LocalAudit { repeated LocalAuditEntry entries = 1; }`;
  - `ServerFrame.local_audit_ack = 8` cu `message LocalAuditAck { uint64 up_to_seq = 1; }` (`agent.proto` importă `local.proto`).
- Produces, `guardian_core::keys`:
  - `KeyPair::x25519_secret(&self) -> Zeroizing<[u8; 32]>` (= `SigningKey::to_scalar_bytes`);
  - `PublicKey::x25519(&self) -> [u8; 32]` (= `VerifyingKey::to_montgomery().to_bytes()`, `Malformed` dacă punctul nu se decomprimă: `fn x25519(&self) -> Result<[u8; 32], KeyError>`);
  - `KeyPair::sign(&self, ctx: SignContext, msg: &[u8]) -> Signature` și `PublicKey::verify(&self, ctx: SignContext, msg: &[u8], sig: &Signature) -> Result<(), KeyError>`, cu `pub enum SignContext { Discover, DiscoverReply }` și etichetele `b"guardian/v1/discover"` / `b"guardian/v1/discover-reply"` puse în fața mesajului, ca la `AuthContext`.
- Produces, `guardian_core::local`:
  - `pub const OFFLINE_AFTER: Duration = Duration::from_secs(60);`
  - `pub const DEFAULT_LOCAL_PORT: u16 = 11500;`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Refusal { Online, Revoked, Unauthorized, NotInRoom, HasException, UnknownProfile, FreeProfile, BadRequest }`, cu `fn code(self) -> &'static str` și `fn message(self) -> &'static str` (textele din tabelul din Global Constraints) și `fn to_proto(self) -> LocalRefused`;
  - `pub fn is_authorized(users: &[AuthorizedUser], actor: &LocalActor) -> bool` (`user_id` **și** `login` egale);
  - `pub fn check_set_profile(state: &AgentState, users: &[AuthorizedUser], actor: &LocalActor, req: &LocalSetProfile) -> Result<(), Refusal>`, cu ordinea verificărilor: `Unauthorized` → `NotInRoom` (`state.room_id == 0` sau diferit) → `HasException` → `UnknownProfile` → `FreeProfile` (un `Mode` invalid = `UnknownProfile`);
  - `pub fn apply_override(state: &AgentState, ov: Option<&LocalOverride>) -> AgentState`: copia stării cu `effective_profile_id = ov.profile_id` doar dacă override-ul trece aceleași reguli de sală, excepție și profil (nu și de utilizator), altfel starea neschimbată. O stare nouă de la server în care regula nu mai trece face override-ul inert.

- [ ] **Step 1:** Scrie testele.
  - În `keys.rs`:
    - `x25519_halves_match`: `x25519(secret) · basepoint == public.x25519()`. Folosește `curve25519_dalek::MontgomeryPoint::mul_base_clamped` ca dev-dependency a `guardian-core`, sau `snow` cu un DH; alege una și consemnează alegerea.
    - `sign_contexts_do_not_mix`: o semnătură `Discover` nu verifică drept `DiscoverReply`.
  - În `local.rs`:
    - `set_profile_rules`, tabelar, câte un caz pentru fiecare refuz și unul acceptat;
    - `login_must_match_user_id`;
    - `free_is_refused_even_for_admin`;
    - `override_applies_only_in_its_room`;
    - `exception_beats_override`;
    - `stale_override_is_inert` (profilul șters din stare sau calculatorul mutat în altă sală → starea neschimbată).
- [ ] **Step 2:** `cargo test -p guardian-core` — Expected: FAIL (nu compilează: lipsesc `local`, `x25519`, `sign`).
- [ ] **Step 3:** Scrie `.proto`-urile și implementează. Serverul și agentul compilează în continuare: câmpurile noi au valori implicite, iar `match`-urile exhaustive pe `agent_frame::Msg` / `server_frame::Msg` primesc un braț care ignoră variantele noi până la Task 3 / Task 4.
- [ ] **Step 4:** `cargo test -p guardian-core && cargo build --workspace` — Expected: PASS.
- [ ] **Step 5:** Commit `core: add the local fallback contract and its rules`.

### Task 2: `guardian-local` — canalul Noise și descoperirea

**Files:**
- Create: `crates/guardian-local/{Cargo.toml,src/lib.rs,src/channel.rs,src/discover.rs}`
- Modify: `Cargo.toml` (workspace: `snow = "0.10"`, `guardian-local = { path = … }`), `deny.toml` doar dacă e nevoie (cu ruling)
- Test: `crates/guardian-local/tests/{channel,discover}.rs`

**Interfaces:**
- Consumes: `guardian_core::keys::{KeyPair, PublicKey, SignContext}`, mesajele din Task 1.
- Produces, `channel.rs`:
  - `pub const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";`
  - `pub struct Limits { pub handshake: Duration, pub idle: Duration }`, cu `Default` = 5 s / 30 s;
  - `#[derive(Debug, thiserror::Error)] pub enum ChannelError { Io(std::io::Error), Timeout, Handshake, UnknownPeer, FrameTooLarge, Decode(prost::DecodeError), Closed }`;
  - `pub struct LocalStream` (un `TcpStream` + `snow::TransportState`);
  - `pub async fn connect(addr: SocketAddr, me: &KeyPair, console_id: i64, agent: &PublicKey, limits: Limits) -> Result<LocalStream, ChannelError>`. Inițiatorul e consola, iar payload-ul primului mesaj e `console_id` (8 octeți LE).
  - `pub async fn accept(tcp: TcpStream, me: &KeyPair, trusted: &[(i64, PublicKey)], limits: Limits) -> Result<(LocalStream, i64 /*console_id*/), ChannelError>`. Cheia statică a inițiatorului trebuie să fie X25519-ul unei console din `trusted`, iar `console_id` din payload trebuie să fie al aceleiași console. Altfel `UnknownPeer`, iar agentul nu trimite al doilea mesaj.
  - `impl LocalStream { pub async fn send<M: prost::Message>(&mut self, m: &M) -> Result<(), ChannelError>; pub async fn recv<M: prost::Message + Default>(&mut self) -> Result<M, ChannelError> }`:
    - cadru = `u16` big-endian + text cifrat;
    - un mesaj protobuf peste 65 535 − 16 octeți dă `FrameTooLarge` la trimitere;
    - `recv` respectă `limits.idle`.
- Produces, `discover.rs`:
  - `pub fn request(me: &KeyPair, console_id: i64) -> (DiscoverRequest, [u8; 16])`: nonce aleator, semnătură `SignContext::Discover` peste `console_id_le || nonce`;
  - `pub fn verify_request(r: &DiscoverRequest, trusted: &[(i64, PublicKey)]) -> Option<i64>`;
  - `pub fn reply(me: &KeyPair, computer_id: u64, nonce: &[u8], port: u16, online: bool) -> DiscoverReply`: semnătură `DiscoverReply` peste `nonce || computer_id_le || port_le || online as u8`;
  - `pub fn verify_reply(r: &DiscoverReply, nonce: &[u8; 16], agent: &PublicKey) -> bool`;
  - `pub const MAX_DATAGRAM: usize = 1024;`

- [ ] **Step 1:** Scrie testele.
  - `channel.rs`:
    - `handshake_and_round_trip`: consola trimite un `LocalRequest` și agentul răspunde cu un `LocalReply`, peste `127.0.0.1:0`.
    - `unknown_console_is_refused`: `accept` → `UnknownPeer`, iar `connect` primește eroare sau `Closed`.
    - `console_id_must_match_key`: cheia consolei 1 cu payload-ul `console_id = 2` → `UnknownPeer`.
    - `wrong_agent_key_fails`: consola se așteaptă la altă cheie de agent → `Handshake`.
    - `silent_peer_times_out`: un `TcpStream` care nu trimite nimic → `Timeout` în ~`limits.handshake`; testul folosește 200 ms.
    - `garbage_is_an_error_not_a_panic`: 3 octeți aleatori, apoi un cadru de lungime 65 535 cu zgomot.
    - `oversized_message_is_refused`.
  - `discover.rs`:
    - `request_round_trip`;
    - `tampered_request_is_rejected`;
    - `reply_signed_by_other_key_is_rejected`;
    - `reply_for_other_nonce_is_rejected`.
- [ ] **Step 2:** `cargo test -p guardian-local` — Expected: FAIL.
- [ ] **Step 3:** Implementează. `snow::Builder::new(NOISE_PATTERN.parse()?)` cu `local_private_key(&x25519_secret)` și `remote_public_key(&agent.x25519()?)` (inițiator). Responder-ul citește cheia inițiatorului cu `get_remote_static()` după primul mesaj.
- [ ] **Step 4:** `cargo test -p guardian-local && cargo clippy -p guardian-local --all-targets -- -D warnings && cargo deny check` — Expected: PASS, deny ok.
- [ ] **Step 5:** Commit `local: add the Noise channel and signed LAN discovery`.

### Task 3: Serverul — cheia în `Computer`, excepția în stare, jurnalul local

**Files:**
- Modify: `crates/guardian-server/src/db/computers.rs` (`to_proto` completează `public_key`), `src/hub.rs` (`build_agent_state` completează `exception_profile_id`), `src/ws.rs` (`LocalAudit` → audit + ack), `src/batch.rs` (`AuditEntry.at: Option<i64>`), `src/state.rs` (`audit_local`), `src/testing.rs` (`stop` / `StoppedServer::start`, `SimAgent::send_local_audit`)
- Test: `crates/guardian-server/tests/local_audit.rs`

**Interfaces:**
- Produces:
  - `AuditEntry.at: Option<i64>`: `None` = acum (toate apelurile existente), `Some(t)` = ora acțiunii locale.
  - `AppState::audit_local(&self, computer_id: u64, entries: &[LocalAuditEntry])`:
    - pentru fiecare intrare scrie `actor_user_id = Some(user_id)`, `action = entry.action`, `target = "computer:<id>"`, `detail = "<detail> (console <console_id>, <login>)"`, `source = Local`, `at = Some(at_unix)`;
    - textele sunt tăiate la 200 de caractere;
    - se iau maximum 1000 de intrări dintr-un cadru, restul se ignoră cu avertisment în log.
  - `ws.rs`: după `audit_local`, serverul trimite `LocalAuditAck { up_to_seq: max(seq) }`.
  - `testing.rs`:
    - `TestServer::stop(self) -> StoppedServer` (păstrează directorul, config-ul și adresa);
    - `StoppedServer::start(self) -> TestServer` (pe același port, cu reîncercarea de 2 s din `restart_same_port`, care se rescrie pe baza lor);
    - `SimAgent::send_local_audit(&mut self, entries: Vec<LocalAuditEntry>)`.

- [ ] **Step 1:** Scrie testele:
  - `local_audit_is_stored_with_source_local_and_its_time`: `SimAgent` trimite 2 intrări, apoi primește `LocalAuditAck { up_to_seq: 2 }`; `audit_log` are 2 rânduri, cu `source = 'local'` și `at` = valorile trimise.
  - `oversized_journal_is_capped`: 1500 de intrări → 1000 de rânduri, iar ack-ul are `up_to_seq` = seq-ul celei de-a 1000-a intrări.
  - `computer_carries_its_public_key` (`ListComputers` și `Watch` dau cheia cu care s-a înrolat `SimAgent`).
  - `exception_is_in_agent_state`: `UpdateComputer` cu excepție → `ServerFrame.state.exception_profile_id`.
  - `stopped_server_restarts_on_the_same_port`.
- [ ] **Step 2:** `cargo test -p guardian-server --features testing --test local_audit` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** `cargo test -p guardian-server --all-features` — Expected: PASS (toată suita serverului).
- [ ] **Step 5:** Commit `server: accept the local audit journal and send keys and exceptions`.

### Task 4: Agentul — schimbarea locală, jurnalul și revenirea serverului

**Files:**
- Create: `crates/guardian-agent/src/local.rs` (logica, fără rețea)
- Modify: `crates/guardian-agent/src/{agent.rs,conn.rs,store.rs,config.rs}`
- Test: `crates/guardian-agent/tests/local.rs`

**Interfaces:**
- Consumes: `guardian_core::local::*`, `LocalStore`, `LocalAudit`, `LocalAuditAck`, `TestServer::stop`.
- Produces:
  - `AgentPaths::local(&self) -> PathBuf` (`local.pb`, privat ca `state.pb`);
  - `store::{load_local, save_local}`: un `LocalStore` al altui `computer_id` se ignoră, ca la `state.pb`; `enroll` șterge `local.pb`.
  - `AgentOptions.offline_after: Duration`: `OFFLINE_AFTER` în producție; testele folosesc 300 ms; toate construcțiile existente se actualizează.
  - `AgentHandle::local_request(&self, console_id: i64, req: LocalRequest) -> LocalReply`, **singurul punct de intrare** (listener-ul din Task 5 îl apelează):
    - online sau offline de mai puțin de `offline_after` → `Online`;
    - revocat → `Revoked`;
    - `status` → `LocalStatus` (`capabilities` = `CAPABILITIES`);
    - `set_profile` → `check_set_profile`, apoi override, apoi `engine.set_policy(Policy::from_state(&apply_override(..)))`, apoi intrare în jurnal (`action = "local.set_profile"`, `detail = "room <r> → profile <p> (<name>)"`), apoi `save_local` și `LocalStatus` nou;
    - `command` → `LocalReply.command_result` cu `RESULT_UNSUPPORTED`, `detail = "not supported by this agent version"`, fără intrare în jurnal;
    - cerere fără `kind` sau fără `actor` → `BadRequest`.
  - `AgentStatus.local_override: Option<i64>` (profilul pus local).
  - La `ConnEvent::Online`:
    1. dacă jurnalul nu e gol, `conn.send_local_audit(entries)` (metodă nouă pe `Connection`);
    2. override-ul se șterge și se salvează;
    3. starea serverului se aplică **chiar dacă versiunea e aceeași** (`apply_state` primește un `force: bool`).
  - La `ConnEvent::LocalAuditAck(up_to)`: intrările cu `seq <= up_to` se șterg din jurnal.
  - `ConnEvent::State`, primit cât timp există override, nu poate apărea (starea vine doar online). Starea încărcată la pornire trece prin `apply_override`.
  - Jurnalul are maximum 1000 de intrări; a 1001-a o împinge afară pe cea mai veche, cu `tracing::warn!`.

- [ ] **Step 1:** Scrie testele, cu `TestServer` și `FakePlatform`. Setup-ul din `tests/agent.rs`:
  - sala are „Lecție” (blacklist `game.exe`) și „Examen” (whitelist `notes.exe`); există și „Liber”;
  - un profesor e alocat sălii;
  - consola e înrolată prin `EnrollConsole` cu un cod `ENROLL_CONSOLE`.

  Testele:
  - `online_agent_refuses_with_online`.
  - `offline_agent_accepts_set_profile`: `TestServer::stop`, apoi după `offline_after` → `set_profile(Examen)` → `LocalStatus.local_override == true`, iar `FakePlatform` omoară un `other.exe` pornit.
  - `free_profile_is_refused`.
  - `teacher_of_other_room_is_refused`.
  - `removed_teacher_is_refused_by_the_agent`: profesorul e scos din sală cât agentul e online, apoi serverul se oprește → `unauthorized`.
  - `computer_with_exception_refuses`.
  - `command_is_unsupported_and_not_journaled`.
  - `override_and_journal_survive_restart`: `AgentHandle::shutdown`, apoi `run` din nou, cu serverul tot oprit → politica e „Examen” înainte de orice conexiune, iar `local.pb` are o intrare.
  - `server_return_sends_journal_and_undoes_override`: `StoppedServer::start` → `audit_log` are rândul `local.set_profile` cu `source = 'local'`; jurnalul din `local.pb` e gol după ack; politica e din nou „Lecție”.
  - `override_is_undone_when_server_returns_with_same_version`: între oprire și repornire nu se schimbă nimic pe server.
  - `journal_is_capped_at_1000`: test unitar în `local.rs` pe structura jurnalului.
- [ ] **Step 2:** `cargo test -p guardian-agent --test local` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Logica (decizii, jurnal, override) stă în `local.rs`, peste un `&mut LocalStore` și un `&StoredState`, ca să poată fi testată unitar. `agent.rs` doar o leagă de `Live`, `Engine` și `Connection`.
- [ ] **Step 4:** `cargo test -p guardian-agent --all-features` — Expected: PASS (toată suita agentului de pe acest OS).
- [ ] **Step 5:** Commit `agent: switch profiles locally while offline and report it on return`.

### Task 5: Agentul — listener-ul TCP/UDP pe 11500

**Files:**
- Create: `crates/guardian-agent/src/listener.rs`
- Modify: `crates/guardian-agent/src/{agent.rs,config.rs,main.rs}`, `crates/guardian-agent/Cargo.toml` (`guardian-local`), `packaging/agent-linux-v3/install.sh` (comentariu despre port; fără firewall pe Linux)
- Test: `crates/guardian-agent/tests/listener.rs`

**Interfaces:**
- Consumes: `guardian_local::{channel::accept, discover}`, `AgentHandle::local_request`.
- Produces:
  - `AgentConfig.local_port: Option<u16>`: lipsă = `DEFAULT_LOCAL_PORT`, `0` = rezerva oprită. Câmpul e `#[serde(default)]`, ca `agent.toml`-urile din etapa 2 să se citească în continuare.
  - `pub struct LocalOptions { pub tcp: SocketAddr, pub udp: SocketAddr, pub limits: guardian_local::channel::Limits }` și `AgentOptions.local: Option<LocalOptions>`. Producția folosește `0.0.0.0:<local_port>` pentru ambele, cu `Limits::default()`. Testele folosesc `127.0.0.1:0`, cu limite scurte.
  - `AgentHandle::local_addrs(&self) -> Option<(SocketAddr, SocketAddr)>` (porturile legate efectiv).
  - Comportamentul listener-ului:
    - un `Semaphore` de 16 conexiuni; peste el conexiunea nouă se închide imediat;
    - fiecare conexiune: `accept` cu `Limits::default()` și cheile `stored.consoles` din momentul conexiunii, apoi o buclă `recv::<LocalRequest>` → `local_request` → `send`, până la eroare sau `idle`;
    - UDP: o datagramă peste `MAX_DATAGRAM` sau nedecodabilă se ignoră; o cerere care nu verifică se ignoră; altfel `DiscoverReply` trimis înapoi la adresa sursă, cu `online` = conexiune activă la server;
    - limita de 50 de răspunsuri pe secundă se aplică printr-un contor resetat în fiecare secundă;
    - un port ocupat la pornire nu oprește agentul: `tracing::error!` și agentul rulează fără rezervă.
  - Fiecare conexiune refuzată (`UnknownPeer`, `Timeout`) se scrie în log la nivel `debug`, ca un elev să nu poată umple jurnalul.

- [ ] **Step 1:** Scrie testele:
  - `console_switches_profile_over_tcp`: canalul real, peste `127.0.0.1`, cu serverul oprit.
  - `unknown_console_gets_nothing`.
  - `half_open_connections_do_not_lock_out_the_console`: 20 de `TcpStream` deschise și tăcute; consola cunoscută se conectează și primește `status` în maximum `handshake + 1 s`, cu `handshake` = 300 ms în test. Testul reîncearcă conectarea la 50 ms: cât sunt ocupate cele 16 locuri, conexiunile noi se închid imediat, iar locurile se eliberează după timeout-ul de handshake.
  - `discovery_answers_known_console_only`.
  - `oversized_datagram_is_ignored`.
  - `busy_port_does_not_stop_the_agent`: portul TCP e ținut ocupat de test → agentul pornește, `local_addrs() == None`, iar politica se aplică.
  - `config_without_local_port_still_loads` (un `agent.toml` din etapa 2).
- [ ] **Step 2:** `cargo test -p guardian-agent --test listener` — Expected: FAIL.
- [ ] **Step 3:** Implementează; `main.rs` (`run` și serviciul Windows) construiește `LocalOptions` din `local_port`. `guardian-agent status` arată și `local fallback: listening on :11500` / `off` / `port busy`.
- [ ] **Step 4:** `cargo test -p guardian-agent --all-features && cargo clippy -p guardian-agent --all-targets --all-features -- -D warnings`, plus clippy pe ambele ținte Windows — Expected: PASS.
- [ ] **Step 5:** Commit `agent: listen for teacher consoles on the local network`.

### Task 6: Consola — cheia, înrolarea și cache-ul de rezervă

**Files:**
- Create: `crates/guardian-console/src/fallback/{mod.rs,cache.rs,key.rs}`
- Modify: `crates/guardian-console/src/{lib.rs,settings.rs}`, `crates/guardian-console/Cargo.toml` (`guardian-core`, `guardian-local`, `argon2`, `prost`)
- Test: `crates/guardian-console/tests/fallback_cache.rs`

**Interfaces:**
- Consumes: `Fleet` (3a), `User`, `ConsoleCache`, `KeyPair`.
- Produces:
  - `settings`: `ConsoleSettings::fallback_port() -> u16` (cheia `fallback_port`, implicit 11500) și `settings::fallback_paths(settings_path: &Path) -> (PathBuf /*console.key*/, PathBuf /*fallback.pb*/)`, în același director, ambele 0600.
  - `key.rs`: `pub fn load_or_create(path: &Path) -> Result<KeyPair, FallbackError>`.
  - `cache.rs`:
    - `pub struct FallbackCache(ConsoleCache)`, cu `load(path, server: &str) -> Option<FallbackCache>` (un cache al altui server dă `None`) și `save(&self, path) -> Result<(), FallbackError>`;
    - `pub fn new(server: &str, console_id: i64) -> FallbackCache`, `console_id()`;
    - `pub fn remember_login(&mut self, user: &User, password: &Secret) -> Result<(), FallbackError>`: verificator argon2id PHC cu parametrii impliciți ai crate-ului (m = 19456 KiB, t = 2, p = 1); actualizează rolul și sălile;
    - `pub fn verify(&self, login: &str, password: &Secret) -> Option<CachedUser>`: login fără diferență de majuscule, parola verificată în timp constant de `argon2`; un login necunoscut dă `None` după un calcul argon2 fals, ca timpul să nu-l trădeze;
    - `pub fn update_user(&mut self, user: &User)` (rol și săli, fără parolă: sesiune reluată din token);
    - `pub fn merge_fleet(&mut self, fleet: &Fleet)`: săli, profiluri și calculatoare fac upsert după id, iar ce nu se vede nu se șterge, pentru că alt profesor poate avea alte săli. Calculatoarele fără `public_key` se ignoră. Excepție: un `computer_removed` / `room_removed` venit prin `Watch` șterge, deci `Fleet` expune `removed_ids()` sau backend-ul apelează `forget_computer` / `forget_room`; alege și consemnează.
    - `pub fn rooms_for(&self, user: &CachedUser) -> Vec<Room>` (adminul vede toate sălile);
    - `pub fn computers_in(&self, room_id: i64) -> Vec<CachedComputer>`;
    - `pub fn fleet_for(&self, user: &CachedUser, reach: &BTreeMap<u64, Reach>) -> Fleet`: `Fleet` din 3a, construit pentru UI;
    - `pub enum Reach { Unknown, Reachable(LocalStatus), Unreachable, StillOnline, Refused(String) }`;
    - `pub fn note(r: &Reach) -> &str`:

      | Reach | Text |
      |---|---|
      | `Unknown` | `Checking…` |
      | `Reachable` | `Reachable locally` |
      | `Unreachable` | `Not reachable` |
      | `StillOnline` | `Still connected to the server` |
      | `Refused(m)` | `m` |

  - `Fleet` (3a) primește:
    - `Fleet::from_parts(rooms: Vec<Room>, computers: Vec<Computer>, profiles: Vec<Profile>) -> Fleet`;
    - `Fleet::set_notes(&mut self, notes: BTreeMap<u64, String>)`: o notă înlocuiește `ComputerView.connection`.
    - În `fleet_for`, `online` = `Reachable`, iar `effective_profile_id` vine din `LocalStatus`.

- [ ] **Step 1:** Scrie testele:
  - `verifier_accepts_right_password_only`;
  - `unknown_login_is_none`;
  - `cache_of_other_server_is_ignored`;
  - `merge_keeps_rooms_of_other_teachers`;
  - `removed_computer_is_forgotten`;
  - `computers_without_key_are_skipped`;
  - `teacher_sees_only_their_rooms_admin_sees_all`;
  - `fleet_for_marks_reachable_online_with_local_profile`;
  - `notes_replace_connection_text`;
  - `cache_round_trips_and_is_private` (0600 pe Unix);
  - `key_is_created_once`.
- [ ] **Step 2:** `cargo test -p guardian-console --features testing --test fallback_cache` — Expected: FAIL.
- [ ] **Step 3:** Implementează (logică pură plus fișiere, fără rețea și fără egui).
- [ ] **Step 4:** Același test plus `--test fleet` — Expected: PASS.
- [ ] **Step 5:** Commit `console: keep the fallback key and the offline cache`.

### Task 7: Consola — clientul local (probe, schimbare, descoperire)

**Files:**
- Create: `crates/guardian-console/src/fallback/client.rs`
- Modify: `crates/guardian-console/Cargo.toml` (dev: `guardian-agent`, `guardian-platform` cu `testing`, `guardian-core`)
- Test: `crates/guardian-console/tests/fallback_client.rs`

**Interfaces:**
- Consumes: `guardian_local::{channel::connect, discover}`, `CachedComputer`, `CachedUser`, `Reach`.
- Produces:
  - `pub struct LocalClient { key: Arc<KeyPair>, console_id: i64, port: u16, limits: Limits, broadcast: Vec<SocketAddr> }`;
  - `LocalClient::new(..)`; `broadcast` implicit = `255.255.255.255:port` plus adresa de broadcast a fiecărei interfețe IPv4 (`if_addrs`), iar testele dau `127.0.0.1:<udp>`;
  - `pub async fn probe(&self, c: &CachedComputer, actor: &CachedUser, addr_hint: Option<SocketAddr>) -> (Reach, Option<SocketAddr>)`:
    - încearcă pe rând `addr_hint` și apoi fiecare `lan_addresses:port`, fiecare cu un timeout de conectare de 2 s;
    - `LocalRefused{code:"online"}` → `StillOnline`;
    - alt refuz → `Refused(message)`;
    - întoarce adresa care a mers;
  - `pub async fn discover(&self, wait: Duration) -> BTreeMap<u64, SocketAddr>`: o cerere semnată pe fiecare adresă de broadcast; adună răspunsurile valide (verificate cu cheia din cache) până la `wait`; adresa e sursa datagramei cu portul din răspuns;
  - `pub async fn set_profile(&self, addr: SocketAddr, c: &CachedComputer, actor: &CachedUser, room_id: i64, profile_id: i64) -> Reach`;
  - `pub async fn sweep(&self, computers: &[CachedComputer], actor: &CachedUser, known: &BTreeMap<u64, SocketAddr>) -> BTreeMap<u64, (Reach, Option<SocketAddr>)>`: maximum 32 de probe în paralel; pentru calculatoarele `Unreachable` rulează un `discover(1 s)` și le reîncearcă o dată pe adresa găsită.

- [ ] **Step 1:** Scrie testele, cu agenți reali din `guardian-agent` (librărie, `FakePlatform`), `local: Some(127.0.0.1:0)` și `TestServer` oprit după înrolare:
  - `probe_reads_status`;
  - `probe_of_online_agent_says_still_online`;
  - `set_profile_applies` (`FakePlatform` vede politica nouă);
  - `stale_address_is_found_by_discovery`: `CachedComputer.lan_addresses = ["127.0.0.2"]` (nimic nu ascultă acolo) → `sweep` îl găsește prin UDP pe `127.0.0.1`;
  - `reply_from_impostor_is_ignored`: un responder UDP de test semnează cu altă cheie;
  - `unreachable_is_unreachable_within_3s`.
- [ ] **Step 2:** `cargo test -p guardian-console --features testing --test fallback_client` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** Același test — Expected: PASS.
- [ ] **Step 5:** Commit `console: talk to agents directly on the local network`.

### Task 8: Consola — backend-ul în modul de rezervă

**Files:**
- Create: `crates/guardian-console/src/backend/fallback.rs`
- Modify: `crates/guardian-console/src/backend/{mod.rs,conn.rs,actions.rs}`
- Test: `crates/guardian-console/tests/backend_fallback.rs`

**Interfaces:**
- Consumes: Task 6 și Task 7, `Core` din 3a.
- Produces, `Action` noi:
  - `EnableFallback { code: Option<String> }`:
    - `None` = adminul creează el codul (`CreateEnrollCode { kind: ENROLL_CONSOLE, max_uses: 1, ttl_seconds: 900 }`);
    - apoi `EnrollConsole { code, public_key, name: hostname }`;
    - apoi creează și salvează cache-ul.
  - `FallbackLogin { login: String, password: Secret }`: verificare locală, apoi intrare în modul de rezervă.
  - `EnterFallback`: utilizatorul deja logat online intră direct, cu identitatea lui. E permis doar cu cache și cu `ConnState::Reconnecting`.
  - `LeaveFallback`.
- Produces, `Event` nou și starea afișată:
  - `Event::Fallback(FallbackState)`, cu `pub enum FallbackState { Off, Available, Active { user: CachedUser, fleet: Fleet } }`;
  - `Available` = modul remote, cache cu cel puțin un verificator, serverul nu răspunde (`Reconnecting`, sau `Connecting` cu cel puțin o încercare eșuată);
  - `Event::FallbackEnabled(Option<i64>)` = id-ul consolei sau `None`, emis la pornire și după `EnableFallback`.
- Produces, comportament:
  - **Cât e online și înrolată:** la `Login` reușit, `remember_login`; la `WhoAmI` reușit, `update_user`. La fiecare `Fleet`, `merge_fleet`, cu salvare când conținutul s-a schimbat, cel mult o dată la 5 s.
  - **În `Active`:**
    - o buclă `sweep` la 10 s pe calculatoarele sălilor utilizatorului emite `Fallback(Active{..})` după fiecare rundă;
    - `SetRoomProfile` merge pe calea locală: `set_profile` paralel pe fiecare calculator al sălii, apoi `Event::Done("<Profile> applied on N of M computers in <Room>.")`, cu adaosul `" K could not be reached."` când `K > 0`;
    - un profil `free` e refuzat chiar de backend, înainte de rețea, cu `Event::Error("A free profile cannot be set in fallback mode.")`.
  - **Reconectarea la server continuă în fundal** (bucla din 3a nu se oprește). Primul `ConnState::Online` oprește bucla `sweep` și emite `Fallback(Off)` și `Event::Done("The server is back; fallback mode ended.")`.
  - **`ForgetMode` și schimbarea serverului** șterg `fallback.pb` și `console.key`. Consola rămâne înrolată pe server, iar adminul o poate revoca.
  - **În modul încorporat:** `FallbackState::Off` mereu, iar `EnableFallback` dă `Error("Local fallback is only for a remote server.")`.
- Mesaje de eroare:
  - `FallbackLogin` greșit: `"Unknown login or wrong password. Only users who have signed in on this computer while the server was reachable can work locally."`;
  - cod invalid: textul serverului (3a: `status_text`).

- [ ] **Step 1:** Scrie testele, cu `RealBackend` contra `TestServer` plus 2 agenți-librărie cu `local` pe `127.0.0.1` (helper-ul din `tests/common` din 3a, extins):
  - `admin_enables_fallback_in_one_step` (`ListConsoles` are consola);
  - `teacher_enables_with_a_code`;
  - `fallback_is_available_only_when_server_is_down`;
  - `teacher_signs_in_locally_and_switches_room`: serverul oprit → `FallbackLogin` → `SetRoomProfile(Examen)` → `Done("Examen applied on 2 of 2 computers in Sala 1.")`, iar politica ambilor agenți e „Examen”;
  - `signed_in_user_enters_without_password` (`EnterFallback`);
  - `wrong_password_is_refused_locally`;
  - `free_is_refused_before_the_network`;
  - `fallback_ends_when_server_returns`: `StoppedServer::start` → `Fallback(Off)`, `ConnState::Online`, iar agenții revin la „Lecție”;
  - `embedded_mode_has_no_fallback`;
  - `forget_mode_deletes_key_and_cache`.
- [ ] **Step 2:** `cargo test -p guardian-console --features testing --test backend_fallback` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Acțiunile noi sunt „de ciclu de viață” și rulează în ordine, ca `Login` (regula din fix pass-ul 3a), sub `catch_unwind`.
- [ ] **Step 4:** `cargo test -p guardian-console --features testing` — Expected: PASS (toată suita consolei).
- [ ] **Step 5:** Commit `console: work directly with the computers while the server is down`.

### Task 9: Consola — UI-ul modului de rezervă

**Files:**
- Create: `crates/guardian-console/src/ui/fallback.rs`
- Modify: `crates/guardian-console/src/ui/{mod.rs,login.rs,rooms.rs,status.rs}`, `crates/guardian-console/src/testing.rs` (`FakeBackend` emite evenimentele noi)
- Test: `crates/guardian-console/tests/ui_fallback.rs`

**Interfaces:**
- Consumes: `Event::{Fallback, FallbackEnabled}`, `Action::{EnableFallback, FallbackLogin, EnterFallback, LeaveFallback, SetRoomProfile}`.
- Produces (textele exacte):
  - **Bara de jos (modul remote):** buton `Local fallback…`, care deschide fereastra `Local fallback`:
    - neînrolat: `Not enabled on this computer.`, câmpul `Console code` și butonul `Enable`; adminul are în plus butonul `Enable (create a code)`;
    - înrolat: `Enabled (console #<id>). Users who have signed in here can work locally when the server is down.`
  - **`FallbackState::Available`:**
    - pe ecranul de sign-in apare butonul `Work locally (fallback mode)…`, care deschide `Sign in to fallback mode` (login și parolă, `Sign in`);
    - pe ecranul principal, în `Reconnecting`, bannerul din 3a primește butonul `Work locally (fallback mode)` → `EnterFallback`.
  - **`FallbackState::Active`:**
    - banner chihlimbariu: `Fallback mode: the server cannot be reached. Changes go straight to the computers on this network and are undone when the server is back.`, cu butonul `Leave fallback mode`;
    - ecranul de săli din 3a se desenează din `fleet`-ul primit;
    - combo-ul de profil nu conține profilurile `free`, deci nu există nici confirmarea „free”;
    - tab-urile de admin sunt ascunse, la fel `Connection info…` și `Change password…`;
    - bara de jos arată `Signed in locally as <login>`.

- [ ] **Step 1:** Scrie testele, cu kittest și `FakeBackend`:
  - `fallback_window_enables_with_code` (trimite `EnableFallback{code:Some("ABCD-EFGH")}`);
  - `admin_sees_create_a_code`;
  - `sign_in_screen_offers_fallback_when_available`;
  - `local_sign_in_sends_fallback_login`;
  - `active_fallback_shows_banner_and_hides_admin_tabs`;
  - `free_profile_is_not_offered_in_fallback`;
  - `switch_in_fallback_sends_set_room_profile`;
  - `notes_are_shown_per_computer` (`Not reachable`);
  - `leave_button_sends_leave`;
  - `embedded_mode_has_no_fallback_button`.
- [ ] **Step 2:** `cargo test -p guardian-console --features testing --test ui_fallback` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Numele accesibile urmează regula din 3a (`widget_info` cu numele rândului), iar simbolurile se limitează la cele din fonturile egui.
- [ ] **Step 4:** `cargo test -p guardian-console --features testing` — Expected: PASS.
- [ ] **Step 5:** Commit `console: show fallback mode and its sign-in`.

### Task 10: Prima utilizare reală — server oprit, profil schimbat local

**Files:**
- Create: `crates/guardian-console/tests/real_fallback.rs` (`#![cfg(unix)]`)

**Interfaces:**
- Consumes:
  - binarele `guardian-server` și `guardian-agent` (`cargo build --workspace --bins`), ca în `real_embedded` din 3a;
  - `RealBackend`.

- [ ] **Step 1:** Scrie testul `real_agent_follows_the_console_while_the_server_is_down`:
  1. pornește binarul `guardian-server` (TLS self-signed, port liber) și îl inițializează prin `guardian-cli init` sau gRPC;
  2. creează o sală cu „Lecție” (blacklist `guardian-test-game`) și un profesor;
  3. înrolează binarul `guardian-agent` cu `local_port` liber în `agent.toml` și îl pornește;
  4. consola (`RealBackend`) se loghează ca profesor, apoi `EnableFallback` cu un cod creat de admin;
  5. oprește serverul (kill) și așteaptă `Fallback(Available)`;
  6. `FallbackLogin`, apoi `SetRoomProfile(Examen)` → `Done(... "1 of 1" ...)`;
  7. pe Linux, o copie a lui `sleep` numită `guardian-test-other` e omorâtă sub „Examen”; pe macOS testul se oprește la `Done`, ca în 3a;
  8. repornește serverul pe același port → agentul revine la „Lecție”, iar `audit_log` (citit cu `sqlite` din directorul serverului, read-only) are `local.set_profile` cu `source = 'local'`.
- [ ] **Step 2:** `cargo build --workspace --bins && cargo test -p guardian-console --features testing --test real_fallback` — Expected: PASS, pe macOS și în containerul Linux. Testul confirmă ce au construit deja task-urile anterioare, deci un FAIL înseamnă un bug de integrare: se depanează cu superpowers:systematic-debugging.
- [ ] **Step 3:** Commit `console: cover fallback mode with the real server and agent`.

### Task 11: CI și documentația

**Files:**
- Modify: `.github/workflows/rust.yml` (jobul `linux` rulează și `real_fallback`; jobul `windows` rulează testele `guardian-local` și `guardian-agent --test listener`)
- Create: `specs/local-fallback-v3.md`
- Modify:
  - `specs/agent-v3.md` (listener-ul, `local.pb`, `local_port`);
  - `specs/console-v3.md` (fereastra, cache-ul, fișierele noi în tabel; scoate limitarea care nu mai e adevărată);
  - `specs/server-v3.md` (`LocalAudit` / `LocalAuditAck`, `Computer.public_key`, `AgentState.exception_profile_id`);
  - `specs/2026-11-17-rust-rewrite-design.md` §14 (nota „3b făcută pe <data>”);
  - `AGENTS.md` (tabelul crate-urilor: `guardian-local`; etapa 3b făcută);
  - `/Users/poligon/Projects/PROJECTS.md` (secțiunea Guardian și data de sus).

- [ ] **Step 1:** Scrie `local-fallback-v3.md`, cu secțiunile:
  - când se activează;
  - cheile și canalul (Noise IK, conversia cheilor, limitele);
  - descoperirea;
  - ce se poate face local (doar profilul, în 3b; tabelul refuzurilor);
  - revenirea serverului;
  - fișierele (`local.pb`, `console.key`, `fallback.pb`);
  - pașii manuali pe Windows:
    - regula de firewall pentru 11500 TCP/UDP, pe care o creează installer-ul din 3c; până atunci se face manual cu `netsh`, cu comanda exactă în document;
    - doi profesori pe același PC;
    - IP schimbat prin DHCP;
  - limitările:
    - comenzile one-shot vin în etapa 4;
    - o consolă revocată nu află că e revocată, agenții doar o refuză;
    - pot apărea dubluri în `audit_log` dacă ack-ul se pierde;
    - un port pentru toată școala.
- [ ] **Step 2:** Rulează toată verificarea:
  - `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && cargo deny check`;
  - clippy pe ambele ținte Windows;
  - suita în containerul Linux, inclusiv `real_linux` și `real_fallback`.

  Expected: totul trece.
- [ ] **Step 3:** Commit `docs: describe local fallback mode`. `PROJECTS.md` e în afara repo-ului, deci nu intră în commit.
