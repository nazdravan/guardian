# Analiza Veyon pentru rescrierea Guardian în Rust

> Data: 2026-11-17 · Sursă: [github.com/veyon/veyon](https://github.com/veyon/veyon) (branch `main`, 997 fișiere) + [docs.veyon.io](https://docs.veyon.io/en/latest/)
> Scop: documentarea completă a felului în care e construit Veyon (arhitectură, protocol, plugin-uri, autentificare, config) ca să putem reimplementa **orice** funcțional în Rust și să rescriem Guardian-ul actual folosind cele mai bune idei din Veyon.

---

## 0. Rezumat executiv

Veyon este software GPL-2.0 de **monitorizare și control al calculatoarelor** într-o clasă/laborator. Este scris în **C++ / Qt5**, puternic modular (o librărie `core` + ~17 plugin-uri încărcate dinamic), multiplataformă (Windows, Linux, build Android pentru Master).

Ideea arhitecturală centrală — și cea mai valoroasă pentru noi — este:

1. **Totul se tunelează printr-o conexiune VNC/RFB.** Imaginea ecranului vine prin VNC standard; comenzile (lock, mesaj, power, start program…) sunt mesaje custom RFB (`FeatureMessage`, tip RFB 41) peste **aceeași** conexiune. Nu există un al doilea canal de comenzi.
2. **Model cu 3 roluri pe fiecare calculator client**: `Service` (rulează ca SYSTEM, supraveghează sesiunile) → pornește `Server` (per sesiune, deține VNC-ul și logica) → pornește `Worker`-i (în contextul userului, pentru GUI: tray, lock screen, demo, dialoguri).
3. **Fiecare funcționalitate e un plugin** care implementează aceeași interfață (`FeatureProviderInterface`) și decide singur ce face pe partea de Master, de Server și de Worker. Adăugarea unei funcții noi = un plugin nou, fără să atingi nucleul.

Guardian-ul actual rezolvă o problemă **mai îngustă** (blocare procese/aplicații + power + blocare computere, prin polling HTTP la un server central), deci NU avem nevoie de tot Veyon. Dar putem împrumuta: modelul de plugin/feature, separarea Service/Server/Worker pentru operații privilegiate vs. GUI, modelul de autentificare pe chei, access control, și (dacă vrem monitorizare/remote) stiva VNC. Secțiunea 13–14 dă maparea concretă pe crate-uri Rust și o propunere de arhitectură.

**Despre „plugin-uri care lipsesc în repo":** nu există. Veyon e integral open-source sub GPL-2.0; toate cele 17 plugin-uri sunt în repo (inventar complet în §7). Nu există add-on-uri comerciale separate. Singurele artefacte „din afară" sunt: clientul Python pentru WebAPI (de fapt tot în repo, `plugins/webapi/python/`), binarele 3rd-party VNC vendorizate (UltraVNC, x11vnc, libvncserver — în `3rdparty/`), și proiectul legacy iTALC din care s-a născut Veyon.

---

## 1. Componente (procese separate)

| Componentă | Dir repo | Rol | Privilegii | Echivalent Guardian |
|---|---|---|---|---|
| **Veyon Master** | `master/` | Aplicația profesorului: grila de thumbnail-uri, selecție computere, declanșare funcții | user | ~ aplicația mobilă Flutter + GUI server Windows |
| **Veyon Service** | `service/` (doar `main.cpp`) | Serviciu non-grafic; supraveghează sesiunile de user și pornește `Server` în fiecare | SYSTEM/root | ~ agent Windows service |
| **Veyon Server** | `server/` | Inima clientului: ascultă pe 11100, ține conexiunea VNC, rutează `FeatureMessage`, aplică access control | ridicate (pornit de Service) | ~ logica din agent |
| **Veyon Worker** | `worker/` | Proces efemer pornit de Server pentru lucruri care trebuie în sesiunea userului / izolat: tray icon, lock screen, demo server/client, dialoguri | context user | — (Guardian n-are) |
| **Veyon Configurator** | `configurator/` | GUI de configurare locală (toate secțiunile) | ridicate | ~ Guardian Console (`tools/whitelist-gui`) |
| **Veyon CLI** | `cli/` | `veyon-cli` pentru config/automatizare/feature fără GUI | de regulă admin | — |

**De ce Service → Server → Worker?** Pe Windows, un serviciu rulează în sesiunea 0 și **nu poate** afișa UI sau captura ecranul userului. De aceea Service (sesiune 0) pornește câte un Server în fiecare sesiune de user cu privilegii ridicate (ca userul să nu-l poată omorî), iar Server pornește Worker-i efemeri când trebuie să deseneze ceva pe ecranul userului (lock, mesaj, demo). Aceasta este separarea pe care Guardian o are parțial (agent ca serviciu) dar nu complet — e lecția #1 de preluat.

---

## 2. Model de rețea și porturi

| Serviciu | Port implicit | Bind | Notă |
|---|---|---|---|
| Veyon Server (VNC + feature) | **11100** | rețea | Portul principal; excepția de firewall îl acoperă + demo |
| Internal VNC server | **11200** | localhost | Serverul VNC real; Server-ul face proxy spre el |
| Feature manager | **11300** | localhost | Pornește/oprește procesele Worker |
| Demo server | **11400** | rețea | VNC „high-efficiency" pentru broadcast ecran profesor |
| WebAPI | **11080** | rețea | Doar dacă e activat; HTTP sau HTTPS/TLS 1.3 |

- **Multi-session** (terminal/RDP): fiecare sesiune primește `port_bază + VEYON_SESSION_ID` (ex. sesiune 4 → `11104`). Max implicit 100 sesiuni → porturile trebuie spațiate >100.
- **„Allow connections from localhost only"**: blochează accesul de rețea la Server (implicit off).
- **Thumbnail update interval** (Master): 1000 ms implicit.
- **Network object directory update interval**: 60 s.
- Arhitectural: Master deschide o conexiune TCP per computer monitorizat spre 11100; pe ea curge VNC-ul (framebuffer) + `FeatureMessage`-ele. Pentru demo, Server-ul fiecărui client se conectează ca VNC-client la demo serverul profesorului (11400).

---

## 3. Protocolul de comunicare (esențial)

Veyon **nu** inventează un protocol nou de transport. Folosește **RFB (VNC)** și împachetează comenzile proprii ca mesaje RFB custom.

- **Handshake**: VNC standard + o schemă de securitate Veyon proprie (`RfbVeyonAuth.h`) prin care se negociază metoda de autentificare (cheie / logon / token) înainte de a da acces la framebuffer.
- **`FeatureMessage`** (`core/src/FeatureMessage.h`):
  - câmpuri: `featureUid` (QUuid, identifică plugin-ul/funcția), `command` (enum per-feature, cu `Default=0`, `Init=-2`), `arguments` (map cheie-index → QVariant).
  - se serializează fie „plain" (`sendPlain`), fie ca **mesaj RFB** cu `RfbMessageType = 41` (`sendAsRfbMessage`) — adică injectat în fluxul VNC existent.
  - serializare pe sârmă prin `VariantArrayMessage` / `VariantStream` (QVariant ↔ bytes).
- **Rutare**: când Server-ul primește un mesaj RFB 41, îl dă plugin-ului cu `featureUid`-ul respectiv via `handleFeatureMessage(VeyonServerInterface&, ...)`. Dacă operația ține de UI/user, Server-ul pornește un Worker și-i retrimite mesajul (`handleFeatureMessage(VeyonWorkerInterface&, ...)`).

**Implicație pentru Rust**: dacă vrem paritate de monitorizare/remote, avem nevoie de un client+server RFB. Dacă NU facem ecran (ca Guardian azi), putem păstra doar *ideea* `FeatureMessage` peste un transport propriu (vezi §13).

---

## 4. Sistemul de plugin-uri și feature-uri

### 4.1 `PluginInterface` (`core/src/PluginInterface.h`)
Fiecare plugin expune: `uid()` (QUuid stabil), `version()`, `name()`, `description()`, `vendor()`, `copyright()`, `flags()`, `upgrade(oldVersion)`. Plugin-urile sunt `.dll`/`.so` încărcate dinamic de `PluginManager`.

Sub-interfețe pe care un plugin le poate implementa în plus:
- `FeatureProviderInterface` — oferă funcții (majoritatea plugin-urilor).
- `ConfigurationPagePluginInterface` — adaugă o pagină în Configurator.
- `CommandLinePluginInterface` — adaugă un modul la `veyon-cli`.
- `NetworkObjectDirectoryPluginInterface` — sursă de computere/locații (builtin, LDAP).
- `PlatformPluginInterface` — abstracția de OS (windows/linux).
- `VncServerPluginInterface` — implementare de server VNC.
- `UserGroupsBackendInterface` — sursă de grupuri pentru access control.

### 4.2 `Feature` (`core/src/Feature.h`)
O funcție = un obiect `Feature` cu:
- `name`, `uid` (QUuid fix), `parentUid` (pentru sub-funcții, ex. „PowerDownNow" e copilul lui „PowerDown"), `displayName`, `displayNameActive`, `description`, `iconUrl`, `shortcut`.
- **`Flags`** — cheia modelului:
  - `Mode` (0x01) — mod exclusiv (monitorizare, demo, lock); doar unul activ.
  - `Action` (0x02) — acțiune one-shot (reboot, screenshot).
  - `Session` (0x04) — ține de sesiune (login/logout).
  - `Meta` (0x08) — grupare logică.
  - `Option` (0x10) / `Checked` (0x20) — opțiuni/toggle.
  - componente: `Master` (0x100), `Service` (0x200), `Worker` (0x400), `AllComponents = Master|Service|Worker`.
  - `Builtin` (0x1000).

### 4.3 `FeatureProviderInterface` (`core/src/FeatureProviderInterface.h`)
Un plugin decide *singur* ce se întâmplă pe fiecare parte:
- `featureList()` — funcțiile oferite.
- `controlFeature(uid, Operation{Initialize|Start|Stop}, args, interfaces)` — pe **Master**: construiește `FeatureMessage` și-l trimite către clienți (`sendFeatureMessage`).
- `startFeature()` / `stopFeature()` — variante high-level pe Master.
- `handleFeatureMessage(VeyonServerInterface&, ctx, msg)` — pe **Server** (client): execută operația de sistem.
- `handleFeatureMessage(VeyonWorkerInterface&, msg)` — pe **Worker**: execută operația de UI/user.
- `handleFeatureMessageFromWorker(...)`, `sendAsyncFeatureMessages(...)` — notificări inverse Worker→Server→Master.
- `isFeatureActive(server, uid)` — pentru moduri.

**Exemplu concret (PowerControl, `plugins/powercontrol/`):**
- Master `controlFeature(PowerDownDelayed, Start, {ShutdownTimeout:60})` → trimite `FeatureMessage`.
- Server primește, pornește un Worker.
- Worker `handleFeatureMessage` → `displayShutdownTimeout(60)` (dialog de numărătoare inversă).
- Pentru `PowerOn` nu se trimite mesaj la client (clientul e stins!) — Master trimite **Wake-on-LAN** (`broadcastWOLPacket(mac)`, UDP broadcast magic packet).
- Plugin-ul adaugă și modul CLI `power` (`veyon-cli power on <mac>`).

Acesta este tiparul pe care-l reimplementăm în Rust ca trait (§13).

### 4.4 `FeatureWorkerManager`
Pe Server rulează un „feature manager" (port 11300, localhost) care pornește/oprește procese Worker la cerere și le livrează mesajele. Workerul vorbește înapoi prin `FeatureWorkerManagerConnection`.

---

## 5. Inventar complet al plugin-urilor (17)

### Funcții (feature) vizibile profesorului
| Plugin | Dir | Ce face | Cum e implementat |
|---|---|---|---|
| **Demo** | `plugins/demo/` | Broadcast ecran profesor (fullscreen/fereastră) către toți; share ecran user/propriu; selecție monitor | Profesorul rulează `DemoServer` (VNC eficient pe 11400, cu token de acces); fiecare client rulează `DemoClient` (Worker) care se conectează și afișează. Argumente în mesaj: host, port, parolă, token, offset port, selecție ecran. `DemoServerProtocol`/`DemoServerConnection` custom pentru eficiență (key frames, memory/bandwidth limit). |
| **RemoteAccess** | `plugins/remoteaccess/` | View-only sau control complet al unui computer, într-o fereastră; comutare live view↔control; selecție ecran; trimitere Ctrl+Alt+Del | `RemoteAccessWidget` = un `VncView` pe conexiunea existentă, cu `UpdateMode::RemoteControl`. |
| **ScreenLock** | `plugins/screenlock/` | Blochează ecranul (acoperă cu imagine) și/sau input-ul pe computerele selectate | Mod (`Flag::Mode`). Worker afișează `LockWidget` fullscreen; blocarea input-ului e făcută de platform plugin (`InputBlockHelper` pe Linux, interception/SAS pe Windows). Funcționează și fără user logat (blochează doar input). |
| **Screenshot** | `plugins/screenshot/` | Capturează ecranul clientului și-l salvează pe Master | Master cere framebuffer-ul curent / un capture; se salvează în `%APPDATA%/Screenshots`. `ScreenshotManagementPanel` în Master. |
| **PowerControl** | `plugins/powercontrol/` | Power on (WOL), reboot, shutdown (now/confirmat/întârziat), install updates & shutdown | Vezi §4.3. WOL prin magic packet UDP; shutdown prin platform plugin. |
| **TextMessage** | `plugins/textmessage/` | Trimite mesaj text (cu titlu/iconiță) studenților | Master deschide `TextMessageDialog`; Worker afișează un `Toast`/dialog pe client. |
| **DesktopServices** | `plugins/desktopservices/` | Pornește un program la distanță / deschide un URL / predefiniri | Master: `StartAppDialog`, `OpenWebsiteDialog`; Server/Worker pornește procesul în sesiunea userului. Predefiniri în config. |
| **FileTransfer** | `plugins/filetransfer/` | Distribuie fișiere spre clienți și (4.10+) colectează fișiere de la clienți | `FileReadThread`, chunking, `FileTransferListModel` cu progres; colectare cu grupare pe atribut user, excludere wildcard, recursiv. |
| **UserSessionControl** | `plugins/usersessioncontrol/` | Logon în masă (start lecție), logoff în masă (end lecție) | `UserLoginDialog` pe Master trimite user+parolă; platform plugin face login-ul. `Flag::Session`. |

### Directory / autentificare / grupuri
| Plugin | Dir | Ce face |
|---|---|---|
| **BuiltinDirectory** | `plugins/builtindirectory/` | Sursă implicită de computere/locații din config local (JSON). Implementează `NetworkObjectDirectory`. |
| **LDAP** | `plugins/ldap/` | Computere, locații și grupuri din LDAP/AD. `LdapClient`, `LdapDirectory`, `LdapBrowseModel`, pagină de config + teste. Folosește kldap vendorizat. |
| **AuthKeys** | `plugins/authkeys/` | Managementul perechilor de chei (creare, import/export, tabel). Autentificare cu cheie privată/publică. |
| **SystemUserGroups** | `plugins/systemusergroups/` | Backend de grupuri = grupurile locale ale OS-ului, pentru access control. |

### Server VNC (sub-plugin-uri în `plugins/vncserver/`)
| Variantă | Dir | Platformă/caz |
|---|---|---|
| **x11vnc-builtin** | `vncserver/x11vnc-builtin/` | Linux X11 (x11vnc vendorizat). |
| **ultravnc-builtin** | `vncserver/ultravnc-builtin/` | Windows (UltraVNC vendorizat) + `LogoffEventFilter`. |
| **pipewire** | `vncserver/pipewire/` | Linux Wayland (PipeWire + xdg-desktop-portal, `PortalSession`). |
| **headless** | `vncserver/headless/` | Fără display fizic. |
| **external** | `vncserver/external/` | Un server VNC deja existent (dai port + parolă). |

### Platformă (`plugins/platform/`) — abstracția de OS
- `common/` — `LogonHelper`, `PersistentLogonCredentials`, `PlatformSessionManager`, `ServiceDataManager`.
- `windows/` — cel mai gras: `WindowsServiceCore` (serviciu + spawn server per sesiune via WTS), `WtsSessionManager`, `WindowsInputDeviceFunctions`, `WindowsUserFunctions` (logon/logoff), `WindowsSessionFunctions`, `WindowsNetworkFunctions`, `WindowsServiceControl`, `SasEventListener` (Ctrl+Alt+Del / SAS), `DesktopInputController`, `XEventLog`, firewall (`netfw.h`), interception.
- `linux/` — `LinuxServiceCore`, `LinuxUserFunctions` (PAM), `LinuxSessionFunctions` (logind/DBus), `LinuxInputDeviceFunctions`, `LinuxKeyboardInput`, `InputBlockHelper`, `auth-helper/` (helper DBus privilegiat `io.veyon.AuthHelper`).

### Altele
- **WebAPI** (`plugins/webapi/`) — REST/JSON peste HTTP(S) pe 11080: control de la distanță prin API (ex. integrare externă). Server intern bazat pe `qthttpserver` vendorizat; `WebApiController`, `WebApiAuthenticationProxy`, sesiuni cu lifetime/idle/auth timeout; client Python în `python/`.
- **Testing** (`plugins/testing/`) — modul CLI de test (`TestingCommandLinePlugin`).

---

## 6. Autentificare

Enum `AuthenticationCredentials::Type` (`core/src/AuthenticationCredentials.h`):
- `None`
- **`PrivateKey`** (0x01) — cheie privată RSA; Master semnează o provocare, Server verifică cu cheia publică din `%GLOBALAPPDATA%/keys/public/<nume>/`. Cel mai sigur; necesită distribuirea cheilor. `VEYON_AUTH_KEY_NAME` alege cheia.
- **`UserLogon`** (0x02) — user + parolă de sistem; validate pe client prin PAM (Linux) / LogonUser (Windows). Zero setup.
- **`Token`** (0x04) — token (folosit intern, ex. demo access token, WebAPI).

Cripto: `core/src/CryptoCore` (bazat pe **QCA** / OpenSSL) — chei RSA, semnături; `d3des.h` pentru parola VNC legacy. `RfbVeyonAuth.h` = schema RFB custom care negociază tipul la handshake. `ServerAuthenticationManager` (server) + `AuthenticationProxy` (core).

---

## 7. Access control

`core/src/AccessControlProvider` + `AccessControlRule` (+ UI în `configurator/`). Rulează **după** ce autentificarea reușește, la stabilirea conexiunii (`ServerAccessControlManager`). Trei moduri (config):
1. Acces pentru **orice user autentificat** (implicit).
2. Restricție la membrii unor **grupuri** (backend: system groups sau LDAP).
3. **Reguli** (`AccessControlRule`) — cele mai flexibile: condiții pe user/computer/grup/locație/oră, cu accept/deny, ordonare și concatenare logică; pot fi testate în Configurator (`AccessControlRulesTestDialog`).

---

## 8. Model de configurare

- `core/src/Configuration/` — `Object` (arbore de proprietăți), `Property`, `Store` (interfață), `LocalStore` (registry pe Windows / fișiere pe Linux), `JsonStore`, `Password` (parole criptate), `Proxy`, `UiMapping` (leagă proprietăți de widget-uri).
- `VeyonConfiguration` + `VeyonConfigurationProperties.h` — toate proprietățile tipizate.
- Config e **locală pe fiecare computer** → trebuie distribuită (export/import JSON, sau template-uri în `%GLOBALAPPDATA%/Config/Templates`).
- **Path variables** (`%APPDATA%`, `%GLOBALAPPDATA%` = `C:\ProgramData\Veyon` / `/etc/veyon`, `%HOME%`, `%TEMP%`, `%DESKTOP%`…).
- **Env variables**: `VEYON_AUTH_KEY_NAME`, `VEYON_LOG_LEVEL`, `VEYON_SESSION_ID`.
- Secțiuni config (din reference): General (UI, auth method, directory backend, user groups, logging), Service (session modes, porturi, firewall, VNC plugin, session metadata), Master (directoare, calitate imagine, thumbnail, comportament, listă features), Access control, Authentication keys, Demo server (update 100ms, keyframe 10s, memory 128MB, bandwidth 100MB/s), LDAP, File transfer, WebAPI.

---

## 9. Monitorizare (grila de thumbnail-uri)

- `core/src/MonitoringMode` — modul implicit; Master cere framebuffer scalat (`scaledFramebufferSize`) la interval (1000ms) cu calitate „Medium" (JPEG).
- `ComputerControlInterface` (`core/src/`) — obiectul prin care Master vorbește cu un client: `state()`, `screenSize()`, `scaledFramebuffer()`, `framebuffer()`, `screens()`, `activeFeatures()`, `designatedModeFeature()`, `updateMode()` (`Monitoring`/`RemoteView`/`RemoteControl`), `sendFeatureMessage()`.
- Master: `ComputerMonitoringModel`/`View`/`Widget`, `ComputerManager`, `UserConfig`, `SpotlightPanel`, `SlideshowPanel`, `ScreenshotManagementPanel`.

---

## 10. Build & distribuție (Veyon)

- **CMake** + Qt5; `CGO`-style cross-compile prin toolchain-uri (`Win32/Win64Toolchain.cmake`, `MinGWCrossCompile`).
- Fiecare plugin se compilează cu `BuildVeyonPlugin.cmake`.
- Dependențe: Qt5, X11, libjpeg, zlib, OpenSSL, PAM, procps, LZO, QCA, LDAP, SASL; VNC vendorizat (libvncserver, UltraVNC, x11vnc).
- CI: GitHub Actions (`build.yml`, `release.yml`) + GitLab CI; pachete pentru Debian/Ubuntu/Fedora/openSUSE/RHEL + installer Windows (NSIS).
- Teste: `tests/unit/` (puține) + `tests/libfuzzer/` (fuzzing pe parsarea protocolului VNC/variant — robustețe/securitate).

---

## 11. Lista completă de funcționalități (user-facing)

Monitorizare clasă · Remote view · Remote control · Demo (fullscreen/fereastră, share propriu/user) · Screen lock (ecran + input) · Text message · Start program · Open website · Distribuție fișiere · Colectare fișiere · Screenshot · Login în masă · Logout în masă · Power on (WOL) · Reboot · Shutdown (now/confirmat/întârziat) · Install updates & shutdown · Ctrl+Alt+Del remote · Selecție monitor · WebAPI (control extern).

---

## 12. Comparație Veyon ↔ Guardian actual

| Aspect | Veyon | Guardian azi |
|---|---|---|
| Problemă | Monitorizare + control vizual al ecranelor | Blocare procese/aplicații + blocare computere + power |
| Transport | Conexiune directă Master→client, VNC + FeatureMessage | Polling HTTP agent→server central (10–20s), REST+SQLite |
| Topologie | Peer-ish: profesorul se conectează la fiecare client | Centralizat: server central, agenți care trag config |
| Ecran | Da (thumbnails, remote, demo) | Nu |
| Privilegii | Service(SYSTEM)→Server→Worker(user) | Agent ca serviciu Windows |
| Plugin-uri | 17, încărcate dinamic, trait comun | Monolitic (Go), fără plugin system |
| Auth | Cheie / logon / token, handshake RFB | Bearer token (TOKEN client, ADMIN_TOKEN admin) |
| Access control | Provider + reguli + grupuri + LDAP | — |
| Config | Store local + distribuție/template | `.env` + server DB |
| Platforme | Win/Linux(X11+Wayland)/Android master | Agent Win/Linux/mac, server Linux, mobil Android |

**Concluzie**: Guardian ≈ un subset al lui „Service/Server + PowerControl + (un fel de) access control", dar centralizat prin polling în loc de conexiune directă. Din Veyon merită preluate: (a) modelul **plugin/feature** ca trait, (b) separarea **privilegiat vs. UI-în-sesiunea-userului**, (c) **auth pe chei** (nu doar bearer token partajat), (d) **access control cu reguli**, (e) opțional stiva **VNC** dacă vrem monitorizare/remote. NU trebuie copiat: cuplarea de Qt, VNC-ul vendorizat vechi, modelul de config pe registry.

---

## 13. Mapare Veyon → Rust (crate-uri + tipare)

### 13.1 Sistemul de plugin/feature → trait-uri + enum
Nu avem nevoie de `.dll`/`.so` dinamice (sursa complexității lui Veyon). În Rust: un `trait FeatureProvider` și înregistrare statică (`inventory` sau un `Vec<Box<dyn FeatureProvider>>` construit la pornire). Dacă chiar vrem plugin-uri dinamice: `abi_stable` sau `libloading` — dar recomand **static**.

```rust
pub struct FeatureId(pub Uuid);

bitflags::bitflags! {
    pub struct FeatureFlags: u32 {
        const MODE = 0x01; const ACTION = 0x02; const SESSION = 0x04;
        const MASTER = 0x100; const SERVICE = 0x200; const WORKER = 0x400;
    }
}

pub struct FeatureMessage { pub feature: FeatureId, pub command: i32, pub args: BTreeMap<String, Value> }

pub trait FeatureProvider: Send + Sync {
    fn features(&self) -> &[Feature];
    fn control(&self, f: FeatureId, op: Operation, args: &Args, targets: &[ClientHandle]) -> Result<()>; // master
    fn handle_on_server(&self, ctx: &ServerCtx, msg: &FeatureMessage) -> Result<bool> { Ok(false) }
    fn handle_on_worker(&self, ctx: &WorkerCtx, msg: &FeatureMessage) -> Result<bool> { Ok(false) }
}
```
- Serializare `FeatureMessage`: `serde` + `bincode`/`postcard` (în loc de `VariantArrayMessage`).

### 13.2 Transport
| Nevoie | Crate Rust |
|---|---|
| Server/client TCP async | `tokio` |
| Mesaje lungime-prefixate | `tokio-util` (`LengthDelimitedCodec`) + `serde` |
| TLS (în loc de schema RFB custom) | `rustls` / `tokio-rustls` |
| REST (WebAPI echiv.) | `axum` + `tower` |
| gRPC (alternativă la FeatureMessage) | `tonic` + `prost` |

Recomandare: dacă **nu** facem ecran, înlocuim tunelul RFB cu un canal propriu TLS (rustls) peste tokio, pe care curg `FeatureMessage`-urile serde. Dacă **facem** ecran, vezi 13.5.

### 13.3 Autentificare & cripto
| Nevoie | Crate |
|---|---|
| Chei RSA + semnături (ca PrivateKey auth) | `rsa` + `rand` |
| Ed25519 (modern, recomandat în loc de RSA) | `ed25519-dalek` |
| Hash/TLS primitives | `ring` / `rustls` |
| Token-uri | `jsonwebtoken` (dacă vrem JWT) sau token random `rand` |
| Logon Windows | `windows` crate (`LogonUserW`) |
| PAM Linux | `pam` / `pam-client` |

### 13.4 Platformă (service + sesiuni + input + power)
| Nevoie | Windows | Linux |
|---|---|---|
| Serviciu | `windows-service` crate | `systemd` unit + `sd-notify` |
| Enumerare/spawn în sesiuni user | `windows` (WTS API: `WTSEnumerateSessions`, `CreateProcessAsUser`) | `logind` prin `zbus` (DBus) |
| Shutdown/reboot | `windows` (`ExitWindowsEx`/`InitiateSystemShutdown`) | `zbus` → logind `PowerOff`/`Reboot`, sau `systemctl` |
| Wake-on-LAN | socket UDP broadcast manual (`std::net::UdpSocket`) — magic packet | la fel |
| Blocare input | `windows` (`BlockInput`, low-level hooks) | `evdev` / X11 `XGrab*` (`x11rb`) |
| Listare/kill procese (ce face Guardian azi) | `sysinfo` | `sysinfo` |
| DBus (logind, portal) | — | `zbus` |
| Firewall | `netsh` prin `std::process::Command` | `iptables`/`firewalld` |

### 13.5 Dacă vrem ecran (monitorizare/remote/demo)
| Nevoie | Crate Rust |
|---|---|
| Server VNC/RFB | `vnc-rs`, sau implementare proprie RFB (fuzz-tested ca Veyon) |
| Client VNC | `vnc` / `vnc-rs` |
| Captură ecran | `scap` (modern, DXGI/ScreenCaptureKit/PipeWire), `xcap`, sau `windows-capture` |
| Encodare JPEG (thumbnails) | `image` / `jpeg-encoder` / `turbojpeg` |
| Wayland capture | `scap` (folosește PipeWire portal, ca plugin-ul pipewire din Veyon) |

### 13.6 GUI (Master + Configurator)
| Opțiune | Crate | Notă |
|---|---|---|
| Nativ Rust imediat | **`egui`/`eframe`** | se potrivește cu skill-ul `forge-rust-app` din acest workspace; rapid pentru grilă de thumbnails |
| Nativ „greu" | `slint`, `gtk4-rs`, `iced` | Slint are suport bun cross-platform |
| Web/desktop hibrid | `tauri` | dacă vrem reuse cu un frontend web |
| Mobil (păstrăm Flutter) | — | aplicația Guardian mobilă poate rămâne Flutter, vorbind cu serverul Rust prin REST/gRPC |

### 13.7 Config, storage, logging
| Nevoie | Crate |
|---|---|
| Config tipizat | `serde` + `toml`/`json` (în loc de registry/LocalStore) |
| DB (dacă păstrăm serverul central Guardian) | `rusqlite` / `sqlx` (SQLite, ca azi) |
| Logging | `tracing` + `tracing-subscriber` (+ journald/eventlog appenders) |
| CLI | `clap` (echiv. `veyon-cli`) |

---

## 14. Propunere de arhitectură Rust pentru Guardian (rescriere)

Un singur workspace Cargo cu crate-uri, oglindind separarea Veyon dar fără Qt și fără plugin dinamic:

```
guardian-rs/                     (Cargo workspace)
├── crates/
│   ├── core/            # Feature/FeatureMessage/FeatureProvider trait, auth, config, protocol (serde+tls)
│   ├── proto/           # tipuri serializabile partajate (serde) — sau tonic/prost dacă gRPC
│   ├── agent/           # = Veyon Service+Server: serviciu, ascultă, rutează FeatureMessage, access control
│   ├── worker/          # operații în sesiunea userului (dialog shutdown, lock, mesaj) — doar dacă e nevoie de UI
│   ├── platform/        # trait PlatformFunctions + impl windows/ (windows crate) + linux/ (zbus, sysinfo)
│   ├── master/          # GUI profesor (egui) SAU rămâne doar serverul central + mobil Flutter
│   ├── server-central/  # (dacă păstrăm modelul centralizat Guardian) axum + sqlite, endpoint-urile actuale
│   ├── cli/             # clap, echiv. veyon-cli
│   └── features/        # câte un modul per funcție: appblock, powercontrol, computerblock, (screenlock), (message)
```

Decizii cheie de luat (vezi §15):
1. ~~**Centralizat (polling) sau conexiune directă?**~~ → **Decis: HIBRID** (2026-11-17) — agent cu conexiune persistentă ieșită spre server (WebSocket/TLS), comenzi instant, fără porturi deschise pe agenți. Vezi §15.
2. ~~**Vrem ecran (monitorizare/remote)?**~~ → **Decis: NU** (2026-11-17). Fără stivă VNC/captură; proiect mult mai mic decât Veyon.
3. **Auth**: trecem de la bearer token partajat la chei (ed25519) per-computer ca Veyon? Recomand da.
4. ~~**GUI**~~ → **Decis** (2026-11-17): Flutter pentru mobil + egui pentru GUI-urile Windows. Server și agent rulează pe Linux **și** Windows. Vezi §15.

---

## 15. Decizii & următorii pași

**Decizii luate:**
- ✅ **Fără ecran** (2026-11-17): Guardian Rust NU include monitorizare de ecran / remote control / demo. Rămâne la funcționalul actual (blocare aplicații/procese + blocare computere + power + mesaje). → Dispare toată stiva VNC/captură (§13.5 irelevantă), proiectul devine semnificativ mai mic decât Veyon; din Veyon preluăm doar modelul plugin/feature, separarea privilegiat/UI, auth pe chei și access control.

- ✅ **Arhitectură hibridă** (2026-11-17): fiecare agent deschide o conexiune **persistentă ieșită** spre server; serverul împinge comenzile instant pe ea, agentul trimite înapoi stare/heartbeat. Înlocuiește polling-ul de 10–20 s al Guardian-ului. Agenții nu deschid porturi de intrare → merge prin NAT/firewall de școală (rezolvă exact slăbiciunea modelului direct din Veyon). Transport recomandat: **WebSocket peste TLS** (`axum` ws pe server + `tokio-tungstenite` în agent), ca un singur stack `axum` să servească și stream-ul agenților, și REST-ul pentru Flutter/egui; alternativă: gRPC bidi stream (`tonic`). Pe stream circulă `FeatureMessage` serde (modelul Veyon, §4).
- ✅ **Platforme** (2026-11-17): **atât agentul, cât și serverul rulează pe Linux și pe Windows.** → agentul are trait `Platform` cu impl `windows/` (serviciu Windows, WTS, `windows` crate) și `linux/` (systemd, `zbus`/logind); serverul e un binar Rust cross-platform (systemd pe Linux, serviciu Windows sau încorporat în GUI-ul egui, ca `server-gui` de azi). macOS nu e în scope.
- ✅ **GUI** (2026-11-17): **Flutter rămâne pentru mobil** (vorbește REST cu serverul), iar **GUI-urile Windows** (`server-gui`, `whitelist-gui`, azi lxn/walk) se rescriu în **egui** (skill `forge-rust-app`).

**Următorul pas:** design (brainstorming → spec în `docs/specs/`) pe baza acestor decizii, apoi plan de implementare în `docs/plans/`.

**Pași tehnici propuși (după ce răspunzi):**
1. Brainstorming + spec pentru scope-ul exact al rescrierii (skill `superpowers:brainstorming`).
2. Prototip `core`: `FeatureProvider` trait + `FeatureMessage` serde + transport TLS/tokio.
3. Portare primei funcții (app-block, cea existentă în Guardian) ca `features/appblock` pe noul model.
4. Platform plugin Windows (serviciu + WTS + kill proces) cu `windows` crate.
5. Auth pe chei (ed25519) + access control minimal.
6. Abia apoi (dacă e în scope) stiva de ecran.

---

### Anexă: fișiere-cheie Veyon de citit la implementare
- Protocol: `core/src/FeatureMessage.h/.cpp`, `VariantArrayMessage`, `VncServerProtocol`, `VncClientProtocol`, `RfbVeyonAuth.h`.
- Plugin model: `core/src/Feature.h`, `FeatureProviderInterface.h`, `PluginInterface.h`, `PluginManager`, `FeatureWorkerManager`.
- Client: `server/src/ComputerControlServer`, `VncProxyServer`, `ServerAuthenticationManager`, `ServerAccessControlManager`.
- Master: `master/src/ComputerControlListModel`, `ComputerManager`, `VeyonMaster`.
- Exemplu feature simplu de copiat: `plugins/powercontrol/`, `plugins/textmessage/`, `plugins/screenlock/`.
- Platformă: `plugins/platform/windows/WindowsServiceCore`, `WtsSessionManager`; `plugins/platform/linux/LinuxServiceCore`.
