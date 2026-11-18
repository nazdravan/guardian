# Guardian 3.0 — Etapa 3a: consola profesorului — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `guardian-console`, aplicația egui a profesorului (urmașul `tools/server-gui`). Se conectează la un server la distanță sau pornește serverul încorporat pe PC-ul profesorului cât e deschisă. Arată sălile, calculatoarele și profilurile live. Profesorul schimbă profilul sălii dintr-un clic, iar administratorul gestionează săli, profiluri, calculatoare, utilizatori, coduri de înrolare și setări.

**Architecture:** Crate nou `crates/guardian-console` (librărie + binar), în straturi:
- **Fără egui:** `settings` (fișierul consolei), `fleet` (starea live construită din evenimentele `Watch`, plus textele derivate), `connect_info` (ce i se dă agentului) și `embedded` (pornirea serverului din `guardian-server` în proces, instanța unică, firewall-ul).
- **`backend`:** rulează pe un fir propriu, cu runtime tokio. Primește `Action`, trimite `Event` și execută apelurile gRPC prin `guardian-client`.
- **`ui`:** desenează doar din `Event`-uri și trimite doar `Action`-uri. Vorbește cu backend-ul prin trait-ul `Backend`, ca testele `egui_kittest` să ruleze cu un backend fals, fără server și fără ecran.

Serverul și protocolul nu se schimbă: `Watch` dă deja tot ce arată consola.

**Tech Stack:**
- egui/eframe **0.37** (render wgpu, cu revenire la glow) și `egui_kittest` 0.37;
- `rfd` 0.17 pentru dialogurile de eroare de dinaintea ferestrei;
- `winresource` pentru iconiță, manifest și VERSIONINFO;
- cele din etapele 1–2: `tonic`, `guardian-client`, `guardian-server` ca librărie, `tokio`, `serde`/`toml`, `tracing`, `if-addrs`, `gethostname`.

**Spec:**
- [`specs/2026-11-17-rust-rewrite-design.md`](../2026-11-17-rust-rewrite-design.md) §2, §3 punctul 6, §8.3, §9.2, §9.5, §9.6, §14 punctul 3 (partea „guardian-console").
- Predecesorul, ale cărui decizii de UX se păstrează unde nu contrazic 3.0: [`specs/2026-11-13-windows-server-gui-design.md`](../2026-11-13-windows-server-gui-design.md).
- Serverul: [`specs/server.md`](../server.md) (roluri, `Watch`). Agentul: [`specs/agent.md`](../agent.md) (comanda `enroll`).

## Etapa 3 e împărțită în trei planuri

§14 punctul 3 cuprinde trei subsisteme independente. Fiecare primește planul lui, în ordinea asta:
- **3a (acest plan):** consola.
- **3b:** modul de rezervă local (§11). Cuprinde înrolarea consolei cu cheie ed25519, canalul Noise agent ↔ consolă, descoperirea în LAN, comutarea profilului fără server și jurnalul local sincronizat la revenire. Are nevoie de consola din 3a.
- **3c:** `guardian-setup` (după `rust:specs/2026-10-09-console-architecture-design.md`, adaptat la agentul Rust), instalatorul agentului pe Windows, arhiva de release 3.0, CI-ul de release, ștergerea codului Go și documentația de release (§17).

## Înainte de a începe

- **Branch:** se lucrează pe `rust-rewrite`, unde sunt etapele 1 și 2. Fără `feat` în nume și mesaje. Commit-urile sunt `<crate sau zonă>: <propoziție>`, fără trailer-e de atribuire.
- **Platforme de test:** consola e o aplicație Windows, dar tot ce nu e `#[cfg(windows)]` se testează pe macOS și în containerul Linux din etapa 2:
  - `fleet`, `settings`, `connect_info`, `backend` (contra `TestServer`), serverul încorporat și UI-ul cu `egui_kittest`;
  - pe Windows: `cargo clippy --target x86_64-pc-windows-gnu` și `i686-pc-windows-gnu` local, iar build-ul `i686-pc-windows-msvc` în CI.
- **UI-ul e în engleză**, ca predecesorul și ca restul uneltelor (decizia proprietarului din `server-gui`).

## Global Constraints

- **MSRV:** urcă de la 1.88 la **1.95** (`rust-version` din `Cargo.toml`), pentru că egui/eframe/egui_kittest 0.37 cer 1.95. E singura schimbare în crate-urile existente; tot CI-ul rulează pe `stable`.
- **Licențe:** doar cele din `deny.toml`. `cargo deny check` trece în fiecare task care adaugă dependențe; dacă un crate din arborele egui/wgpu aduce o licență nouă permisivă (de exemplu `Unicode-3.0`, `Zlib`, `BSL-1.0`), se adaugă în `deny.toml` cu un ruling. GPL și MPL rămân interzise.
- **`unsafe`:** interzis în `guardian-console` (`#![forbid(unsafe_code)]`), fără excepții. Tot ce ține de Windows trece prin `std::process::Command` (`netsh`, `icacls`, `powershell Start-Process -Verb RunAs`) sau prin crate-uri sigure.
- **Panici:** fără `unwrap`/`expect`/`panic` în cod de producție. Fiecare `Action` din backend rulează sub `std::panic::catch_unwind` (prin `futures_util::FutureExt::catch_unwind` cu `AssertUnwindSafe`). O panică devine `Event::Error("internal error: …")` scrisă în log, iar fereastra nu cade.
- **Ținta Windows:** `i686-pc-windows-msvc`, un singur `.exe` de 32 de biți care merge și pe Windows de 64. CRT static (`.cargo/config.toml`: `[target.i686-pc-windows-msvc] rustflags = ["-C", "target-feature=+crt-static"]`).
- **Binarul:** `guardian-console.exe`, cu `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
- **Manifest:** `asInvoker` (un profesor fără drepturi de administrator poate folosi consola), `PerMonitorV2`, `supportedOS` Windows 10. Pașii care cer administrator (regula de firewall) se fac printr-un proces separat, ridicat explicit.
- **Moduri exclusive (§9.6):**
  - **Remote:** URL `https://host:port` + amprenta opțională;
  - **Embedded:** serverul pornește la deschiderea aplicației și se oprește curat la închidere, ascultă pe `0.0.0.0:<port>` (implicit **8443**), iar consola vorbește cu el pe `https://127.0.0.1:<port>`, cu amprenta certificatului self-signed.
- **Fișiere:**

  | | Windows | Alte OS (dezvoltare) |
  |---|---|---|
  | Setările consolei `console.toml` (0600) | `%APPDATA%\Guardian\console.toml` | `<config dir>/guardian/console.toml` (aceeași funcție ca `guardian-cli`) |
  | Datele serverului încorporat | `%ProgramData%\Guardian Server\` | `<data dir>/guardian-server/` |
  | Log | `%LOCALAPPDATA%\Guardian\console.log` (rotit în `.1`, `.2` la pornire, maxim 1 MiB) | `<cache dir>/guardian/console.log` |

  ACL-ul datelor încorporate pe Windows: SYSTEM, Administrators și userul care le-a creat (owner = acel user), moștenire oprită. Conțin cheia TLS și baza.
- **`console.toml`:** `mode` (`remote` | `embedded`), `server` (doar remote), `cert_sha256` (doar remote), `port` (doar embedded, implicit 8443), `login` (ultimul login), `token` (sesiunea; ștearsă la logout și la `UNAUTHENTICATED`). Tokenul nu apare niciodată în log, în `Debug` sau în UI.
- **Instanță unică:** fișierul de lock `<directorul setărilor>/console.lock`, luat cu `std::fs::File::try_lock` (Rust 1.89+). A doua pornire arată dialogul `rfd` „Guardian Console is already running.” și iese cu codul 0.
- **Firewall (doar Windows, doar embedded):** regula `Guardian Console server`, TCP inbound pe port, `profile=any`, `remoteip=localsubnet`, creată cu `netsh advfirewall firewall add rule …`. Dacă procesul nu e ridicat, apare un banner cu butonul **Open firewall port…**, care rulează aceeași comandă prin `powershell -NoProfile -Command Start-Process netsh -Verb RunAs -Wait -ArgumentList '<args>'`.
- **Reconectare la server:** backoff de la 1 s la 30 s. Cât lipsește conexiunea, ultima stare rămâne vizibilă, marcată „Reconnecting…”.
- **Online:** câmpul `online` din `Computer` (serverul îl știe din WebSocket), fără nicio regulă proprie de timp. „Offline · N min ago” vine din `last_seen_unix`.
- **Roluri (§9.2):** un `teacher` vede doar sălile lui (serverul filtrează `Watch`) și poate schimba doar profilul lor. Panourile de administrare apar doar pentru `admin`. Verificarea reală rămâne pe server; un `PERMISSION_DENIED` se arată ca eroare, nu se ascunde.
- **Siguranță (anti-obiectivul „calculator liber”):** schimbarea profilului unei săli pe unul cu mod `free` cere confirmare („All N computers in <sală> will run anything. Continue?”, butonul implicit **No**). Tot cu confirmare: revocarea unui calculator, ștergerea unei săli, a unui profil sau dezactivarea unui utilizator.
- **Comenzile one-shot** (mesaj, oprire…) **nu au UI în 3a.** Agenții anunță `capabilities = []` până la etapa 4, deci orice comandă ar primi „unsupported”. Fiecare funcție din etapa 4 își aduce butonul. *(Abatere conștientă de la textul §14 punctul 3, care listează „comenzi”.)*

## Review Focus

1. **Serverul încorporat nu pornește** (portul ocupat de alt server Guardian sau de alt program, director de date fără drepturi). Profesorul trebuie să vadă un card roșu cu motivul în cuvinte simple („Port 8443 is already in use…”), niciodată „Running”. → Task 5, `port_in_use_is_reported_and_never_running`.
2. **Sesiunea expiră sau utilizatorul e dezactivat cât fereastra e deschisă** (`UNAUTHENTICATED` pe `Watch` sau pe un apel). Consola trebuie să ceară login din nou, cu un mesaj, fără să se blocheze în „Reconnecting…” la nesfârșit și fără să păstreze tokenul. → Task 4, `expired_session_asks_for_login_again`.
3. **Serverul la distanță cade și revine** (internetul școlii). Ultima stare trebuie să rămână vizibilă cu „Reconnecting…”, iar la revenire `Snapshot`-ul nou o înlocuiește complet (calculatoare șterse între timp dispar). → Task 4, `server_restart_reconnects_and_replaces_the_snapshot`.
4. **Un clic greșit pe „Liber” în timpul unui examen.** Comutarea pe un profil `free` cere confirmare, iar „No” nu trimite nimic. → Task 8, `free_profile_needs_confirmation`.
5. **Un profesor cu rol `teacher` deschide consola.** Nu vede panourile de administrare și nici săli care nu sunt ale lui. Dacă totuși trimite o acțiune refuzată, vede eroarea serverului. → Task 8, `teacher_sees_only_room_controls` + Task 4, `permission_denied_is_shown`.

---

### Task 1: Crate-ul `guardian-console` și dependențele egui

**Files:**
- Modify: `Cargo.toml` (`rust-version = "1.95"`, workspace deps `egui = "0.37"`, `eframe = { version = "0.37", default-features = false, features = ["default_fonts", "accesskit", "wgpu", "glow", "x11", "wayland"] }`, `egui_kittest = "0.37"`, `rfd = "0.17"`, `winresource = "0.1"`, `guardian-console = { path = "crates/guardian-console" }`), `deny.toml` (doar dacă e nevoie, cu ruling), `.cargo/config.toml` (CRT static pentru `i686-pc-windows-msvc`)
- Create: `crates/guardian-console/Cargo.toml`, `src/lib.rs`, `src/main.rs` (temporar: tipărește versiunea), `build.rs` (gol pe non-Windows; `winresource` cu iconița `assets/guardian.ico` și manifestul de mai jos pe Windows)
- Create: `crates/guardian-console/assets/guardian.ico` (copiată din `tools/server-gui`, dacă există acolo, altfel din `docs/assets`), `assets/guardian-console.manifest`
- Test: `crates/guardian-console/tests/smoke.rs`

**Interfaces:**
- Produces: `guardian_console::VERSION: &str` (`CARGO_PKG_VERSION`); crate-ul compilează pe macOS, Linux și ambele ținte Windows.

- [ ] **Step 1:** Scrie `tests/smoke.rs`: `version_is_the_workspace_version` → `assert_eq!(guardian_console::VERSION, env!("CARGO_PKG_VERSION"))`.
- [ ] **Step 2:** `cargo test -p guardian-console` — Expected: FAIL (crate-ul nu există).
- [ ] **Step 3:** Creează crate-ul și modifică workspace-ul. Rulează `cargo deny check`; pentru o licență nouă, adaugă-o în `deny.toml` și scrie ruling-ul.
- [ ] **Step 4:** Rulează:
  - `cargo test -p guardian-console`;
  - `cargo deny check`;
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
  - clippy pentru `x86_64-pc-windows-gnu` și `i686-pc-windows-gnu` (comanda din etapa 2);
  - containerul Linux: `cargo build -p guardian-console`. Dacă lipsesc biblioteci de sistem pentru winit/wgpu, notează în ruling ce pachete `apt` trebuie instalate în CI în Task 12.

  Expected: PASS / curat.
- [ ] **Step 5:** Commit `console: add the guardian-console crate and raise the MSRV to 1.95 for egui 0.37`.

### Task 2: Setările consolei

**Files:**
- Create: `crates/guardian-console/src/settings.rs`
- Test: `crates/guardian-console/tests/settings.rs`

**Interfaces:**
- Produces:
  - `pub enum Mode { Remote { server: String, cert_sha256: Option<String> }, Embedded { port: u16 } }`;
  - `pub struct ConsoleSettings { pub mode: Option<Mode>, pub login: Option<String>, token: Option<Secret> }`, cu `token() -> Option<&str>`, `set_token(Option<String>)`, `load(path) -> Result<ConsoleSettings, SettingsError>` (fișier lipsă → `Default`) și `save(&self, path)` (atomic, 0600);
  - `pub struct Secret(String)`, al cărui `Debug` scrie `"***"`;
  - `pub fn settings_path() -> PathBuf`, `pub fn embedded_data_dir() -> PathBuf`, `pub fn log_path() -> PathBuf` (valorile din Global Constraints);
  - `pub fn validate_server_url(s: &str) -> Result<String, String>` (acceptă `https://host[:port]`, adaugă `:8443` dacă lipsește portul, respinge `http://` cu „use https://”, respinge spații și text gol);
  - `pub fn normalize_pin(s: &str) -> Result<Option<String>, String>` (gol → `None`; acceptă hex cu `:` sau spații, 64 de cifre hex, lowercase; altfel eroare).
- TOML-ul: `mode = "remote"|"embedded"`, plus câmpurile plate din Global Constraints. `Mode` se serializează printr-un struct intermediar, ca fișierul să rămână plat.

- [ ] **Step 1:** Scrie testele:
  - `missing_file_is_default`;
  - `remote_roundtrip` și `embedded_roundtrip` (scrie, citește, compară);
  - `token_is_hidden_in_debug` (`format!("{:?}", s)` nu conține tokenul);
  - `file_is_private` (`#[cfg(unix)]`, mod 0600);
  - `url_validation`:

    | Intrare | Rezultat |
    |---|---|
    | `"https://srv.school"` | `"https://srv.school:8443"` |
    | `"https://10.0.0.5:443"` | neschimbat |
    | `"http://x"` | eroare care conține `"https://"` |
    | `""` | eroare |

  - `pin_normalization`: `"AB:CD:…"` (32 de octeți) → hex lowercase fără `:`; 63 de cifre → eroare; `""` → `Ok(None)`.
- [ ] **Step 2:** `cargo test -p guardian-console --test settings` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Directorul de config: aceeași regulă ca `guardian-cli/src/session.rs`; mută funcția `config_dir()` în `guardian-client`, ca `pub fn user_config_dir() -> Option<PathBuf>`, și folosește-o din ambele (fără copie).
- [ ] **Step 4:** `cargo test -p guardian-console --test settings -p guardian-cli` — Expected: PASS.
- [ ] **Step 5:** Commit `console: store the console settings, with the session token kept out of logs`.

### Task 3: `Fleet` — starea live din `Watch`

**Files:**
- Create: `crates/guardian-console/src/fleet.rs`
- Test: `crates/guardian-console/tests/fleet.rs`

**Interfaces:**
- Consumes: `guardian_proto::v1::{WatchEvent, watch_event, Snapshot, Room, Computer, Profile, Mode}`.
- Produces:
  - `#[derive(Default, Clone, Debug, PartialEq)] pub struct Fleet { rooms: BTreeMap<i64, Room>, computers: BTreeMap<u64, Computer>, profiles: BTreeMap<i64, Profile> }`;
  - `pub fn apply(&mut self, ev: WatchEvent)`: `Snapshot` înlocuiește tot; `Computer` și `Room` fac upsert; `computer_removed` și `room_removed` șterg; `command_result` e ignorat în 3a;
  - `pub fn set_profiles(&mut self, ps: Vec<Profile>)`, după ce adminul editează profiluri (`Watch` nu trimite schimbările de profil);
  - `pub fn rooms(&self) -> Vec<RoomView>`, sortat după nume (fără diferență de majuscule), cu `pub struct RoomView { pub id: i64, pub name: String, pub profile: Option<Profile>, pub computers: Vec<ComputerView>, pub online: usize }`;
  - `pub fn unassigned(&self) -> Vec<ComputerView>` (calculatoare cu `room_id == 0`, vizibile doar adminului);
  - `pub struct ComputerView { pub id: u64, pub name: String, pub online: bool, pub connection: String, pub profile: Option<String>, pub exception: bool }`, cu calculatoarele sortate întâi online, apoi după nume;
  - `pub fn connection_text(online: bool, last_seen_unix: i64, now_unix: i64) -> String`:

    | Condiție | Text |
    |---|---|
    | online | `"● Online"` |
    | `last_seen_unix == 0` | `"Never connected"` |
    | sub 60 s | `"Offline · just now"` |
    | sub 1 h | `"Offline · N min ago"` |
    | sub 24 h | `"Offline · N h ago"` |
    | altfel | `"Offline · N days ago"` |

  - `pub fn profile(&self, id: i64) -> Option<&Profile>`, `pub fn profiles(&self) -> Vec<&Profile>` (după nume) și `pub fn totals(&self) -> (usize /*online*/, usize /*all*/)`.

- [ ] **Step 1:** Scrie testele:
  - `snapshot_replaces_everything` (un calculator care nu mai e în al doilea snapshot dispare);
  - `computer_update_upserts`;
  - `removals_remove`;
  - `rooms_sorted_with_online_count`: două săli, „b” și „A” → ordinea „A”, „b”; `online` numără corect;
  - `room_profile_is_resolved`;
  - `exception_is_flagged` (`exception_profile_id != 0` → `exception: true`, iar `profile` e numele profilului efectiv);
  - `unassigned_lists_room_zero`;
  - `connection_texts` (fiecare rând din tabelul de mai sus, cu `now` fix);
  - `diacritics_survive` (profilul „Lecție” apare neschimbat în `RoomView`).
- [ ] **Step 2:** `cargo test -p guardian-console --test fleet` — Expected: FAIL.
- [ ] **Step 3:** Implementează (logică pură, fără egui și fără rețea).
- [ ] **Step 4:** `cargo test -p guardian-console --test fleet` — Expected: PASS.
- [ ] **Step 5:** Commit `console: keep the live fleet from Watch events`.

### Task 4: Backend-ul — conexiune, login, `Watch`

**Files:**
- Create: `crates/guardian-console/src/backend/mod.rs`, `backend/conn.rs`, `backend/errors.rs`
- Test: `crates/guardian-console/tests/backend_conn.rs`

**Interfaces:**
- Consumes: `guardian_client::{connect, with_token}`, `guardian_proto::v1::{auth_client::AuthClient, watch_client::WatchClient, admin_client::AdminClient, control_client::ControlClient}`, `Fleet`, `ConsoleSettings`.
- Produces:
  - trait-ul UI–backend:
    ```rust
    pub trait Backend {
        fn send(&self, a: Action);
        fn poll(&self) -> Vec<Event>;   // nu blochează
    }
    ```
  - `pub struct RealBackend` cu `RealBackend::start(settings_path: PathBuf, repaint: Box<dyn Fn() + Send + Sync>) -> RealBackend`. Pornește un fir `std::thread` cu runtime tokio multi-thread, iar `repaint` se cheamă după fiecare `Event` (în UI, `ctx.request_repaint()`). Are `Drop` care oprește firul (cu `Action::Shutdown` și așteptare de cel mult 5 s).
  - `pub enum Action`:
    - în acest task: `Connect { server: String, cert_sha256: Option<String> }`, `Login { login: String, password: Secret }`, `Init { bootstrap_code: String, login: String, password: Secret }`, `Logout`, `Shutdown`;
    - cele din Task 6 se adaugă la același enum.
  - `pub enum Event`:
    - `Connection(ConnState)`, cu `pub enum ConnState { Disconnected, Connecting, NeedsLogin { can_init: bool }, Online, Reconnecting { attempt: u32, error: String } }`;
    - `LoggedIn(User)`, `LoggedOut { reason: Option<String> }`;
    - `Fleet(Fleet)`: snapshot complet al stării după fiecare eveniment `Watch` (simplu, fără diff; e mic);
    - `Profiles(Vec<Profile>)`, `Users(Vec<User>)`, `Settings(Settings)`, `Code(EnrollCode)`, `Done(String)` (mesaj scurt de confirmare), `Error(String)`.
  - `pub fn status_text(s: &tonic::Status) -> String`, cu textul pentru om:

    | `Status` | Text |
    |---|---|
    | `UNAUTHENTICATED` | `"Your session has ended. Sign in again."` |
    | `PERMISSION_DENIED` | `"You are not allowed to do that."` |
    | `INVALID_ARGUMENT` / `FAILED_PRECONDITION` / `NOT_FOUND` / `ALREADY_EXISTS` | mesajul serverului |
    | `UNAVAILABLE` | `"Cannot reach the server: …"` |
    | altfel | `"Server error: …"` |

    La login cu parolă greșită serverul dă `UNAUTHENTICATED` cu mesajul lui: pentru `Login` se arată mesajul serverului, nu textul de sesiune expirată.
- **Comportament:**
  - `Connect` salvează modul în setări. Dacă există un token salvat, încearcă `WhoAmI`: reușit → `LoggedIn` + `Watch`; `UNAUTHENTICATED` → tokenul se șterge și urmează `NeedsLogin`.
  - `can_init` e `true` doar pentru embedded cu `bootstrap_code` disponibil (vine din Task 5). Pentru remote e mereu `false`; un server la distanță nou se inițializează din CLI (`guardian-cli init`).
  - Bucla `Watch`: la `UNAVAILABLE` sau la ruperea stream-ului → `Reconnecting{attempt}`, cu backoff 1 s → 30 s, apoi reia `Watch` cu același token. La `UNAUTHENTICATED` → tokenul se șterge din setări, apoi `LoggedOut { reason: Some("Your session has ended. Sign in again.") }` și `NeedsLogin`.
  - `Logout` cheamă `Auth.Logout`, apoi șterge tokenul indiferent de rezultat.

- [ ] **Step 1:** Scrie testele, contra `guardian_server::testing::TestServer` (plaintext, URL `http://127.0.0.1:port`). `RealBackend` nu validează URL-ul: îl validează UI-ul cu `validate_server_url` înainte să trimită `Connect`, deci testele de backend pot folosi `http://`. Un helper `events_until(&backend, timeout, pred)` adună evenimentele până când `pred` e adevărat:
  - `login_then_snapshot`: admin creat cu `TestServer::admin_token` (sau `Init` pe server nou), apoi `Connect`, `NeedsLogin`, `Login`, `LoggedIn` cu `role == ADMIN`, `Fleet` cu cele trei profiluri implicite;
  - `wrong_password_is_an_error`: `Event::Error` cu mesajul serverului, starea rămâne `NeedsLogin`;
  - `saved_token_skips_login`: a doua pornire `RealBackend` pe același `console.toml` → `LoggedIn` fără `Login`;
  - `agent_coming_online_updates_the_fleet`: `SimAgent::enroll` cu un cod de sală → un `Fleet` în care calculatorul e `online` în sala lui;
  - `expired_session_asks_for_login_again` (Review Focus 2): adminul dezactivează utilizatorul consolei prin `AdminClient::update_user(active=false)` → `LoggedOut{reason: Some(..)}`, `NeedsLogin`, iar `console.toml` nu mai are token;
  - `server_restart_reconnects_and_replaces_the_snapshot` (Review Focus 3): `TestServer::restart_same_port`, iar între timp un calculator revocat direct în baza veche prin `AdminClient::revoke_computer` înainte de restart → `Reconnecting`, apoi `Online` și un `Fleet` fără acel calculator;
  - `permission_denied_is_shown` (Review Focus 5, partea backend): un `teacher` fără săli trimite `SetRoomProfile` (`Action` din Task 6) → `Event::Error("You are not allowed to do that.")`. Testul se scrie aici marcat `#[ignore]` și se activează în Task 6.
- [ ] **Step 2:** `cargo test -p guardian-console --test backend_conn` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Fiecare `Action` se execută într-un task tokio sub `catch_unwind`. Conexiunile gRPC se fac cu `guardian_client::connect(url, pin)` și `with_token`.
- [ ] **Step 4:** Rulează testul de 3 ori la rând — Expected: PASS de fiecare dată.
- [ ] **Step 5:** Commit `console: connect, sign in and follow Watch with reconnection`.

### Task 5: Serverul încorporat, instanța unică, firewall-ul

**Files:**
- Create: `crates/guardian-console/src/embedded.rs`, `src/instance.rs`, `src/firewall.rs`
- Modify: `crates/guardian-console/src/backend/mod.rs` (`Action::StartEmbedded`, `Action::StopEmbedded`, `Action::OpenFirewall`; `Event::Embedded(EmbeddedState)`)
- Test: `crates/guardian-console/tests/embedded.rs`

**Interfaces:**
- Consumes: `guardian_server::{start, ServerConfig, TlsMode, ServerHandle, StartError}`.
- Produces:
  - `pub enum EmbeddedState { Stopped, Starting, Running { port: u16, cert_sha256: String, started_at: Instant }, Failed { reason: String } }`;
  - `pub async fn start_embedded(data_dir: &Path, port: u16) -> Result<ServerHandle, String>`: `ServerConfig::new(data_dir, TlsMode::SelfSigned)` cu `listen = 0.0.0.0:<port>`; erorile devin text:

    | Eroare | Text |
    |---|---|
    | `AddrInUse` | `"Port <port> is already in use — another Guardian server or program is using it. Close it or choose another port in Settings."` |
    | `PermissionDenied` | `"Cannot use <data_dir>: access denied."` |
    | altfel | `"The server could not start: <eroare>"` |

  - comportamentul backend-ului în embedded:
    - `StartEmbedded` → `Starting`, apoi `Running` sau `Failed`;
    - la `Running`, backend-ul face singur `Connect { server: "https://127.0.0.1:<port>", cert_sha256: Some(pin) }`;
    - dacă `handle.bootstrap_code()` e `Some`, `NeedsLogin { can_init: true }` și codul rămâne în backend: UI-ul cere doar login și parolă pentru primul admin (§9.2), iar `Action::Init` folosește codul ținut în backend dacă `bootstrap_code` e gol;
    - la `Shutdown`, `handle.shutdown().await` înainte ca firul să se termine.
  - `pub fn prepare_data_dir(dir: &Path) -> Result<(), String>`:
    - creează directorul;
    - pe Windows rulează `icacls <dir> /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *<SID-ul userului curent>:(OI)(CI)F /T /C /Q`. SID-ul vine din `whoami /user /fo csv /nh`, parsat de `pub fn parse_whoami_sid(csv: &str) -> Option<String>`;
    - pe alte OS face doar `create_dir_all` + 0700.
  - `pub struct InstanceLock(File)` cu `pub fn acquire(dir: &Path) -> Result<InstanceLock, AlreadyRunning>` (cu `File::try_lock` pe `console.lock`).
  - `pub fn firewall_args(port: u16) -> Vec<String>`:
    `["advfirewall","firewall","add","rule","name=Guardian Console server","dir=in","action=allow","protocol=TCP","localport=<port>","profile=any","remoteip=localsubnet"]`, plus `firewall_delete_args()`.
  - `#[cfg(windows)] pub fn open_firewall(port: u16) -> Result<(), String>`: întâi direct cu `netsh` (șterge, apoi adaugă); dacă eșuează cu acces refuzat, prin `powershell … Start-Process netsh -Verb RunAs -Wait`. Pe alte OS, funcția nu există.
  - `Event::Embedded(Running)` vine la pornire. Dacă procesul nu poate crea regula, vine și `Event::Error` cu banner-ul.

- [ ] **Step 1:** Scrie testele (porturi libere luate din `TcpListener::bind("127.0.0.1:0")`, apoi eliberate):
  - `embedded_first_run_creates_admin_and_lists_profiles`: `StartEmbedded` → `Running` cu amprentă de 64 de cifre hex → `NeedsLogin{can_init: true}` → `Init{bootstrap_code: "", login: "admin", password}` → `LoggedIn` → `Fleet` cu „Liber”, „Lecție”, „Examen”;
  - `embedded_second_run_keeps_data_and_pin`: pornire, oprire, pornire pe același director → aceeași amprentă, `can_init: false`;
  - `port_in_use_is_reported_and_never_running` (Review Focus 1): ține portul ocupat cu un `TcpListener` → `Embedded(Failed{reason})`, cu `reason` care conține `"already in use"`; nu apare niciun `Running`;
  - `second_instance_is_refused`: `acquire` de două ori pe același director → al doilea `Err(AlreadyRunning)`; după `drop` primul se poate lua din nou;
  - `firewall_args_are_exact` (comparație cu vectorul de mai sus pentru portul 8443);
  - `whoami_sid_is_parsed`: `"\"pc\\elev\",\"S-1-5-21-1-2-3-1001\"\r\n"` → `Some("S-1-5-21-1-2-3-1001")`; text fără SID → `None`.
- [ ] **Step 2:** `cargo test -p guardian-console --test embedded` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** `cargo test -p guardian-console --test embedded` + clippy pe ambele ținte Windows — Expected: PASS / curat.
- [ ] **Step 5:** Commit `console: run the embedded server while the console is open, one instance at a time`.

### Task 6: Backend-ul — acțiunile de control și administrare

**Files:**
- Create: `crates/guardian-console/src/backend/actions.rs`
- Test: `crates/guardian-console/tests/backend_actions.rs`

**Interfaces:**
- Produces, în plus în `Action` (fiecare rulează apelul gRPC din paranteză; reușita trimite `Done("<text>")`, iar acolo unde are sens și lista nouă):
  - `SetRoomProfile { room_id, profile_id }` (`Control.SetRoomProfile`) — `Watch` aduce oricum `Room`;
  - `CreateRoom { name }`, `RenameRoom { id, name }`, `DeleteRoom { id }`;
  - `LoadProfiles` (→ `Event::Profiles` + `Fleet::set_profiles`), `SaveProfile(Profile)` (`id == 0` → `CreateProfile`, altfel `UpdateProfile`; apoi `LoadProfiles`), `DeleteProfile { id }`;
  - `UpdateComputer { id, name: Option<String>, room_id: Option<i64>, exception_profile_id: Option<i64> }`, `RevokeComputer { id }`;
  - `LoadUsers` (→ `Event::Users`), `CreateUser { login, password: Secret, role }`, `UpdateUser { id, role: Option<Role>, active: Option<bool>, new_password: Option<Secret> }`, `SetUserRooms { user_id, room_ids }`;
  - `CreateAgentCode { room_id: i64, max_uses: u32, ttl_seconds: u32 }` (→ `Event::Code`);
  - `LoadSettings` / `SaveSettings(Settings)` (→ `Event::Settings`);
  - `ChangePassword { old: Secret, new: Secret }`.
- Textele `Done`: `"Room created"`, `"Profile saved"`, `"<sală>: <profil>"` etc. Forma exactă e a implementatorului, scurtă și în engleză. Parolele nu ajung niciodată în `Done`, `Error` sau log.

- [ ] **Step 1:** Scrie testele (TestServer + admin logat prin backend):
  - `room_lifecycle` (creare, redenumire, ștergere → `Fleet` urmează);
  - `set_room_profile_reaches_the_agent`: `SimAgent` în sală, apoi `SetRoomProfile` pe „Examen” → agentul primește un `AgentState` cu `effective_profile_id` al lui „Examen”, iar `Fleet` arată profilul sălii;
  - `profile_edit_refreshes_profiles`;
  - `computer_move_and_exception`;
  - `revoke_removes_the_computer`;
  - `user_lifecycle` (creare teacher, săli, dezactivare);
  - `agent_code_is_returned` (cod nevid, `max_uses` respectat);
  - `settings_roundtrip`;
  - `change_password_then_login_with_new`;
  - activează `permission_denied_is_shown` din Task 4.
- [ ] **Step 2:** `cargo test -p guardian-console --test backend_actions --test backend_conn` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** Același test, de 3 ori — Expected: PASS.
- [ ] **Step 5:** Commit `console: add room, profile, computer, user, code and settings actions`.

### Task 7: Ce primește agentul — informațiile de conectare

**Files:**
- Create: `crates/guardian-console/src/connect_info.rs`
- Test: `crates/guardian-console/tests/connect_info.rs`

**Interfaces:**
- Produces:
  - `pub struct AgentTarget { pub server: String /* wss://HOST:port */, pub cert_sha256: Option<String> }`;
  - `pub fn agent_target(mode: &Mode, hostname: &str, port_override: Option<u16>) -> AgentTarget`:
    - remote → `https://` devine `wss://`, cu același host și port, iar amprenta din setări;
    - embedded → `wss://<HOSTNAME>:<port>` cu amprenta serverului încorporat (dată de apelant).
  - `pub fn windows_command(t: &AgentTarget, code: &str) -> String`:
    `guardian-agent.exe enroll --server <s> [--cert-sha256 <p>] --code <c>`, urmat pe rândul următor de `guardian-agent.exe service install`.
  - `pub fn linux_command(t: &AgentTarget, code: &str) -> String`:
    `sudo ./install.sh <s> <c> [<p>]` (ordinea din `packaging/agent-linux-v3/install.sh`).
  - `pub fn lan_fallback(addrs: &[IpAddr], port: u16) -> Vec<String>`: doar IPv4 private (10/8, 172.16/12, 192.168/16), ca `wss://<ip>:<port>`.
  - `pub fn no_agent_warning(state: &EmbeddedState, ever_online: bool, now: Instant) -> bool`: embedded `Running` de cel puțin 120 s și niciun calculator văzut vreodată online în această rulare.

- [ ] **Step 1:** Scrie testele:
  - `remote_target`: `https://srv.school:8443` + pin → `wss://srv.school:8443`, pin păstrat;
  - `embedded_target` (`PROF-PC`, 8443);
  - `windows_command_with_and_without_pin`;
  - `linux_command_order`;
  - `lan_fallback_filters_public_and_v6`;
  - `no_agent_warning_after_two_minutes` (119 s → false, 120 s → true, `ever_online` → false, `Failed` → false).
- [ ] **Step 2:** `cargo test -p guardian-console --test connect_info` — Expected: FAIL.
- [ ] **Step 3:** Implementează (logică pură; hostname-ul și adresele le dă apelantul, din `gethostname` și `if-addrs`).
- [ ] **Step 4:** `cargo test -p guardian-console --test connect_info` — Expected: PASS.
- [ ] **Step 5:** Commit `console: build the agent enrollment commands and the LAN fallback`.

### Task 8: UI — pornire, login, sălile profesorului

**Files:**
- Create: `crates/guardian-console/src/ui/mod.rs` (`ConsoleApp`), `ui/setup.rs`, `ui/login.rs`, `ui/rooms.rs`, `ui/status.rs`, `ui/confirm.rs`
- Create: `crates/guardian-console/src/testing.rs` (feature `testing`: `FakeBackend`)
- Test: `crates/guardian-console/tests/ui_main.rs`

**Interfaces:**
- Consumes: `Backend`, `Action`, `Event`, `Fleet`, `EmbeddedState`, `connection_text`.
- Produces:
  - `pub struct ConsoleApp<B: Backend> { … }` cu `new(backend: B, settings: ConsoleSettings) -> Self` și `pub fn ui(&mut self, ctx: &egui::Context)`, apelat din `eframe::App::update` și din teste;
  - `impl<B: Backend> eframe::App for ConsoleApp<B>`;
  - `FakeBackend` cu `push(Event)` (evenimentele pe care UI-ul le primește la `poll`) și `sent() -> Vec<Action>`.
- **Ecranele:**
  - **Setup** (fără `mode` în setări): două opțiuni mari, **Connect to a server** (URL + amprentă opțională, validate cu Task 2; erorile sub câmp) și **Run the server on this computer** (port, implicit 8443).
  - **Sign in:** login + parolă, Enter trimite. Cu `can_init`, titlul e „Create the administrator account” și are câmp de confirmare a parolei, care trebuie să fie identic, cu minimum 8 caractere, verificat înainte de trimitere.
  - **Main:**
    - **cardul de stare** de sus:
      - verde: „Connected to <server>” sau „Server running on this computer · port N”;
      - galben: „Reconnecting… (attempt N)”;
      - roșu: „Server is not running” + motivul din `Failed`;
    - **bannerul galben**, ascuns când n-are ce spune: avertismentul de 2 minute fără agenți cu butonul **Connection info…**, firewall-ul cu **Open firewall port…**, ultima eroare cu **×**;
    - **lista de săli**: câte un card cu numele, „n/m online” și un combobox cu profilul activ (toate profilurile, după nume). Schimbarea trimite `SetRoomProfile`; pe un profil `free` apare întâi dialogul de confirmare din Global Constraints;
    - sub fiecare sală, **tabelul de calculatoare**: nume, conexiune (`connection_text`), profil și „exception” dacă e cazul;
    - **bara de jos**: utilizatorul logat, rolul, **Change password…** și **Sign out**;
    - **liste goale**: un rând de indicație, de exemplu „No rooms yet. An administrator creates them in Rooms.” sau „No computers in this room yet — use Connection info… to enroll them.”.
  - **Sesiune încheiată:** un `LoggedOut{reason}` duce la ecranul Sign in, cu motivul afișat.

- [ ] **Step 1:** Scrie testele kittest, cu `egui_kittest::Harness::new_ui` sau `new`, pe `ConsoleApp<FakeBackend>`. Nodurile se caută după text sau label (`get_by_label`):
  - `setup_remote_sends_connect`: completează URL, apoi **Connect** → `sent()` conține `Connect{server: "https://srv.school:8443", ..}`;
  - `setup_rejects_http`: URL `http://x` → eroarea „https://” vizibilă, nimic trimis;
  - `first_admin_needs_matching_passwords`;
  - `rooms_show_counts_and_computers`: împinge `Online`, `LoggedIn(admin)` și un `Fleet` cu „Sala 1” (2 calculatoare, 1 online) → textele „Sala 1”, „1/2 online” și „● Online” sunt vizibile;
  - `switching_profile_sends_action`: alege „Examen” → `SetRoomProfile{room_id, profile_id}`;
  - `free_profile_needs_confirmation` (Review Focus 4): alege „Liber” → dialogul e vizibil și nimic nu e trimis; **No** → tot nimic, iar comboboxul revine; din nou, apoi **Yes** → `SetRoomProfile` cu id-ul lui „Liber”;
  - `failed_embedded_is_red_with_reason`: `Embedded(Failed{reason: "Port 8443 is already in use…"})` → textul vizibil, „Server running” absent;
  - `error_event_shows_banner_and_ui_survives`: `Error("boom")` → vizibil; un al doilea frame nu cade;
  - `teacher_sees_only_room_controls` (Review Focus 5): `LoggedIn(teacher)` → fără tab-urile „Rooms”, „Profiles”, „Computers”, „Users”, „Settings”;
  - `logged_out_shows_reason`.
- [ ] **Step 2:** `cargo test -p guardian-console --features testing --test ui_main` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Tot textul vizibil vine din `Fleet` și `Event`, fără logică de business în `ui/`. Fiecare frame începe cu `backend.poll()`.
- [ ] **Step 4:** `cargo test -p guardian-console --features testing --test ui_main` — Expected: PASS.
- [ ] **Step 5:** Commit `console: add the setup, sign-in and rooms screens`.

### Task 9: UI — panourile de administrare și „Connection info”

**Files:**
- Create: `crates/guardian-console/src/ui/admin_rooms.rs`, `ui/admin_profiles.rs`, `ui/admin_computers.rs`, `ui/admin_users.rs`, `ui/admin_settings.rs`, `ui/connect_window.rs`
- Test: `crates/guardian-console/tests/ui_admin.rs`

**Interfaces:**
- Consumes: acțiunile din Task 6, `connect_info` din Task 7.
- Produces (tab-uri vizibile doar pentru `ROLE_ADMIN`: Rooms · Profiles · Computers · Users · Settings, plus fereastra Connection info, deschisă de oricine):
  - **Rooms:** listă, **Add room**, redenumire pe loc, **Delete** cu confirmare („Computers in it move to no room.”).
  - **Profiles:**
    - selectorul de profil;
    - mod în cuvinte: „Nothing is blocked” (`free`), „Block the listed apps” (`blacklist`), „Allow only the listed apps” (`whitelist`);
    - lista de aplicații: **+** adaugă un nume, **Remove** cere o selecție;
    - **Save** trimite `SaveProfile`, **New profile** și **Delete** cu confirmare;
    - deschiderea tab-ului trimite `LoadProfiles`.
  - **Computers:** tabel cu nume, sală (combobox, inclusiv „— no room —”), excepție (combobox, „— none —” = 0), conexiune, OS și versiunea agentului; **Rename…** și **Revoke…** cu confirmare („It must be enrolled again with a new code.”).
  - **Users:** listă, **Add user** (login, parolă, rol), rol, săli (checkbox-uri), **Disable/Enable** cu confirmare la dezactivare, **Reset password…**; deschiderea tab-ului trimite `LoadUsers`.
  - **Settings:** profilul implicit al școlii (combobox) și retenția în zile (1–3650), cu **Save**. Dedesubt, doar citire: modul consolei, iar pentru embedded portul, amprenta (cu **Copy**) și directorul de date. **Change mode…** șterge `mode` din setări și duce la ecranul Setup (cu confirmare; oprește serverul încorporat).
  - **Connection info…:**
    - alegerea sălii;
    - **New code** trimite `CreateAgentCode{room_id, max_uses: 50, ttl_seconds: 86400}` pentru sală sau `{0, 1, 0}` pentru un singur calculator;
    - afișează comanda Windows și comanda Linux din Task 7, fiecare cu **Copy**, plus amprenta;
    - în embedded, adresele `lan_fallback` cu nota „Use one of these if the agents cannot find <HOSTNAME>.”;
    - copierea prin `ctx.copy_text`.

- [ ] **Step 1:** Scrie testele kittest:
  - `rooms_add_sends_create`;
  - `delete_room_needs_confirmation`;
  - `profile_mode_and_apps_are_saved`: alege „Examen”, adaugă `notepad.exe`, **Save** → `SaveProfile` cu `apps` care conține `notepad.exe` și modul `whitelist`;
  - `computer_move_sends_update`;
  - `revoke_needs_confirmation`;
  - `add_user_sends_create_without_echoing_password` (parola nu apare în niciun text vizibil după trimitere);
  - `settings_save`;
  - `connection_info_shows_commands_after_new_code`: `Event::Code` cu codul `ABCD-1234` → ambele comenzi conțin `ABCD-1234` și `wss://`;
  - `change_mode_returns_to_setup`.
- [ ] **Step 2:** `cargo test -p guardian-console --features testing --test ui_admin` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** `cargo test -p guardian-console --features testing --test ui_admin --test ui_main` — Expected: PASS.
- [ ] **Step 5:** Commit `console: add the administration tabs and the connection info window`.

### Task 10: Binarul — fereastra, render-ul, log-ul

**Files:**
- Modify: `crates/guardian-console/src/main.rs`
- Create: `crates/guardian-console/src/logging.rs`
- Test: `crates/guardian-console/tests/logging.rs`, `tests/binary.rs`

**Interfaces:**
- Produces:
  - **`main()`:**
    1. ia `InstanceLock`;
    2. pornește log-ul;
    3. încarcă setările;
    4. pornește `RealBackend`; dacă modul e embedded, trimite `StartEmbedded`, altfel `Connect`;
    5. deschide fereastra cu `eframe::run_native("Guardian Console", NativeOptions { renderer: Wgpu, viewport: 1100×720, minim 800×520, iconița }, …)`;
    6. la eroare de render, reîncearcă cu `Renderer::Glow`; `GUARDIAN_RENDERER=gl` forțează glow;
    7. dacă nu merge niciunul, dialogul `rfd` „Guardian Console cannot open its window: <eroare>”, cu calea log-ului, și cod 1;
    8. la închidere, `drop(backend)` oprește serverul încorporat curat înainte de ieșire.
  - **Argumente:**
    - `--version` tipărește `guardian-console <VERSION>` și iese cu 0;
    - `--settings <PATH>` e doar pentru dezvoltare și teste;
    - altceva → mesajul de utilizare și codul 2.
  - **`pub fn rotate(path: &Path, max_bytes: u64) -> std::io::Result<()>`** (funcție pură) și **`pub fn init_logging(path: &Path) -> bool`** (`true` dacă se scrie în fișier):
    - rotește `console.log` în `.1` și `.1` în `.2` la pornire, dacă depășește 1 MiB;
    - scrie cu `tracing-subscriber` (`info` implicit, `RUST_LOG` îl suprascrie), fără ANSI;
    - dacă nu poate scrie, merge mai departe fără fișier.
  - **Panicile:** `std::panic::set_hook` le scrie în log, cu mesajul și locația.

- [ ] **Step 1:** Scrie testele:
  - `logging.rs`: `rotates_over_one_mib` (un fișier de 1 MiB + 1 → după `init` există `.1`, iar `console.log` e nou), `keeps_at_most_two_old_files`, `unwritable_dir_does_not_fail`;
  - `binary.rs`: `version_flag` (rulează `CARGO_BIN_EXE_guardian-console --version` → cod 0, ieșire `guardian-console <VERSION>`) și `unknown_flag_is_usage_error` (cod 2).

  Rotația e o funcție pură `rotate(path, max)`, ca testul să nu depindă de subscriberul global.
- [ ] **Step 2:** `cargo test -p guardian-console --test logging --test binary` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Argumentele se citesc înainte de orice fereastră, ca `--version` să meargă fără ecran.
- [ ] **Step 4:** Rulează:
  - testele de mai sus;
  - clippy pe macOS, pe Linux (container) și pe ambele ținte Windows-gnu;
  - local, `cargo run -p guardian-console -- --settings /tmp/…`: fereastra se deschide, modul embedded pornește serverul și se poate crea adminul. Este o verificare de mână pe macOS; dacă nu se poate, se scrie în ruling.

  Expected: PASS / curat.
- [ ] **Step 5:** Commit `console: open the window with a wgpu or glow renderer and log to a rotated file`.

### Task 11: Prima utilizare reală — consolă încorporată + agent + profil schimbat

**Files:**
- Create: `crates/guardian-console/tests/real_embedded.rs`

**Interfaces:**
- Consumes: `RealBackend` (fără UI), `guardian_server::testing::SimAgent`, iar pe Linux binarul real `guardian-agent` (ca în etapa 2, din `GUARDIAN_BIN_DIR` sau `target/debug`).
- **Testul `console_embedded_server_drives_a_real_agent`:**
  1. `RealBackend` pe un director temporar, embedded pe un port liber;
  2. `Init` admin, `CreateRoom("Sala 1")`, `SaveProfile` pentru „Lecție” cu `guardian-test-game`, `CreateAgentCode`;
  3. construiește comanda de enroll Linux cu `connect_info::agent_target` și o rulează cu binarul `guardian-agent` (`enroll`, apoi `run`, cu `--config` / `--data-dir` temporare);
  4. așteaptă un `Fleet` cu calculatorul online în „Sala 1”;
  5. o copie a `/bin/sleep` cu numele `guardian-test-game` e omorâtă;
  6. `SetRoomProfile` „Liber”, fără confirmare (e backend) → o copie nouă trăiește 3 s;
  7. `Shutdown` → agentul rămâne viu, iar `Fleet` nu mai vine.

  Pe macOS (fără control de procese), pașii 5–6 se înlocuiesc cu verificarea profilului primit de agent, prin `guardian-agent status`. Se folosește `#[cfg(target_os = "linux")]` pentru pașii cu procese.

- [ ] **Step 1:** Scrie testul.
- [ ] **Step 2:** Rulează-l în container: `cargo build --workspace --bins && cargo test -p guardian-console --test real_embedded`. Expected: FAIL înainte ca toate piesele să fie legate, PASS după.
- [ ] **Step 3:** Repară ce iese la iveală în codul taskului vinovat, nu în test.
- [ ] **Step 4:** Rulează testul de 3 ori în container, apoi o dată pe macOS — Expected: PASS.
- [ ] **Step 5:** Commit `console: cover the embedded server driving a real agent`.

### Task 12: CI pentru consolă

**Files:**
- Modify: `.github/workflows/rust.yml`

**Interfaces:**
- Produces:
  - **`linux`:** instalează pachetele `apt` notate în ruling-ul din Task 1, dacă sunt necesare. `cargo test --workspace --all-features` acoperă deja `guardian-console` (kittest e headless). Adaugă `cargo test -p guardian-console --test real_embedded` după `cargo build --workspace --bins`.
  - **`windows`:**
    - `cargo test -p guardian-console --features testing`;
    - `cargo build --release -p guardian-console --target i686-pc-windows-msvc`;
    - artefactul `console-windows` cu `target/i686-pc-windows-msvc/release/guardian-console.exe`.
  - `paths` rămân aceleași (`crates/**` acoperă crate-ul nou).
- [ ] **Step 1:** Scrie modificările.
- [ ] **Step 2:** Verifică sintaxa cu `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/rust.yml'))"`. Rularea reală are loc la primul push (decizia proprietarului) și se spune asta în raport.
- [ ] **Step 3:** Commit `build: test the console and build its 32-bit Windows binary`.

### Task 13: Documentația

**Files:**
- Create: `specs/console.md`
- Modify:
  - `AGENTS.md` (crate-ul nou în tabel, MSRV 1.95, comenzile de test cu `--features testing`, etapa 3a gata);
  - `specs/2026-11-17-rust-rewrite-design.md` (§14 punctul 3: notă datată că 3a e făcută, cu trimitere la `console.md` și la împărțirea în 3a/3b/3c);
  - `~/Projects/PROJECTS.md` (secțiunea guardian, data).

- [ ] **Step 1:** Scrie `specs/console.md`:
  - straturile și modulele;
  - fișierele și drepturile;
  - modurile remote/embedded și ce se întâmplă la pornire și la închidere;
  - textele de eroare pentru server;
  - ecranele (cu o schiță text a ferestrei principale);
  - rolurile;
  - confirmările;
  - ce dă Connection info;
  - limitarea „editările de profil făcute de alt admin apar la redeschiderea tab-ului Profiles” (`Watch` nu le trimite);
  - lipsa UI-ului pentru comenzi până la etapa 4;
  - **lista de verificare manuală pe Windows**:
    - pornire ca user fără drepturi de administrator, cu modul remote funcțional;
    - modul embedded: banner-ul de firewall, **Open firewall port…** cu prompt UAC, regula creată cu `remoteip=localsubnet`;
    - un agent de pe alt PC se înrolează cu comanda din Connection info;
    - închiderea ferestrei oprește serverul, iar agentul rămâne pe ultimul profil;
    - a doua pornire arată „already running”;
    - textul cu diacritice („Lecție”) se vede corect;
    - render-ul pe un PC fără driver GPU (WARP sau glow).
- [ ] **Step 2:** Rulează verificarea finală:
  - `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && cargo deny check`;
  - containerul Linux (inclusiv `real_embedded`);
  - clippy pe ambele ținte Windows-gnu.

  Expected: totul verde.
- [ ] **Step 3:** Commit `console: document the 3.0 console and its manual Windows checks`.
