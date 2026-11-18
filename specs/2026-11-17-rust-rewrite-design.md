# Guardian 3.0 în Rust — design

Data: 2026-11-17. Status: draft, așteaptă aprobarea proprietarului. Verificare de builder: făcută (2026-11-17) pe varianta cu compatibilitate; răspunsurile care au rămas valabile sunt incluse aici.
Punct de plecare: Guardian simplu de pe `main` (2.2.0, Go + SQLite). Analiză de fond: [`docs/research/2026-11-17-veyon-analysis.md`](../docs/research/2026-11-17-veyon-analysis.md).

Notă de convenție: în acest repo spec-urile stau în `specs/`, planurile în `specs/plans/`.

---

## 1. Intenție

Guardian se rescrie în Rust ca **versiunea 3.0, cu o arhitectură nouă și curată, fără nimic moștenit**. Proiectul e al unui singur om, nu există utilizatori de care să depindă compatibilitatea, iar 3.0 se instalează de la zero (decizia proprietarului, 2026-11-17).

Ținta: un instrument pentru o școală, în care:

- **serverul de control** rulează fie **la distanță** (internet/cloud, ca serviciu pe Linux sau Windows), fie **încorporat în aplicația profesorului** (cât e deschisă, §9.6); agenții de pe calculatoarele elevilor se conectează **ei** la server și țin o conexiune permanentă, deci comenzile ajung pe loc și în școală nu se deschide niciun port spre internet;
- regulile de aplicații se organizează în **profiluri pe sală** („Lecție", „Examen", „Liber"…), iar profesorul schimbă profilul sălii lui dintr-un clic;
- **când pică internetul**, profesorul poate controla calculatoarele **local**, din rețeaua școlii, ca rezervă;
- funcțiile noi (mesaj, program/site, fișiere, oprire/restart cu numărătoare inversă) se adaugă pe un **model de feature-uri** luat din Veyon, fără nimic legat de ecran.

## 2. Cine și când

- **Administratorul școlii** — instalează serverul, înrolează calculatoarele, creează sălile, profilurile și conturile profesorilor.
- **Profesorul** — în timpul orei, din consola egui de pe calculatorul lui (Windows) sau din CLI: schimbă profilul sălii, trimite mesaje, pornește programe/site-uri, trimite și strânge fișiere, oprește/repornește calculatoarele sălii.
- **Elevul** — vede doar efectele: aplicații închise, mesaje, numărătoare inversă înainte de oprire, fișiere primite.

## 3. Obiective

1. **Server Rust** (librărie + binar) cu bază SQLite nouă, API **gRPC** pentru consolă, CLI și viitoarea aplicație mobilă, și **canal WebSocket/TLS** permanent pentru agenți.
2. **Model de politică pe profiluri**: profiluri definite de administrator, un profil activ pe fiecare sală, excepții pe calculator (§8.3).
3. **Identitate pe calculator**: înrolare cu cod (unic sau de sală) și chei ed25519 generate de agent; serverul nu ține niciun secret al agenților.
4. **Agent Rust** pe Windows (32 și 64 de biți) și Linux: aplicarea profilului (blacklist/whitelist cu protecția proceselor de sistem), detectare hibridă a proceselor, funcționare offline cu ultima stare, plus un **worker** în sesiunea userului pentru tot ce se afișează pe ecran.
5. **Conturi, săli și control acces**: un profesor comandă doar calculatoarele sălilor lui.
6. **Consolă egui** pentru profesor (urmașul `server-gui`): se conectează la un server la distanță sau pornește serverul încorporat; are modul de rezervă local.
7. **`guardian-setup`** (urmașul Guardian Console): instalare, diagnostic, update și dezinstalare agent, editarea `whitelist.txt` local.
8. **Mod de rezervă local**: când agentul pierde serverul, acceptă un set restrâns de comenzi direct de la consola profesorului din LAN; la revenirea serverului se întoarce la starea serverului.
9. **Funcțiile noi**, adăugate una câte una după ce fundația există.
10. **CLI** (`guardian-cli`) pentru administrare și automatizare.

## 4. Non-obiective

- **Fără compatibilitate cu 2.x**: nici API-ul REST vechi, nici agenții Go, nici migrarea `guardian.db`, nici upgrade-ul pe loc peste agentul Go. 3.0 se instalează de la zero (decizia proprietarului).
- **Nimic legat de ecran**: fără miniaturi, vizualizare/control de la distanță, demo, screenshot (decizia proprietarului).
- **Aplicația Flutter nu face parte din proiect**. Va fi rescrisă mai târziu, probabil pe gRPC. Până atunci **nu există aplicație mobilă** pentru 3.0 — consecință acceptată de proprietar.
- **Fără multi-tenant / Postgres / cabinet web** (acelea trăiesc pe `dev`): un server = o școală.
- **Fără LDAP/AD și fără sesiuni multiple (RDP/terminal server)**; arhitectura nu le blochează.
- **Modul local nu e un mod de lucru de sine stătător**: nu ține reguli proprii, nu editează profiluri, nu înrolează calculatoare.
- **macOS** nu e țintă.
- **Funcțiile din backlog** (§12.1) nu intră în etapele acestui design.

## 5. Anti-obiective (eșec chiar dacă „merge")

- Un calculator care devine **liber** pentru că a pierdut legătura cu serverul, a fost revocat, a pierdut worker-ul sau a primit o comandă locală (fail-secure).
- Un canal local prin care oricine din rețeaua școlii (un elev cu laptop) poate trimite comenzi agenților.
- O a doua copie a logicii de politică: serverul, agentul și consola folosesc aceleași tipuri și aceeași logică de decizie din crate-urile comune.
- O comandă locală din timpul căderii care rămâne activă după ce serverul a revenit.
- Secrete în clar în baza de date a serverului sau în fișiere citibile de orice user pe agent.
- Cod sau câmpuri păstrate „pentru 2.x" (`IDENTITY` ales de agent, `blocked`, comutatorul global `power`, `enabled` global, token comun pentru agenți).

## 6. Constrângeri

- **Platforme**: serverul și agentul rulează pe **Linux și Windows**. Agentul Windows există pe **32 și 64 de biți**; consola și `guardian-setup` sunt un binar Windows de 32 de biți care merge pe ambele (Windows 10+; `i686-pc-windows-msvc` e tier 1).
- **Build din CI**: totul se cross-compilează sau se construiește în GitHub Actions; fără dependențe native grele care cer toolchain-uri speciale.
- **Licență MIT**: nu se copiază cod din Veyon (GPL-2.0) sau RustDesk (AGPL); doar idei. Crate-urile au licențe compatibile (MIT/Apache/BSD/ISC/Zlib), verificate cu `cargo-deny`.
- **Rețelele școlilor**: NAT, IP-uri din DHCP care se schimbă, uneori fără DNS local, uneori proxy-uri; agenții nu sunt accesibili din internet.

## 7. Variante luate în calcul

### 7.1 Ordinea trecerii la Rust
| Variantă | Ce înseamnă | Decizie |
|---|---|---|
| **A. Rescriere curată** | Workspace Rust nou, protocol și model de date noi; Go rămâne neatins până la 3.0; apoi instalare de la zero | **Aleasă** — după ce proprietarul a renunțat la compatibilitate (2026-11-17) |
| B. Pe rând, întâi serverul compatibil | Server Rust care vorbește și REST-ul vechi, apoi agent, apoi consolă | Aleasă inițial pentru a nu strica școlile; abandonată odată cu cerința de compatibilitate |
| C. Întâi agentul, pe serverul Go | — | Respinsă: tot ce aduce nou cere server |

Ordinea de **construcție** rămâne cea din B (server → agent → consolă → funcții), pentru că fiecare pas are nevoie de cel dinainte (§14).

### 7.2 Regulile de aplicații
| Variantă | Decizie |
|---|---|
| A. Globale, ca în 2.x | Respinsă: un profesor ar schimba regula pentru toată școala |
| B. Pe sală (mod + liste proprii) | Acoperită de C |
| **C. Profiluri pe sală** | **Aleasă de proprietar** (2026-11-17) |

### 7.3 Alegeri de „cum" (ale mele, pot fi schimbate)
- **gRPC (`tonic`) pentru API-ul de administrare și control** (consolă, CLI, viitoarea aplicație Flutter cu `grpc-dart`): fără REST nou care ar fi fost înlocuit mai târziu; streaming de la server pentru starea live a sălilor și rezultatul comenzilor.
- **WebSocket/TLS pentru agenți**, nu gRPC: trece mai ușor prin proxy-urile și firewall-urile școlilor (HTTP/1.1 upgrade pe 443, inclusiv prin proxy-uri care strică HTTP/2/gRPC), și prin orice reverse proxy simplu. Mesajele de pe el sunt definite tot în `.proto` (codare protobuf binară), ca toate tipurile să aibă o singură sursă.
- **Fără plugin-uri dinamice** ca în Veyon: feature-urile sunt module compilate, înregistrate static.
- **Același repo**: workspace Cargo la rădăcină; codul Go (`server/`, `agent/`, `tools/server-gui`, `tools/whitelist-gui`) rămâne până la release-ul 3.0 și se șterge atunci; `mobile/` rămâne până la rescrierea Flutter, dar nu mai intră în arhiva de release. Lucrul se face pe un branch propriu (de exemplu `rust-rewrite`).

## 8. Arhitectură

```
                     internet / cloud  (sau PC-ul profesorului, §9.6)
               ┌───────────────────────────────────────┐
               │  guardian-server (Rust, librărie+bin) │  Linux: systemd · Windows: serviciu
               │  gRPC (tonic) · WebSocket /agent/ws   │  sau încorporat în guardian-console
               │  HTTPS pentru fișiere · SQLite        │
               └──────▲──────────────────▲─────────────┘
          WSS (ieșit) │                  │ gRPC/TLS
 ┌────────────────────┴─────┐   ┌────────┴──────────────────────┐   ┌──────────────┐
 │ guardian-agent           │   │ guardian-console (egui, Win)  │   │ guardian-cli │
 │ serviciu Win/Linux       │   │ PC profesor: server la distanță│   └──────────────┘
 │  └ guardian-worker       │◄──┤ SAU server încorporat          │
 │    (UI în sesiunea user) │   └───────────────────────────────┘
 └──────────────────────────┘    LAN, doar când agentul e offline:
                                 canal local criptat (Noise, chei ed25519, port 11500)
```

### 8.1 Workspace Cargo

```
Cargo.toml                 # [workspace]; lints comune (unsafe interzis unde nu e nevoie, fără unwrap/expect în cod de producție)
proto/                     # fișierele .proto: API-ul gRPC, mesajele agent↔server, canalul local
crates/
  guardian-proto/          # cod generat din proto/ (prost/tonic) + conversii; versiunea protocolului
  guardian-core/           # logica comună, fără OS și fără rețea: calculul politicii efective, decizia de
                           #   enforcement (ce proces se omoară), lista proceselor de sistem protejate,
                           #   registrul de feature-uri, chei ed25519, config, path-uri
  guardian-server/         # librărie + binar subțire: tonic, hub WebSocket, SQLite, conturi, săli,
                           #   profiluri, înrolare, comenzi, fișiere, audit
  guardian-agent/          # binar: serviciu, conexiune WS, enforcement, stare offline, listener local, worker manager
  guardian-worker/         # binar: proces în sesiunea userului (mesaj, countdown, pornire program/site)
  guardian-platform/       # trait Platform + impl windows/ și linux/
  guardian-console/        # binar egui „Guardian Server": consola profesorului (server la distanță sau încorporat)
                           #   + modul de rezervă local
  guardian-setup/          # binar egui + mod CLI: instalare/diagnostic/update/dezinstalare agent, whitelist.txt
  guardian-cli/            # binar clap: administrare și control prin gRPC
```

`guardian-proto` și `guardian-core` sunt singurul loc unde trăiesc tipurile și logica de decizie (anti-obiectivul „a doua copie").

### 8.2 Modelul de feature (din Veyon, simplificat)

- Fiecare funcție are un `FeatureId` stabil (UUID scris în cod, nu se schimbă niciodată) și un nume (`message`, `program.start`, `site.open`, `power.shutdown`, `files.push`, …).
- Un mesaj de feature (`id` unic, `feature`, `command`, argumente tipizate în `.proto`) circulă pe canalul agentului și pe canalul local. `id` servește la confirmări și deduplicare.
- În agent, fiecare feature implementează un trait (formă orientativă):

```rust
trait Feature: Send + Sync {
    fn id(&self) -> FeatureId;
    fn allowed_offline(&self) -> bool;          // poate rula în modul local de rezervă
    async fn handle(&self, ctx: &AgentCtx, msg: &FeatureMessage) -> Result<FeatureReply>;
}
```
- Ce se afișează userului (mesaj, countdown, program pornit în sesiunea lui) agentul nu face singur: îl trimite **worker-ului** (separarea Service→Worker din Veyon; pe Windows serviciul rulează în sesiunea 0 și nu poate desena pe ecranul elevului).
- La conectare agentul raportează feature-urile pe care le suportă (`capabilities`); serverul nu trimite unui agent o comandă pe care nu o știe și răspunde profesorului cu motivul.

### 8.3 Politica: profiluri pe sală (decizia proprietarului)

- **Profil** = nume + mod (`free` / `blacklist` / `whitelist`) + listă de aplicații (nume de executabil, potrivire fără diferență de majuscule pe numele extras al procesului). Le definește administratorul.
- La prima pornire serverul creează trei profiluri editabile: **„Liber"** (`free`), **„Lecție"** (`blacklist`, listă goală), **„Examen"** (`whitelist`, listă goală).
- Fiecare **sală** are un **profil activ**. Profesorul sălii îl poate schimba cu oricare dintre profilurile existente.
- **Excepție pe calculator** (opțională): un calculator anume poate avea un profil propriu care are prioritate față de profilul sălii (de exemplu calculatorul de la catedră pe „Liber"). Excepțiile le pune doar administratorul (alegerea mea: o excepție poate debloca un calculator, deci e o decizie de administrator).
- **Profil implicit al școlii**: pentru calculatoarele care nu sunt încă într-o sală. Implicit „Lecție" (alegerea mea: fail-secure, nu „Liber").
- **Politica efectivă** a unui calculator = excepția lui, altfel profilul activ al sălii, altfel profilul implicit. O calculează `guardian-core` (aceeași funcție pe server, în agent și în consolă).
- Agentul primește politica efectivă **și toate profilurile** (sunt mici), ca modul local să poată comuta între ele fără server.
- **`whitelist.txt` local** rămâne, ca supliment pe calculator în modul whitelist: îl generează `guardian-setup` la instalare din procesele care rulează atunci (drivere, utilitare ale calculatorului), editabil local. Lista proceselor de sistem protejate e în `guardian-core` și se aplică în ambele moduri.

### 8.4 Starea vs. comenzile

- **Starea** (ce se aplică continuu): politica efectivă + profilurile + versiunea stării. Se trimite întreagă la conectare și la fiecare schimbare; agentul o salvează atomic și o aplică și offline. Formatul e versionat, ca stări noi (lock-ul din backlog) să se poată adăuga fără a strica agenții existenți.
- **Comenzile one-shot**: mesaj, pornire program/site, oprire/restart (cu countdown opțional), trimitere/strângere fișiere. Au `id`, termen de expirare (o comandă de oprire veche de o oră nu se mai execută când calculatorul revine) și confirmare cu rezultat.

## 9. Serverul (`guardian-server`)

### 9.1 Bază de date
- SQLite, schemă nouă, migrații versionate (de la 1), rulate la pornire.
- Tabele (forma exactă e a builder-ului): `computers` (id alocat de server, nume, hostname, OS, versiune agent, cheie publică, sală, excepție de profil, adrese LAN, înrolat la, văzut ultima dată), `rooms` (nume, profil activ), `profiles` și `profile_apps`, `settings` (profil implicit, retenție, limite), `users` (login, hash argon2id, rol `admin`/`teacher`, activ), `user_rooms`, `sessions` (token hash-uit, expirare), `consoles` (cheie publică, revocat), `enroll_codes` (cod, tip agent/consolă, sală opțională, folosiri rămase, expirare), `commands` + rezultatul per calculator, `files`, `audit_log` (cine, ce, când, pe ce, sursa: server/local).
- **Încărcarea I/O** (2026-11-17): detectarea proceselor nu atinge baza (rămâne pe agent); sarcina țintă e mică. Reguli:
  1. **Un singur scriitor**: toate scrierile trec printr-o singură conexiune / un task dedicat; citirile merg în paralel (WAL).
  2. **Setări**: `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`.
  3. **`last_seen` în memorie**: starea online vine din conexiunea WebSocket și ping; în bază se scrie doar la conectare, la deconectare și cel mult o dată la 5 minute.
  4. **Scrieri grupate**: confirmările comenzilor și jurnalul de audit se scriu în tranzacții pe loturi.
  5. **Fișierele pe disc, nu în SQLite**: director lângă baza de date, fișiere numite după SHA-256; în bază doar metadatele.
  6. **Retenție**: comenzile încheiate, `audit_log` și fișierele expirate se șterg după un termen configurabil (implicit 90 de zile), cu `incremental_vacuum` sau echivalent.
- Pentru serverul încorporat pe PC-uri vechi (HDD, antivirus): documentăm excluderea directorului de date din scanarea în timp real, dacă apar încetiniri.

### 9.2 Prima pornire și conturi
- La prima pornire cu baza goală, serverul generează un **cod de inițializare** unic (afișat în jurnal/consola serviciului) cu care se creează primul cont `admin` (`guardian-cli init` sau consola). În modul încorporat (§9.6) consola cere direct crearea contului de administrator la prima deschidere (alegerea mea).
- Login cu utilizator + parolă → token de sesiune opac (stocat hash-uit, cu expirare), trimis în metadata gRPC.
- Roluri: **`admin`** — tot (utilizatori, săli, profiluri, excepții, înrolări, setări, toate comenzile); **`teacher`** — schimbă profilul activ și trimite comenzi doar în sălile lui. Verificarea se face **pe server** la fiecare apel.

### 9.3 API-ul gRPC
Servicii (împărțirea exactă e a builder-ului): `Auth` (login, logout, schimbare parolă), `Admin` (utilizatori, săli, alocări, profiluri și liste, excepții, coduri de înrolare, calculatoare — redenumire, mutare, revocare —, setări), `Control` (schimbare profil sală, trimitere comandă către calculatoare/sală, starea unei comenzi), `Watch` (**streaming de la server**: calculatoarele sălilor vizibile cu online/offline, politica efectivă și rezultatele comenzilor, live), `Files` (încărcare/descărcare prin streaming, cu limită de mărime). Reflection gRPC activată, ca `grpcurl` să poată fi folosit la depanare.

### 9.4 Hub-ul WebSocket al agenților
- `GET /agent/ws` (upgrade), doar peste TLS.
- Handshake: agentul trimite `hello` (id, versiune, versiune protocol, capabilities) → serverul trimite o provocare aleatoare → agentul o semnează cu cheia ed25519 → serverul verifică cu cheia publică înregistrată → serverul trimite starea completă, comenzile în așteptare nexpirate, cheile publice ale consolelor neretrase cu sălile profesorilor lor și lista calculatoarelor din aceeași sală (pentru modul local).
- *Notă 2026-11-17 (etapa 1):* lista calculatoarelor din aceeași sală nu se trimite în `Welcome`; niciun consumator nu o folosește în etapele 1–3, se adaugă când modul local (§11) are nevoie de ea.
- Ping/pong la ~20 s; agent offline după ~60 s fără trafic.
- Schimbările de stare și comenzile se împing imediat. Scalare țintă: câteva sute de agenți pe un server mic.
- Revocare: ștergerea calculatorului închide conexiunea și cheia nu mai e acceptată. Agentul revocat continuă să aplice ultima stare (fail-secure).
- Fișierele nu trec pe WebSocket: comanda conține un URL HTTPS pe același server și un token de unică folosință cu termen scurt, cu care agentul descarcă sau încarcă.

### 9.5 Rețea, TLS, deployment
- Un singur port pentru gRPC, WebSocket și HTTPS de fișiere (implicit **8443**, configurabil; alegerea mea).
- TLS nativ (`rustls`) din fișiere cert/cheie, sau în spatele unui reverse proxy (Caddy) cu suport pentru gRPC și WebSocket — ambele documentate. Fără TLS serverul pornește doar dacă e configurat explicit pentru teste în LAN.
- **Încrederea în certificat**: clienții (agent, consolă, CLI) acceptă implicit certificate validate de depozitul sistemului (server cu domeniu și CA publică). Pentru un server fără domeniu, clientul poate fixa amprenta certificatului (`SERVER_CERT_SHA256`); atunci acceptă doar acel certificat. Fără certificat configurat, serverul generează unul self-signed la prima pornire și afișează amprenta, ca administratorul s-o pună în installer.
- Linux: binar + unitate systemd cu user dedicat și restricții (`NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`), date în `/var/lib/guardian`, `install.sh`.
- Windows: același binar rulează ca serviciu Windows; regula de firewall se creează la instalare.
- Protecții: limită de mărime a mesajelor, rate limiting pe login și înrolare, comparații în timp constant.
- ACME (Let's Encrypt automat) e o extensie posibilă, nu e cerută acum.

### 9.6 Serverul încorporat în consolă (decizia proprietarului, 2026-11-17)
- `guardian-server` e o librărie, ca `guardian-console` să ruleze exact același cod în proces.
- În setările consolei: **„Conectare la server"** (adresă la distanță) sau **„Pornește serverul pe acest calculator"** — exclusive (alegerea mea); cu serverul încorporat, consola vorbește cu el pe `127.0.0.1`.
- Serverul încorporat rulează **exact cât rulează aplicația**: pornește la deschidere (dacă e activat), se oprește curat la închidere; nu e serviciu.
- Preia ce avea `server-gui` (`specs/2026-11-13-windows-server-gui-design.md`, șters la 3.0, în istoria git): date în `%ProgramData%\Guardian Server\`, certificat self-signed și cont de administrator create la prima pornire, regulă de firewall, o singură instanță, mesaj clar dacă portul e ocupat (fereastra nu arată „pornit" când serverul n-a reușit să asculte), agenții găsesc PC-ul profesorului după **numele calculatorului** (`SERVER_ADDRESS=wss://<PC-PROFESOR>:8443`), cu amprenta certificatului afișată în setări pentru installer.
- Cu aplicația închisă agenții sunt offline și aplică ultima stare. Modul de rezervă local practic nu intră în joc aici: cere consola deschisă, iar atunci serverul încorporat rulează.
- Consecințe acceptate: dacă mai mulți profesori pornesc fiecare serverul încorporat, fiecare are propriile calculatoare, săli și conturi (servere independente).

## 10. Agentul (`guardian-agent` + `guardian-worker`)

### 10.1 Aplicarea politicii
- `free`: nu omoară nimic. `blacklist`: omoară procesele al căror **nume extras** e în listă (nu potrivire pe subșir al liniei de comandă). `whitelist`: omoară tot ce nu e în listă, în `whitelist.txt` local sau în lista proceselor de sistem protejate. Lista de procese protejate se aplică în ambele moduri (agentul însuși, worker-ul și procesele critice ale OS-ului nu sunt omorâte niciodată).
- *Notă 2026-11-17 (review etapa 1), rezolvată 2026-11-18 în etapa 2 (`guardian_core::trusted::TrustedRoots`, rădăcinile fiecărui OS în `guardian-platform`):* lista proceselor protejate se potrivește azi doar după nume, deci în modul Examen un program redenumit `svchost.exe` (sau `bash` pe Linux) n-ar fi omorât. Agentul trebuie să protejeze un nume doar când calea executabilului e în directorul sistemului (`%SystemRoot%`, `/usr`, `/bin`, …) sau în directorul Guardian; `decide` din `guardian-core` primește pentru asta rădăcinile de încredere.
- **Detectarea proceselor e hibridă** (decizia proprietarului, 2026-11-17):
  - **Notificări de la sistem (calea principală)**: agentul e anunțat la pornirea fiecărui proces și aplică regula în milisecunde. Windows: ETW, providerul de procese al kernelului (de exemplu crate-ul `ferrisetw`). Linux: proc connector prin netlink (evenimente `exec`).
  - **Verificare completă de reconciliere**: la pornirea agentului, la fiecare schimbare a stării și periodic, implicit la ~10 s. Prinde evenimentele pierdute și procesele care rulau dinainte.
  - **Rezervă automată**: dacă sursa de notificări nu poate fi pornită sau cade, agentul trece singur pe verificare completă la 1 s, scrie asta în jurnal și reîncearcă periodic notificările.
  - Motorul de enforcement e construit în jurul unei surse de evenimente abstracte, ca decizia din `guardian-core` să fie aceeași pe toate căile și testabilă fără OS.
  - Listarea și omorârea se fac prin API-uri native (Toolhelp/`OpenProcess`+`TerminateProcess` pe Windows, `/proc` + semnale pe Linux); la scanare se citesc doar nume, PID și cale.
- Fișierele de stare și cheia privată au drepturi doar pentru SYSTEM/root; directorul agentului e scriibil doar de admin.

### 10.2 Conexiunea și starea offline
- Agentul se conectează la `SERVER_ADDRESS` (`wss://…`) și se reconectează cu backoff exponențial cu jitter (max ~60 s).
- Starea primită se salvează atomic (fișier temporar + rename) și se încarcă la pornire înainte de orice conectare. Fără stare salvată (prima pornire înainte de prima conectare), agentul aplică profilul implicit „Lecție" cu listă goală, adică nu omoară nimic în afara regulii, dar nici nu e marcat liber (alegerea mea; nu are de unde ști altceva).
- **Offline** = fără conexiune autentificată la server de cel puțin **60 s**; abia atunci se activează modul de rezervă local (§11).

### 10.3 Înrolarea
- La instalare se dă un **cod de înrolare** (din consolă sau CLI). Agentul generează o pereche ed25519, trimite cheia publică împreună cu codul și primește id-ul calculatorului. Serverul nu ține niciun secret al agentului.
- Două tipuri de cod: **de unică folosință** (15 minute) și **de sală** (mai multe folosiri, număr maxim + expirare, implicit 1 zi; calculatoarele intră direct în sala respectivă) — alegerea mea, pentru înrolarea unei săli întregi.
- Agentul raportează la fiecare conectare hostname, OS, versiune și adresele IP din LAN (pentru modul local).
- Reînrolare: ștergerea cheii locale (din `guardian-setup`) + un cod nou.

### 10.4 Worker-ul
- Proces separat, pornit de agent **în sesiunea userului activ** (Windows: sesiunea consolei activă, prin WTS / `CreateProcessAsUser`; Linux: sesiunea grafică activă din logind, cu mediul ei de afișare).
- Comunică cu agentul prin IPC local (named pipe pe Windows, socket unix pe Linux), cu acces restrâns la SYSTEM/root și userul sesiunii; worker-ul nu poate fi folosit de alt proces ca să comande agentul.
- Agentul repornește worker-ul dacă moare și îl mută când se schimbă sesiunea activă. Fără user logat worker-ul nu rulează; regulile le aplică agentul oricum, iar comenzile care au nevoie de worker se confirmă cu eșec și motivul „niciun user logat".
- UI simplu (dialog de mesaj, countdown); tehnologia e alegerea builder-ului (egui/winit implicit).

### 10.5 Platformă (`guardian-platform`)
Trait cu: listare/omorâre procese, sursa de notificări de procese, oprire/restart, enumerare sesiuni și pornire proces în sesiunea userului, adrese de rețea, informații despre sistem, serviciu (instalare/pornire/oprire), firewall. Implementări `windows/` (crate `windows`, `windows-service`) și `linux/` (`zbus` pentru logind, `nix`, systemd).

## 11. Modul de rezervă local

### 11.1 Când
Doar când agentul e **offline** (§10.2). Cât timp e conectat la server, refuză comenzile locale cu „sunt online, folosiți serverul", iar consola trece automat pe calea prin server.

### 11.2 Încredere și criptare
- Consola se înrolează la server cu un cod de tip „consolă" și primește o pereche ed25519 proprie. **Cheia ține de calculatorul profesorului, nu de un profesor**: pe un PC de clasă lucrează mai mulți profesori. Fiecare profesor se loghează cu contul lui — online prin server, offline față de un verificator argon2id păstrat local pentru profesorii care s-au logat deja pe acel PC cât timp era online. Comenzile locale poartă identitatea profesorului, iar agentul aplică sălile acelui profesor (din cache).
- Serverul distribuie agenților cheile publice ale consolelor neretrase împreună cu sălile profesorilor; consolelor le distribuie cheile publice ale agenților din sălile profesorilor care s-au logat pe ele.
- Canalul local: protocol criptat cu autentificare reciprocă pe aceste chei fixate (Noise IK prin crate-ul `snow`, sau echivalent; cerința: criptat, autentificat în ambele sensuri, fără certificate și fără PKI). Un elev cu laptop nu poate trimite comenzi, pentru că nu are o cheie de consolă cunoscută.

### 11.3 Descoperire
- Consola ține în cache, cât e online, calculatoarele sălilor cu ultima adresă IP din LAN.
- Dacă adresa s-a schimbat (DHCP), consola trimite un broadcast UDP de descoperire în subrețea; agenții răspund semnat. Doar consolele și agenții cunoscuți se recunosc.
- Port local implicit **11500** (TCP + UDP), configurabil; installer-ul deschide portul în firewall doar pentru rețeaua locală.

### 11.4 Ce se poate face local
Mesaj · oprire/restart (inclusiv cu countdown) · pornire program/site · **schimbarea profilului activ al sălii** între profilurile deja sincronizate, **cu excepția celor cu mod `free`** (fail-secure). Schimbarea e temporară și se anulează la revenirea serverului.
Nu: editarea profilurilor, excepții, înrolări, fișiere, conturi, săli.

### 11.5 Revenirea serverului
- La reconectare agentul trimite jurnalul acțiunilor locale (cine, ce, când) → `audit_log` cu sursa `local`.
- Apoi aplică starea de la server; profilul schimbat local se **anulează**. Comenzile one-shot deja executate rămân executate.
- Consola afișează clar „mod de rezervă" cât timp lucrează local.

## 12. Funcțiile

| Funcție | Tip | Online | Local | Note |
|---|---|---|---|---|
| Profiluri pe sală | stare | da | comutare (fără `free`) | §8.3 |
| Mesaj text | comandă | da | da | Titlu + text; worker afișează dialog. |
| Pornire program / deschidere site | comandă | da | da | În sesiunea userului prin worker; site = browserul implicit. Program: cale + argumente, sau presetări salvate pe server de administrator. |
| Trimitere fișiere | comandă | da | nu | Profesorul încarcă pe server; agentul descarcă prin HTTPS cu verificare SHA-256 în directorul destinație (implicit Desktop-ul userului, configurabil). Limită de mărime configurabilă. |
| Strângere fișiere | comandă | da | nu | Agentul încarcă pe server fișierele din directorul sursă (filtru de extensii/excluderi, opțional recursiv); profesorul le descarcă grupate pe calculator. |
| Oprire / restart | comandă | da | da | Opțional cu countdown afișat de worker (implicit 60 s). |
| Săli, conturi, control acces | infrastructură | da | cache | §9.2 |
| Chei și înrolare | infrastructură | — | — | §10.3, §11.2 |
| CLI | unealtă | — | — | `guardian-cli`: init, login, utilizatori, săli, profiluri, coduri, calculatoare, comenzi, fișiere. |
| Jurnal de audit | infrastructură | da | sincronizat la revenire | cine, ce, când, de unde |

### 12.1 Backlog (decizia proprietarului, 2026-11-17)

Nu intră în etapele de mai jos; se reiau după etapa 4, fiecare cu propriul design:

| Funcție | De reținut pentru când se reia |
|---|---|
| Lock ecran + input | Stare (fail-secure), nu comandă; worker-ul există deja. Ctrl+Alt+Del nu poate fi blocat pe Windows fără driver (aceeași problemă în Veyon); pe Linux/Wayland lock-ul e „cel mai bun efort" (X11 permite grab). |
| Login pe clasă | Cea mai grea funcție (Veyon injectează tastări în ecranul de login prin propriul server VNC, pe care noi nu-l avem); varianta tehnică se aprobă separat. |
| Logout pe clasă | WTS logoff / logind terminate-session. |
| Wake-on-LAN | Agentul trebuie să raporteze adresele MAC; serverul nu poate face broadcast în LAN-ul școlii, deci trimite pachetul printr-un agent online din aceeași subrețea; local, consola trimite direct. Cere BIOS/placă de rețea configurate. |
| Blocare înainte de pornire | Idee din `dev:specs/2026-11-09-open-source-alternatives.md` (LabSCH/OpenPath). Windows: IFEO `Debugger` pentru blacklist (toate edițiile, schimbare imediată, ocolibil prin redenumirea `.exe`); AppLocker pentru whitelist (Win10 2004+/11, Pro inclusiv după KB5024351, nu Home; politică locală pusă de agent). Linux: fanotify `FAN_OPEN_EXEC_PERM`. WDAC respins (restart, risc de a bloca sistemul). Kill-loop-ul rămâne plasă de siguranță. Risc: intrările IFEO/AppLocker supraviețuiesc agentului — dezinstalarea și profilul `free` trebuie să le curețe garantat; unele antivirusuri marchează IFEO `Debugger`. |
| Aplicația mobilă | Rescrierea Flutter pe gRPC (`grpc-dart`), cu streaming-ul din `Watch`; design separat. |
| Site-uri blocate/permise | Lista de site-uri ar intra în **profil** (mod liber/blacklist/whitelist propriu, de exemplu „Examen" = whitelist de aplicații + câteva site-uri). Metode: politicile browserelor (Chrome/Edge/Firefox `URLBlocklist`/`URLAllowlist`, opresc și DNS-over-HTTPS) + filtru DNS local în agent pentru restul programelor; firewall (WFP/nftables) doar pentru „fără internet" în afară de serverul Guardian; fișierul `hosts` respins (doar blacklist, ușor de ocolit). Serverul Guardian e mereu permis. Alternativă de evaluat: **Pi-hole** în rețeaua școlii, comandat de serverul Guardian prin API-ul lui (grupuri de clienți pe sală/profil) — fără cod de filtrare în agent, dar cere un server în LAN, identifică clienții după IP/MAC și filtrează doar la nivel de domeniu. Idei din `dev:specs/2026-11-09-open-source-alternatives.md` (LabSCH, OpenPath). |

## 13. Limitări cunoscute

- 3.0 nu are aplicație mobilă până la rescrierea Flutter.
- ETW și netlink pot pierde evenimente sub încărcare; reconcilierea la ~10 s acoperă golul, deci în cel mai rău caz o aplicație interzisă poate rula câteva secunde.
- Blocarea după nume de executabil e ocolibilă prin redenumirea fișierului (în whitelist mai puțin, pentru că numele nou nu e în listă).

## 14. Etapele de construcție (fiecare cu plan propriu în `specs/plans/`)

1. **Fundația și serverul** — workspace, `proto/`, `guardian-proto`, `guardian-core` (politica efectivă, decizia de enforcement, chei), `guardian-server` (schemă, prima pornire, conturi, săli, profiluri, înrolare, gRPC `Auth`/`Admin`/`Control`/`Watch`, hub WebSocket, TLS), `guardian-cli`. Testare cap-coadă cu un agent simulat în teste.
2. **Agentul** — `guardian-platform` (Windows + Linux), `guardian-agent` (înrolare, WebSocket, stare offline, detectare hibridă, enforcement), `guardian-worker` (schelet + IPC), instalare ca serviciu (Windows 32/64, Linux systemd). Prima utilizare reală: server + agent + CLI.
   *Făcută pe 2026-11-18 (branch `rust-rewrite`), descrisă în [`agent.md`](agent.md); testele de Windows rulează doar în CI, la primul push.*
3. **Consola și instalarea** — `guardian-console` (conectare la server, server încorporat, săli, profiluri, calculatoare, comenzi), modul de rezervă local (Noise, descoperire, comutare de profil), `guardian-setup` (după `rust:specs/2026-10-09-console-architecture-design.md`: straturi, tabel de operații folosit și de GUI și de CLI, `egui_kittest`, reguli „never panic", `cargo-deny`; adaptat la workspace-ul comun și la agentul Rust), arhiva de release și CI. **→ release 3.0**; codul Go se șterge din repo.
   *Împărțită pe 2026-11-18 în trei planuri: 3a consola (făcută pe 2026-11-18, branch `rust-rewrite`, descrisă în [`console.md`](console.md); butoanele pentru comenzi vin cu funcțiile din etapa 4), 3b modul de rezervă local (făcut pe 2026-11-18, descris în [`local-fallback.md`](local-fallback.md); local se poate schimba deocamdată doar profilul sălii), 3c `guardian-setup`, arhiva și ștergerea codului Go.*
4. **Funcțiile noi**, câte una, fiecare un release mic (3.1, 3.2…): mesaj → program/site → oprire/restart cu countdown → fișiere (trimitere, apoi strângere).

Arhiva de release 3.0 (forma exactă e a builder-ului): `server-linux/` (binar, unitate systemd, `install.sh`), `server-windows/` (binar de serviciu), `console-windows/` (`guardian-console`, `guardian-setup`), `agent-windows/` (agent + worker pe 32 și 64 de biți, installer care cere adresa serverului, amprenta opțională și codul de înrolare), `agent-linux/`. Fără APK.

## 15. Lăsat builder-ului

- Forma exactă a fișierelor `.proto`, numele serviciilor și câmpurilor, codurile de eroare.
- Crate-urile concrete (sugestii în analiza Veyon §13: `tokio`, `tonic`/`prost`, `axum`, `tokio-tungstenite`, `rustls`, `ed25519-dalek`, `snow`, `sqlx` sau `rusqlite`, `argon2`, `ferrisetw`, `windows`, `windows-service`, `zbus`, `nix`, `egui`/`eframe`, `egui_kittest`, `clap`, `tracing`), în limitele de licență din §6.
- Structura internă a modulelor, schema SQL exactă, strategia de test a fiecărei etape (logica din `guardian-core` cu teste unitare fără OS; serverul cu teste de integrare cu agent simulat).
- Cross-compilarea în CI și forma exactă a arhivei.
- Detaliile UI ale consolei, `guardian-setup` și worker-ului.

## 16. Decizii și întrebări

Decizii ale proprietarului (2026-11-17): fără ecran · server la distanță sau încorporat în consolă, cât e deschisă · agentul ține conexiunea, comenzile ajung pe loc · mod local doar ca rezervă la căderea internetului · server și agent pe Linux și Windows · consolă egui, Flutter rescris mai târziu · fără compatibilitate, arhitectură curată · profiluri pe sală · detectare hibridă a proceselor în etapa 2 · reguli de I/O pentru SQLite · backlog-ul din §12.1.

Rămase deschise, fără să blocheze etapele 1–3:
- Varianta tehnică pentru login pe clasă, când se reia din backlog.
- Ce se întâmplă cu linia 2.x (PR #2 `server-gui` în Go, release 2.3.0) până la 3.0 — decizia proprietarului; design-ul 3.0 nu depinde de ea.

## 17. Documentație de actualizat

La release-ul 3.0 se rescriu `specs/server.md`, `specs/agent.md`, `specs/api.md` (devine referința gRPC), `AGENTS.md`, `README.md` și pagina `docs/index.html` (funcții, instalare), plus secțiunea Guardian din `PROJECTS.md`. Analiza Veyon (`docs/research/2026-11-17-veyon-analysis.md`) se actualizează cu deciziile de aici (gRPC pentru API, profiluri, fără compatibilitate).
