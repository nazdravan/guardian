# Guardian 3.0 — Etapa 3c: `guardian-setup`, arhiva 3.0, ștergerea codului Go — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Guardian 3.0 se poate livra. Asta înseamnă:
- **`guardian-setup.exe`** instalează, diagnostichează, actualizează și dezinstalează agentul Rust pe Windows, din fereastră sau din linia de comandă;
- **arhiva `guardian-3.0.0.zip`** se construiește în CI cu serverul, consola, setup-ul și agenții;
- **codul Go dispare** din repo;
- **documentația** descrie 3.0.

**Architecture:**
- **Un crate nou, `guardian-setup`, construit după `rust:specs/2026-10-09-console-architecture-design.md`:**
  - un strat pur `model` (căi, arhitectură, whitelist, planuri de instalare), fără egui și fără apeluri de sistem;
  - trait-ul `Host`, care ascunde tot ce atinge Windows; `FakeHost` (feature `testing`) îl înlocuiește în teste;
  - un tabel de operații `ops`, folosit și de CLI, și de fereastră;
  - interfața egui, cu teste `egui_kittest`;
  - un binar care alege între fereastră și CLI.
- **Agentul rămâne cel care se înregistrează ca serviciu** (`guardian-agent service install`). Setup-ul doar copiază binarele, rulează `enroll`, generează `whitelist.txt`, deschide portul 11500 și pornește serviciul.
- **Logging:** rotația log-ului și hook-ul de panică ies din `guardian-console` într-un crate mic, comun.

**Tech Stack:** cele din 3a/3b:
- egui/eframe 0.37, `egui_kittest`, `clap`, `serde_json`, `windows-service` 0.8;
- `guardian-agent` ca librărie (`AgentPaths`, `AgentConfig`, `store`);
- `guardian-platform` (procese; FFI nou doar în `windows/ffi_*.rs`), `guardian-core` (`AppName`, `protected`).

**Spec:**
- [`specs/2026-11-17-rust-rewrite-design.md`](../2026-11-17-rust-rewrite-design.md): §8.1 (`guardian-setup`), §8.3 (`whitelist.txt` generat de setup la instalare), §10.3 (reînrolarea din `guardian-setup`), §11.3 (installer-ul deschide portul doar pentru rețeaua locală), §14 punctul 3 (forma arhivei), §17 (documentația).
- Arhitectura setup-ului: `git show rust:specs/2026-10-09-console-architecture-design.md`, adaptată la agentul Rust: fără `contract.json` (contractul e chiar `guardian-agent`/`guardian-core`), fără `.env`.
- Cel de înlocuit: `tools/whitelist-gui` ([`specs/whitelist-gui.md`](../whitelist-gui.md)).
- Starea de acum: [`agent-v3.md`](../agent-v3.md), [`console-v3.md`](../console-v3.md), [`server-v3.md`](../server-v3.md), [`local-fallback-v3.md`](../local-fallback-v3.md).

## Înainte de a începe

- **Branch:** `rust-rewrite`, peste 3b (`e73b5e4`).
  - Fără `feat`; commit-urile sunt `<crate sau zonă>: <propoziție>`, fără trailer-e.
  - Nimic nu se împinge și nu se face merge în `main`: release-ul pornește doar la push pe `main`, iar decizia aceea e a proprietarului.
- **Teste:** ce nu e `#[cfg(windows)]` rulează pe macOS și în containerul Linux. Codul Windows se verifică cu clippy pe `x86_64-pc-windows-gnu` și `i686-pc-windows-gnu`; testele lui și build-urile MSVC rulează în jobul `windows` din CI.
- **Limba:** UI-ul și mesajele sunt în engleză.

## Deciziile planului (ale mele)

1. **Unde stă `guardian-setup.exe`:** în **`agent-windows/`**, lângă `bin/x64/` și `bin/x86/`. E installer-ul care rulează pe calculatoarele elevilor; §14 îl pune în `console-windows/`, dar tot §14 lasă forma exactă builder-ului. `console-windows/` are doar `guardian-console.exe`.
2. **Fără `.bat` și fără `.ps1`:**
   - dublu clic pe `guardian-setup.exe` deschide fereastra, cu câmpurile Server, Certificate SHA-256 (opțional) și Enrollment code;
   - pentru instalări din script: `guardian-setup.exe install --server … --code … [--cert-sha256 …]`, pe care **Connection info…** din consolă îl dă gata scris (Task 7).
3. **Căile pe Windows:**
   - binarele în `%ProgramW6432%\Guardian\` (`C:\Program Files\Guardian`), chiar dacă setup-ul e un proces de 32 de biți: `%ProgramFiles%` ar da `Program Files (x86)`;
   - pe Windows de 32 de biți `ProgramW6432` lipsește și se folosește `%ProgramFiles%`;
   - datele rămân unde le pune agentul (`AgentPaths::system_default`: `%ProgramData%\Guardian Agent\`).
4. **Arhitectura agentului:** după ce raportează sistemul, nu după cum e compilat setup-ul: `PROCESSOR_ARCHITEW6432`, altfel `PROCESSOR_ARCHITECTURE`. `AMD64` → `bin\x64`; `x86` → `bin\x86`; `ARM64` → `bin\x64` (rulează emulat) cu avertisment în log.
5. **Guardian 2.x pe același PC** (serviciul `ProcSentinelAgent`, fișierele în `%SystemRoot%\System32\ProcSentinel\agent`):
   - `install` refuză cu `legacy_agent` dacă îl găsește, ca să nu ruleze doi agenți care omoară procese;
   - `install --remove-legacy` (în fereastră: o confirmare) oprește și șterge serviciul vechi, apoi directorul lui;
   - pentru un proces de 32 de biți, calea trece prin `Sysnative`, doar în `Host::io_path`.
6. **`whitelist.txt` generat la instalare (§8.3):**
   - conține numele proceselor care rulează în acel moment, **în afara** celor al căror executabil e sub `C:\Users\` (aplicațiile pornite de un utilizator, de exemplu un browser deschis) și a celor deja protejate de `guardian-core`;
   - se scrie doar dacă fișierul nu există, deci reinstalarea nu-l suprascrie;
   - cost dacă e greșit: un utilitar care rulează din profilul utilizatorului trebuie adăugat de mână.
7. **Reînrolarea (§10.3):**
   - `reenroll --code … [--server …]` oprește serviciul, șterge `agent.key`, `computer_id` din config, `state.pb`, `seen.txt` și `local.pb`, apoi rulează `enroll` și pornește serviciul;
   - `install` peste o instalare deja înrolată păstrează înrolarea și doar actualizează binarele.
8. **Ce face fiecare operație cu fișierele:**
   - `uninstall --yes` oprește și șterge serviciul, regula de firewall și directorul cu binare, dar păstrează datele (cheia și starea);
   - `uninstall --yes --purge` șterge și `%ProgramData%\Guardian Agent\`;
   - `update` oprește serviciul, înlocuiește binarele (copia veche merge în `*.old`), pornește serviciul; dacă nu pornește în 30 s, pune înapoi copia veche și o pornește.
9. **Drepturi de administrator:**
   - manifest `asInvoker`;
   - operațiile care schimbă ceva cer administrator. CLI-ul neridicat iese cu codul **5** și mesajul `Run this from an elevated prompt (Run as administrator).`;
   - fereastra neridicată se repornește o singură dată ridicată, prin `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe -NoProfile -Command Start-Process -Verb RunAs -FilePath <exe> -ArgumentList --elevated-relaunch`; dacă UAC e refuzat, iese fără mesaj;
   - verificarea de token și `AttachConsole` sunt singurul cod `unsafe` nou, în `guardian-platform/src/windows/ffi_token.rs` și `ffi_console.rs`.
10. **O singură operație care schimbă ceva la un moment dat:** prin `File::try_lock` pe `%ProgramData%\Guardian\setup.lock`, ca în consolă (`instance.rs`). Al doilea primește `Another Guardian Setup operation is in progress.`
11. **Logging-ul:** `crates/guardian-console/src/logging.rs` se mută în crate-ul nou `crates/guardian-applog`, folosit de consolă și de setup. Altfel ar exista două copii ale aceluiași cod.
12. **Arhiva 3.0:**
    - serverul de Windows e `guardian-server.exe` pe **64 de biți** (MSVC), cu `service install`, plus `guardian-cli.exe`;
    - consola și setup-ul sunt pe **32 de biți**;
    - agentul și worker-ul există pe 32 și pe 64 de biți;
    - pe Linux: server, CLI, agent și worker pe `x86_64-unknown-linux-gnu`;
    - **fără APK** (§14);
    - `mobile/` rămâne în repo, neatins, până la rescrierea Flutter.
13. **Spec-urile Go-ului** (`specs/server.md`, `agent.md`, `api.md`, `whitelist-gui.md`, `2026-11-13-windows-server-gui-design.md`):
    - `server.md`, `agent.md` și `api.md` se rescriu (§17): `*-v3.md` devin `server.md`, `agent.md`, `console.md`, `local-fallback.md`, iar `api.md` devine referința gRPC;
    - `whitelist-gui.md` și design-ul `server-gui` se șterg, pentru că descriu cod care dispare (rămân în istoria git);
    - planurile vechi din `specs/plans/` rămân.

## Global Constraints

- **MSRV 1.95**, edition 2024, `cargo deny` (fără GPL/MPL).
- **`unsafe`:** `#![forbid(unsafe_code)]` în `guardian-setup` și `guardian-applog`; `unsafe` doar în `guardian-platform/src/windows/ffi_*.rs`, cu un comentariu `// SAFETY:` deasupra fiecărui bloc.
- **Fără `unwrap`/`expect`/`panic` în producție.** Fiecare operație rulează sub `catch_unwind`: o panică devine `OpError::Internal` și ajunge în log.
- **Ținta setup-ului:** `i686-pc-windows-msvc`, CRT static (`.cargo/config.toml` are deja regula), `windows_subsystem = "windows"` în release, iconiță și manifest `asInvoker` + `PerMonitorV2` + Windows 10 prin `winresource`.
- **Uneltele de sistem se pornesc doar cu cale absolută** sub `%SystemRoot%\System32` (regula din 3a: `netsh`, `powershell`, `icacls`).
- **Codurile de ieșire ale CLI-ului:**

  | Cod | Când |
  |---|---|
  | `0` | succes |
  | `1` | operația a eșuat |
  | `2` | utilizare greșită |
  | `5` | lipsesc drepturile de administrator |
  | `10` | `status --check` a găsit o problemă |

  `--json` dă o schemă stabilă (`serde`). Codul de înrolare nu apare niciodată în log, în JSON sau în `Debug` (`Secret`).
- **Regula de firewall:** numele `Guardian local fallback`, TCP **și** UDP, portul `local_port` din `agent.toml` (implicit 11500), `remoteip=localsubnet`, profil `any` (§11.3).
- **Numele fixe:** serviciul `GuardianAgent` (din `guardian-agent`), cel vechi `ProcSentinelAgent`, binarele `guardian-agent.exe` și `guardian-worker.exe`.
- **`VERSION`** devine `3.0.0`; din el se scrie și `version` din workspace (`3.0.0`, fără `-dev`).

## Review Focus

1. **Setup-ul de 32 de biți pe Windows de 64.** Agentul de 64 de biți ajunge în `C:\Program Files\Guardian`, nu în `Program Files (x86)`, iar calea veche din `System32` e găsită prin `Sysnative`. Test în Task 1: `layout_on_64_bit_windows_from_a_32_bit_process`.
2. **Un PC cu Guardian 2.x instalat.** `install` nu lasă doi agenți activi: refuză fără `--remove-legacy` și îl elimină complet cu el. Test în Task 2: `legacy_agent_blocks_install_until_removed`.
3. **Codul de înrolare e greșit sau serverul nu răspunde la instalare.** Eroarea e clară, nu rămâne niciun serviciu înregistrat, iar o a doua încercare cu codul bun merge. Test în Task 2: `failed_enroll_leaves_no_service_and_retry_works`.
4. **`update` când noul agent nu pornește.** Binarele vechi revin și serviciul rulează din nou. Test în Task 2: `update_rolls_back_when_the_new_agent_does_not_start`.
5. **Setup-ul pornit fără drepturi de administrator, din CLI sau din fereastră.** CLI: codul 5 și un mesaj clar, fără nicio modificare. Fereastra: o singură repornire ridicată, nu o buclă. Teste: Task 3 `mutating_op_without_admin_exits_5`, Task 6 `relaunch_happens_once`.

---

### Task 1: `guardian-setup` — modelul pur (căi, arhitectură, whitelist) și `Host`

**Files:**
- Create: `crates/guardian-setup/{Cargo.toml,build.rs,src/lib.rs,src/model.rs,src/host.rs,src/fake.rs}`
- Create: `crates/guardian-setup/assets/guardian-setup.manifest` (copie a manifestului consolei, alt `name`)
- Test: `crates/guardian-setup/tests/model.rs`

**Interfaces:**
- Produces, `model.rs`:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] pub enum Arch { X64, X86 }`;
  - `pub fn os_arch(env: &dyn Fn(&str) -> Option<String>) -> Arch` (decizia 4);
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize)] pub struct Layout { pub program_dir: PathBuf, pub agent_exe: PathBuf, pub worker_exe: PathBuf, pub config: PathBuf, pub data_dir: PathBuf, pub legacy_dir: PathBuf, pub lock: PathBuf }`;
  - `pub fn layout(env: &dyn Fn(&str) -> Option<String>) -> Layout`:
    - folosește `ProgramW6432`, `ProgramFiles`, `ProgramData` și `SystemRoot`;
    - valorile implicite sunt `C:\Program Files`, `C:\ProgramData`, `C:\Windows`;
    - `config` și `data_dir` sunt cele din `AgentPaths::system_default()` construite din același `ProgramData`;
    - `legacy_dir` = `<SystemRoot>\System32\ProcSentinel\agent`, în forma canonică;
  - `pub fn bundled_bins(setup_dir: &Path, arch: Arch) -> (PathBuf, PathBuf)`: `bin\x64\…` sau `bin\x86\…` lângă setup;
  - `pub struct Whitelist { names: BTreeSet<AppName>, other_lines: Vec<String> }`, cu metodele:
    - `parse(text: &str) -> (Whitelist, Vec<String> /*warnings*/)`: acceptă BOM, CRLF și `#`; liniile invalide devin avertismente;
    - `add(&mut self, name: &str) -> Result<bool, AppNameError>`;
    - `remove(&mut self, name: &str) -> bool`;
    - `names(&self) -> Vec<String>`;
    - `render(&self) -> String` păstrează comentariile de la început și scrie câte un nume pe linie, sortat;
  - `pub fn generated_whitelist(procs: &[ProcessInfo], roots: &TrustedRoots) -> Whitelist` (decizia 6): sare peste executabilele sub `C:\Users\` (fără diferență de majuscule) și peste numele pe care `guardian_core::protected` le protejează deja;
  - `pub struct Secret(String)`, cu `Debug` = `"***"`, `Serialize` = `"***"` și `expose()`.
- Produces, `host.rs`:
  ```rust
  pub trait Host: Send + Sync {
      fn env(&self, var: &str) -> Option<String>;
      fn is_elevated(&self) -> bool;
      fn setup_dir(&self) -> PathBuf;
      fn io_path(&self, canonical: &Path) -> PathBuf;            // Sysnative only here
      fn service(&self, name: &str) -> Result<Option<ServiceState>, HostError>;   // None = not registered
      fn start_service(&self, name: &str, timeout: Duration) -> Result<(), HostError>;
      fn stop_service(&self, name: &str, timeout: Duration) -> Result<(), HostError>;
      fn delete_service(&self, name: &str) -> Result<(), HostError>;
      fn run(&self, exe: &Path, args: &[String]) -> Result<RunOutput, HostError>;  // code, stdout, stderr
      fn processes(&self) -> Result<Vec<ProcessInfo>, HostError>;
      fn trusted_roots(&self) -> TrustedRoots;
      fn firewall_rule(&self, name: &str) -> Result<bool, HostError>;
      fn set_firewall_rule(&self, name: &str, port: u16) -> Result<(), HostError>;
      fn delete_firewall_rule(&self, name: &str) -> Result<(), HostError>;
  }
  ```
  Fișierele le scrie și le citește `std::fs` pe căile `io_path`. `ServiceState`: `Running`, `Stopped`, `Starting`, `Stopping`, `Other`.
- Produces, `fake.rs` (feature `testing`): `FakeHost`, cu:
  - un director temporar ca rădăcină, prin env-ul fals (`ProgramW6432` etc. arată în el);
  - servicii și reguli de firewall ținute în memorie;
  - `run` simulat pentru `guardian-agent.exe enroll|service install|service uninstall|status`: scrie `agent.toml` cu `computer_id`, înregistrează serviciul și îl pornește;
  - eșecuri programabile pe fiecare metodă: `fail_on(Method, HostError)`, `fail_enroll(code, message)`, `agent_wont_start()`;
  - un jurnal al apelurilor (`calls()`).

- [ ] **Step 1:** Scrie testele:
  - `x86_and_x64_from_env`;
  - `arm64_uses_x64`;
  - `layout_on_64_bit_windows_from_a_32_bit_process` (env cu `ProgramFiles=C:\Program Files (x86)` și `ProgramW6432=C:\Program Files` → `program_dir == C:\Program Files\Guardian`);
  - `layout_on_32_bit_windows`;
  - `layout_has_no_sysnative`;
  - `whitelist_parses_bom_crlf_and_comments`;
  - `whitelist_add_rejects_paths`;
  - `whitelist_render_round_trips`;
  - `generated_whitelist_skips_user_apps_and_protected`;
  - `secret_never_prints`.
- [ ] **Step 2:** `cargo test -p guardian-setup --features testing --test model` — Expected: FAIL.
- [ ] **Step 3:** Implementează. `build.rs` (winresource, cu verificarea țintei prin `CARGO_CFG_TARGET_OS`, nu prin `cfg!`: minorul din 3a nu se repetă). Crate-ul se adaugă în workspace.
- [ ] **Step 4:** `cargo test -p guardian-setup --features testing --test model && cargo deny check` — Expected: PASS.
- [ ] **Step 5:** Commit `setup: add the install model, the Host trait and its fake`.

### Task 2: Operațiile — install, update, uninstall, reenroll, legacy

**Files:**
- Create: `crates/guardian-setup/src/lifecycle.rs`
- Test: `crates/guardian-setup/tests/lifecycle.rs`

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct InstallArgs { pub server: String, pub code: Secret, pub cert_sha256: Option<String>, pub local_port: Option<u16>, pub remove_legacy: bool, pub no_start: bool }`;
  - `pub fn install(h: &dyn Host, a: &InstallArgs, progress: &dyn Fn(&str)) -> Result<Installed, OpError>`, cu pașii în ordine:
    1. administrator;
    2. `ProcSentinelAgent`: `legacy_agent` fără `remove_legacy`, altfel oprește, șterge serviciul și directorul;
    3. copiază binarele potrivite arhitecturii în `program_dir`, cu serviciul nou oprit dacă rulează;
    4. `enroll`, doar dacă `agent.toml` nu are `computer_id`;
    5. scrie `local_port` în `agent.toml` dacă e dat;
    6. `whitelist.txt` generat, dacă lipsește;
    7. regula de firewall;
    8. `guardian-agent.exe service install`, sau doar pornirea, dacă serviciul există;
  - `pub struct Installed { pub computer_id: u64, pub enrolled_now: bool, pub program_dir: PathBuf }`;
  - `pub fn update(h, progress) -> Result<(), OpError>` (decizia 8, cu rollback);
  - `pub fn uninstall(h, purge: bool, progress) -> Result<(), OpError>`;
  - `pub fn reenroll(h, server: Option<String>, code: Secret, cert: Option<String>, progress) -> Result<u64, OpError>`;
  - `pub fn status(h) -> Status`:
    - `Status { installed: bool, service: Option<ServiceState>, computer_id: Option<u64>, server: Option<String>, local_port: u16, firewall_rule: bool, legacy_agent: bool, agent_version: Option<String>, problems: Vec<String> }`;
    - `problems` cuprinde serviciul oprit, lipsa regulii de firewall, prezența agentului vechi, un agent neînrolat;
  - `#[derive(Debug, thiserror::Error)] pub enum OpError { NeedsAdmin, LegacyAgent, Enroll(String), Busy, NotInstalled, MissingBinaries(PathBuf), Host(HostError), Internal(String) }`, cu mesajele:

    | Variantă | Mesaj |
    |---|---|
    | `NeedsAdmin` | `Run this from an elevated prompt (Run as administrator).` |
    | `LegacyAgent` | `Guardian 2.x (ProcSentinelAgent) is installed. Remove it first (--remove-legacy).` |
    | `Enroll(m)` | `Enrollment failed: <m>` (textul agentului) |
    | `Busy` | `Another Guardian Setup operation is in progress.` |
    | `NotInstalled` | `The Guardian agent is not installed on this computer.` |
    | `MissingBinaries(p)` | `<p> is missing; run guardian-setup.exe from the unpacked archive.` |

- [ ] **Step 1:** Scrie testele, pe `FakeHost`:
  - `fresh_install_runs_every_step_in_order` (verifică `calls()`);
  - `install_twice_keeps_enrollment`;
  - `legacy_agent_blocks_install_until_removed`;
  - `failed_enroll_leaves_no_service_and_retry_works`;
  - `missing_bundled_binaries_is_a_clear_error`;
  - `existing_whitelist_is_not_overwritten`;
  - `local_port_goes_into_agent_toml_and_the_firewall_rule`;
  - `update_replaces_binaries_and_restarts`;
  - `update_rolls_back_when_the_new_agent_does_not_start`;
  - `uninstall_keeps_data_purge_removes_it`;
  - `uninstall_removes_firewall_rule`;
  - `reenroll_clears_identity_and_enrolls`;
  - `status_lists_problems`;
  - `every_mutating_op_needs_admin`;
  - `code_never_reaches_progress_or_errors`.
- [ ] **Step 2:** `cargo test -p guardian-setup --features testing --test lifecycle` — Expected: FAIL.
- [ ] **Step 3:** Implementează. Formatul `agent.toml` îl scriu `guardian_agent::config::AgentConfig::{load,save}`, nu cod propriu.
- [ ] **Step 4:** Același test — Expected: PASS.
- [ ] **Step 5:** Commit `setup: install, update, uninstall and re-enroll the agent`.

### Task 3: Tabelul de operații și CLI-ul

**Files:**
- Create: `crates/guardian-setup/src/{ops.rs,cli.rs}`
- Test: `crates/guardian-setup/tests/ops.rs`

**Interfaces:**
- Consumes: Task 2.
- Produces:
  - `pub struct OpSpec { pub name: &'static str, pub help: &'static str, pub mutating: bool, pub ui: bool }`;
  - `pub const OPS: &[OpSpec]` cu `status`, `install`, `update`, `uninstall`, `reenroll`, `whitelist list`, `whitelist add`, `whitelist remove`, `processes`, `service start`, `service stop`, `service restart`, `logs`, `version`;
  - `pub fn run_op(h: &dyn Host, op: Op, progress: &dyn Fn(&str)) -> Result<Output, OpError>`:
    - ia lock-ul `setup.lock` pentru operațiile `mutating`;
    - rulează sub `catch_unwind`;
    - `Op` e enum-ul cu argumentele fiecărei operații;
  - `Output` (`Serialize`):
    - text pentru oameni;
    - `--json` = `{"ok":bool,"op":…,"result":…,"error":…}`;
  - `cli.rs`:
    - `pub fn main_cli(h: &dyn Host, args: Vec<OsString>, out: &mut dyn Write) -> i32`, cu `clap` derive;
    - `install` are `--server`, `--code` sau `--code-file F` (`-` = stdin, ca să nu ajungă în lista proceselor), `--cert-sha256`, `--local-port`, `--remove-legacy`, `--no-start`;
    - `uninstall` cere `--yes` (fără el: codul 2);
    - `logs [--lines N]` arată finalul lui `agent.log`;
    - `status --check` iese cu 10 dacă există probleme;
  - `pub fn ui_ops() -> &'static [&'static str]`: lista explicită a operațiilor legate în fereastră, verificată de testul de completitudine.

- [ ] **Step 1:** Scrie testele:
  - `every_op_has_cli_help_and_ui_binding_or_is_cli_only` (un test de completitudine peste `OPS`; `logs` și `version` pot fi doar CLI, marcate cu `ui: false`);
  - `mutating_op_without_admin_exits_5`;
  - `uninstall_needs_yes`;
  - `json_output_is_stable` (snapshot pe `status --json`);
  - `code_file_dash_reads_stdin`;
  - `second_mutating_op_is_busy`;
  - `panic_in_op_is_internal_error` (o operație de test care face panic);
  - `status_check_exit_10`;
  - `whitelist_add_remove_round_trip`.
- [ ] **Step 2:** `cargo test -p guardian-setup --features testing --test ops` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** Același test — Expected: PASS.
- [ ] **Step 5:** Commit `setup: one table of operations for the window and the command line`.

### Task 4: `WinHost` și FFI-ul din `guardian-platform`

**Files:**
- Create: `crates/guardian-setup/src/winhost.rs` (`#[cfg(windows)]`)
- Create: `crates/guardian-platform/src/windows/{ffi_token.rs,ffi_console.rs}`
- Modify: `crates/guardian-platform/src/windows/mod.rs` și `lib.rs`: exportă `guardian_platform::windows_util::{is_elevated, attach_parent_console}`
- Modify: `Cargo.toml` (feature-uri `windows` dacă e nevoie: `Win32_System_Console`)
- Test: `crates/guardian-setup/tests/winhost.rs` (`#![cfg(windows)]`, rulează în CI)

**Interfaces:**
- Produces:
  - `pub struct WinHost` cu `impl Host`:
    - SCM prin `windows-service`;
    - `run` prin `std::process::Command` cu `CREATE_NO_WINDOW` (`std::os::windows::process::CommandExt`);
    - `processes` și `trusted_roots` prin `guardian_platform::current()`;
    - firewall prin `netsh advfirewall firewall add|delete|show rule`, cu cale absolută, câte o regulă TCP și una UDP sub același nume;
    - `io_path` traduce `System32` în `Sysnative` doar când procesul e WOW64 (`PROCESSOR_ARCHITEW6432` setat);
  - `is_elevated() -> bool` (`GetTokenInformation(TokenElevation)`);
  - `attach_parent_console() -> bool` (`AttachConsole(ATTACH_PARENT_PROCESS)`; nu se apelează când stdout e deja redirecționat).
- [ ] **Step 1:** Scrie testele Windows:
  - `elevation_is_reported` (doar că nu cade);
  - `io_path_maps_system32_only_under_wow64`;
  - `missing_service_is_none`;
  - `processes_include_this_test`;
  - `netsh_args_for_tcp_and_udp`: funcția pură `firewall_args(name, port, proto) -> Vec<String>` trăiește în `model`, iar testul ei rulează peste tot.
- [ ] **Step 2:** `cargo test -p guardian-setup --features testing --test model` (pentru `firewall_args`) — Expected: FAIL; apoi `cargo clippy -p guardian-setup -p guardian-platform --all-targets --features guardian-setup/testing --target x86_64-pc-windows-gnu -- -D warnings` — Expected: erori de compilare până la implementare.
- [ ] **Step 3:** Implementează; fiecare bloc `unsafe` are un comentariu `// SAFETY:`.
- [ ] **Step 4:** Testul `model` trece; clippy e curat pe `x86_64-pc-windows-gnu` și `i686-pc-windows-gnu`.
- [ ] **Step 5:** Commit `setup: the Windows host (service manager, firewall, elevation)`.

### Task 5: `guardian-applog`, crate-ul comun de logging

**Files:**
- Create: `crates/guardian-applog/{Cargo.toml,src/lib.rs}` (conținutul lui `guardian-console/src/logging.rs`)
- Modify: `crates/guardian-console/{Cargo.toml,src/lib.rs,src/main.rs}`, cu `logging.rs` șters; `crates/guardian-console/tests/logging.rs` se mută în `crates/guardian-applog/tests/logging.rs`
- Test: `crates/guardian-applog/tests/logging.rs`

**Interfaces:**
- Produces: `guardian_applog::{rotate(path, max_bytes) -> io::Result<()>, init_logging(path) -> bool, log_panics()}`, cu același comportament ca în 3a: 1 MiB, `.1` și `.2`, `RUST_LOG`.
- [ ] **Step 1:** Mută testele consolei în noul crate; ele nu compilează (crate-ul nu există) — Expected: FAIL.
- [ ] **Step 2:** Mută codul; consola îl importă din `guardian-applog`.
- [ ] **Step 3:** `cargo test -p guardian-applog && cargo test -p guardian-console --features testing` — Expected: PASS.
- [ ] **Step 4:** Commit `applog: share log rotation and the panic hook between the console and setup`.

### Task 6: Fereastra și binarul `guardian-setup.exe`

**Files:**
- Create: `crates/guardian-setup/src/{ui.rs,app.rs,main.rs,elevate.rs}`
- Test: `crates/guardian-setup/tests/{ui.rs,binary.rs}`

**Interfaces:**
- Consumes: Task 3 (`run_op`, `ui_ops`), Task 5.
- Produces:
  - `pub struct SetupApp<H: Host + 'static>`: operațiile rulează pe un fir de lucru; progresul vine pe un canal și cere repaint;
  - ecranele (textele exacte):
    - **Not installed:** câmpurile `Server address`, `Certificate SHA-256 (optional)`, `Enrollment code` (parolă) și butonul `Install`;
    - dacă există 2.x: bannerul `Guardian 2.x is installed on this computer. It will be removed during the installation.` și o confirmare `Remove Guardian 2.x and install 3.0?` (No / Yes);
    - **Installed:** un card cu `Computer #<id> · <server>`, `Service: Running/Stopped`, `Local fallback port <p>: open/closed` și butoanele `Start`, `Stop`, `Restart`, `Update`, `Re-enroll…`, `Uninstall…`;
    - tab-ul `Whitelist`: lista, un câmp `Program name`, `Add`, `Remove` pe rând;
    - tab-ul `Processes` (doar citire);
    - erorile apar într-un banner roșu, iar fereastra rămâne deschisă;
  - `main.rs`:
    - fără argumente: fereastra (wgpu, cu glow ca rezervă, ca în 3a);
    - cu argumente: `cli::main_cli`, după `attach_parent_console()`;
    - fără consolă părinte, rezultatul apare într-un dialog `rfd`;
  - `elevate.rs`:
    - `pub fn should_relaunch(elevated: bool, args: &[OsString]) -> bool`: fals dacă există deja `--elevated-relaunch`;
    - `pub fn relaunch_args(exe: &Path) -> Vec<String>` (decizia 9).
- [ ] **Step 1:** Scrie testele:
  - kittest:
    - `not_installed_shows_the_install_form`;
    - `install_sends_the_values` (`FakeHost` vede `enroll` cu serverul și codul);
    - `legacy_install_asks_first`;
    - `installed_shows_service_and_buttons`;
    - `stop_and_start_call_the_service`;
    - `whitelist_tab_adds_and_removes`;
    - `op_error_is_shown_and_window_stays`;
  - pur: `relaunch_happens_once`;
  - binar: `version_prints` și `uninstall_without_yes_exits_2`.
- [ ] **Step 2:** `cargo test -p guardian-setup --features testing --test ui --test binary` — Expected: FAIL.
- [ ] **Step 3:** Implementează.
- [ ] **Step 4:** Același test — Expected: PASS; clippy curat pe ambele ținte Windows.
- [ ] **Step 5:** Commit `setup: the window and the guardian-setup binary`.

### Task 7: Consola dă comanda de setup

**Files:**
- Modify: `crates/guardian-console/src/connect_info.rs`, `crates/guardian-console/src/ui/connect_window.rs`
- Test: `crates/guardian-console/tests/connect_info.rs`, `tests/ui_admin.rs`

**Interfaces:**
- Produces: `windows_command(t, code)` = `guardian-setup.exe install --server <wss://…> [--cert-sha256 <pin>] --code <code>` pe o singură linie (înainte: două comenzi `guardian-agent.exe`). Eticheta din fereastră devine `Windows (run as administrator, from the unpacked agent-windows folder)`.
- [ ] **Step 1:** Schimbă testele existente la textul nou — Expected: FAIL.
- [ ] **Step 2:** Implementează.
- [ ] **Step 3:** `cargo test -p guardian-console --features testing --test connect_info --test ui_admin` — Expected: PASS.
- [ ] **Step 4:** Commit `console: give the guardian-setup command for Windows agents`.

### Task 8: Arhiva 3.0 — `packaging/` și `scripts/package.sh`

**Files:**
- Modify/rename:
  - `packaging/server-linux-v3/` devine `packaging/server-linux/`, iar `packaging/agent-linux-v3/` devine `packaging/agent-linux/`;
  - conținutul vechi (Go: `server.env`, Docker, `agent.env`) se șterge;
  - `packaging/agent-windows/` și `packaging/server-windows/` se rescriu; apare `packaging/console-windows/`.
- Create:
  - `packaging/server-windows/README.txt`: `guardian-server.exe service install`, firewall 8443, amprenta din log;
  - `packaging/console-windows/README.txt`;
  - `packaging/agent-windows/README.txt`: dublu clic pe setup sau comanda din Connection info.
- Rewrite: `scripts/package.sh` (doar asamblează, nu compilează).
- Create: `scripts/test-package.sh`
- Modify: `VERSION` (`3.0.0`), `Cargo.toml` (`version = "3.0.0"`)

**Interfaces:**
- Produces: `scripts/package.sh --linux DIR --win64 DIR --win32 DIR [--out DIR]`. Directoarele țin binarele deja construite (`guardian-server`, `guardian-cli`, `guardian-agent`, `guardian-worker`; în `win64`: `guardian-server.exe`, `guardian-cli.exe`, `guardian-agent.exe`, `guardian-worker.exe`; în `win32`: `guardian-agent.exe`, `guardian-worker.exe`, `guardian-console.exe`, `guardian-setup.exe`). Rezultatul:
  ```
  guardian-<VERSION>/
    server-linux/    guardian-server, guardian-cli, guardian-server.service, install.sh
    server-windows/  guardian-server.exe, guardian-cli.exe, README.txt
    console-windows/ guardian-console.exe, README.txt
    agent-windows/   guardian-setup.exe, README.txt, bin/x64/{guardian-agent,guardian-worker}.exe, bin/x86/{…}
    agent-linux/     guardian-agent, guardian-worker, guardian-agent.service, install.sh, uninstall.sh
  ```
  Un binar lipsă oprește scriptul cu un mesaj care îl numește. Versiunea vine din `VERSION`.
- [ ] **Step 1:** Scrie `scripts/test-package.sh`:
  - creează binare false în trei directoare temporare și rulează `package.sh`;
  - verifică cu `unzip -l` fiecare cale de mai sus și lipsa `guardian.apk` și a oricărui `.ps1`/`.bat`;
  - verifică faptul că un binar lipsă dă un cod nenul și un mesaj cu numele lui.
- [ ] **Step 2:** `bash scripts/test-package.sh` — Expected: FAIL.
- [ ] **Step 3:** Rescrie `package.sh` și `packaging/`; `install.sh` de pe Linux copiază și `guardian-cli` pentru server.
- [ ] **Step 4:** `bash scripts/test-package.sh && sh -n packaging/*/install.sh && sh -n packaging/agent-linux/uninstall.sh` — Expected: PASS.
- [ ] **Step 5:** Commit `release: pack the 3.0 archive from prebuilt binaries`.

### Task 9: CI-ul de release

**Files:**
- Rewrite: `.github/workflows/release.yml`
- Modify: `.github/workflows/rust.yml` (jobul `windows` rulează și `cargo test -p guardian-setup --features testing` și construiește `guardian-setup.exe` pe `i686-pc-windows-msvc`; dispar referințele la căile `-v3`)
- Create: `scripts/check-workflows.py` (validează YAML-ul cu `python3 -I`, fără dependențe: `yaml` dacă există, altfel o verificare simplă de indentare și chei)

**Interfaces:**
- `release.yml` (tot doar la push pe `main`; publică doar dacă tag-ul `VERSION` nu există):
  - `linux`: `cargo build --release` pentru `guardian-server`, `guardian-cli`, `guardian-agent`, `guardian-worker`, plus testele workspace-ului; încarcă artifact-ul `bin-linux`;
  - `windows` (`windows-latest`):
    - `x86_64-pc-windows-msvc`: server, CLI, agent, worker → `bin-win64`;
    - `i686-pc-windows-msvc`: agent, worker, console, setup → `bin-win32`;
    - rulează și `cargo test -p guardian-setup --features testing` și `-p guardian-console --features testing`;
  - `package` (`ubuntu-latest`, `needs: [linux, windows]`): descarcă cele trei artifact-uri, rulează `scripts/package.sh`, încarcă `guardian-<v>.zip`, iar dacă e versiune nouă, `gh release create`;
  - **semnarea Authenticode:** doar dacă există secretul `WINDOWS_CERT_PFX_BASE64` (cu `WINDOWS_CERT_PASSWORD`), cu `signtool` în jobul `windows`; fără el, un `::warning::` și continuă (`rust:` spec, cerința 32).
  - Secretele APK nu mai sunt folosite.
- [ ] **Step 1:** Scrie `scripts/check-workflows.py`:
  - fiecare workflow se încarcă;
  - `release.yml` are joburile `linux`, `windows` și `package`, iar `package.needs` le conține pe celelalte două;
  - niciun pas nu mai pomenește `go`, `flutter` sau `whitelist-gui`.

  Rulează-l pe workflow-urile actuale — Expected: FAIL (`release.yml` e cel cu Go).
- [ ] **Step 2:** Rescrie workflow-urile.
- [ ] **Step 3:** `python3 -I scripts/check-workflows.py` — Expected: PASS.
- [ ] **Step 4:** Commit `ci: build and publish the 3.0 archive`.

### Task 10: Ștergerea codului Go

**Files:**
- Delete:
  - `server/`, `agent/`, `tools/server-gui/`, `tools/whitelist-gui/`, `tools/mkico/`, `go.mod` (rădăcina);
  - `packaging/agent-windows/bin/`, `Install-Guardian.bat`, `Uninstall-Guardian.bat` și `agent.env` vechi, dacă au rămas după Task 8.
- Modify: `CHANGELOG.md` (intrarea `3.0.0`), `.gitignore` (regulile Go-ului)
- Create: `scripts/check-no-go.sh`

**Interfaces:**
- `scripts/check-no-go.sh` iese cu un cod nenul dacă:
  - mai există fișiere `*.go`, `go.mod` sau `go.sum` în repo, în afara lui `mobile/` (care nu are);
  - `AGENTS.md`, `README.md` sau `.github/` pomenesc căile șterse: `tools/whitelist-gui`, `tools/server-gui`, `server/guardian`, `agent/main.go`, `procsentinel-agent`.
- [ ] **Step 1:** Scrie scriptul; rulat acum — Expected: FAIL.
- [ ] **Step 2:** Șterge codul și curăță referințele din cod și din `.gitignore`. Documentația vine în Task 11, așa că verificarea `AGENTS.md`/`README.md` din script se activează abia acolo, cu flag-ul `--docs`.
- [ ] **Step 3:** `bash scripts/check-no-go.sh && cargo build --workspace && cargo test --workspace --all-features` — Expected: PASS (nimic din Rust nu depindea de Go).
- [ ] **Step 4:** Commit `repo: remove the Go server, agent and tools`.

### Task 11: Documentația de release (§17)

**Files:**
- Rename/rewrite:
  - `specs/server-v3.md` → `specs/server.md`, `specs/agent-v3.md` → `specs/agent.md`, `specs/console-v3.md` → `specs/console.md`, `specs/local-fallback-v3.md` → `specs/local-fallback.md`; legăturile dintre ele se actualizează;
  - `specs/api.md` se rescrie ca referință gRPC: fiecare serviciu și RPC din `proto/guardian/v1/api.proto`, cu rolul cerut și erorile, plus protocolul WebSocket pe scurt, cu trimitere la `server.md`.
- Create: `specs/setup.md` (`guardian-setup`: operațiile, CLI, codurile de ieșire, căile, legacy, pașii manuali pe Windows)
- Delete: `specs/whitelist-gui.md`, `specs/2026-11-13-windows-server-gui-design.md` (decizia 13)
- Rewrite: `AGENTS.md` (doar 3.0: crate-uri, comenzi, reguli, arhivă; secțiunile Go dispar), `README.md` (ce e Guardian 3.0, instalare server, consolă, agenți, modul de rezervă, build, fără APK)
- Modify:
  - `docs/index.html`: secțiunile de funcții și de instalare descriu 3.0 (consolă, profiluri pe sală, modul de rezervă, `guardian-setup`); cheile i18n noi sau schimbate există în toate cele 5 limbi; secțiunea aplicației Android spune că aplicația revine după 3.0;
  - `docs/research/2026-11-17-veyon-analysis.md` (notă cu deciziile 3.0: gRPC, profiluri, fără compatibilitate);
  - `specs/2026-11-17-rust-rewrite-design.md` §14 (nota „3c făcută”);
  - `/Users/poligon/Projects/PROJECTS.md` (în afara repo-ului).
- [ ] **Step 1:** Activează `--docs` în `check-no-go.sh` și adaugă `scripts/check-docs.sh`. Acesta verifică:
  - fiecare link relativ `.md` din `specs/`, `README.md` și `AGENTS.md` duce la un fișier existent;
  - fiecare cheie i18n din `docs/index.html` are o valoare în toate cele 5 limbi.

  Rulează-l — Expected: FAIL (linkuri spre `*-v3.md`, referințe Go).
- [ ] **Step 2:** Rescrie și mută documentele.
- [ ] **Step 3:** `bash scripts/check-no-go.sh --docs && bash scripts/check-docs.sh` — Expected: PASS. Pagina se verifică și vizual: `cd docs && python3 -m http.server 8099`, deschisă în browser-ul din aplicație, fără erori în consolă.
- [ ] **Step 4:** Verificarea completă:
  - `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && cargo deny check`;
  - clippy pe ambele ținte Windows;
  - suita în containerul Linux, cu `real_linux`, `real_embedded` și `real_fallback`;
  - `bash scripts/test-package.sh`.

  Expected: totul trece.
- [ ] **Step 5:** Commit `docs: describe Guardian 3.0`. `PROJECTS.md` e în afara repo-ului, deci nu intră în commit.
