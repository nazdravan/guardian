# Alternative open source la Guardian — analiză (2026-11-09)

Întrebarea: există deja programe **open source** care fac ce face Guardian — blocare de
aplicații pe calculatoarele dintr-o clasă, cu blacklist/whitelist pe sală, administrate central
(server + agent pe Windows) — sau chiar mai mult?

**Răspuns scurt:** niciun proiect matur nu face exact asta. Proiectele mature (Veyon, Epoptes)
fac supraveghere și control al clasei, dar **nu blochează aplicații**. Proiectele care blochează
aplicații cu un server central sunt foarte noi (create în 2025–2026), au 0–9 stele și sunt
declarate de autori ca versiuni de test.

Datele despre GitHub (stele, licență, ultimul push) sunt luate prin `gh api` pe 2026-11-09.

## Ce face Guardian (baza comparației)

- Agent Go, rulează ca serviciu Windows: închide procesele după blacklist/whitelist, verifică
  o dată pe secundă și continuă să aplice ultima politică și fără rețea (`sync.json`).
- Server multi-tenant (Postgres + RLS): conturi, săli, oaspeți, token de înrolare, jurnal de
  activitate, jurnalul blocărilor.
- Cabinet web, aplicație Android, Guardian Diag pentru diagnoză la fața locului.

## Proiecte mature (dar fără blocarea aplicațiilor)

| Proiect | Licență | Stele | Ultimul push | Ce face | Blochează aplicații? |
|---|---|---|---|---|---|
| [Veyon](https://github.com/veyon/veyon) | GPL-2.0 | 2025 | 2026-10-30 | Vede ecranele, transmite ecranul profesorului, control de la distanță, blocare ecran, mesaje, distribuie/colectează fișiere, deschide site-uri, pornește programe, pornire/oprire calculatoare, LDAP/AD. Windows + Linux | **Nu.** Doar blochează ecranul întreg. Internetul se oprește doar cu add-on-ul *Internet Access Control*, care nu face parte din nucleul open source |
| [Epoptes](https://github.com/epoptes/epoptes) | GPL-3.0 | 104 | 2025-09-19 | Vede și transmite ecrane, rulează comenzi de la distanță, blochează ecranul, oprește sunetul. Doar Linux (LTSP) | **Nu** |

Ambele **fac mai mult** decât Guardian la supravegherea clasei (ecrane, control de la distanță,
fișiere, alimentare). Însă nu au nici politica de aplicații, nici serverul central: Veyon Master
și Epoptes sunt aplicații desktop pentru o singură sală, fără conturi și fără cloud.

## Proiecte care blochează aplicații (noi, nematurizate)

| Proiect | Licență | Stele | Creat → ultimul push | Arhitectură | Ce are în plus față de Guardian | Rezerve |
|---|---|---|---|---|---|---|
| [LabSCH](https://github.com/fajrisilmi12-cyber/labsch) | MIT | 5 | 2026-08 → 2026-10 | Agent Python pe Windows care cere singur comenzile de la server (pull), ca Guardian; server pe Cloudflare Workers + D1; panou web pe Pages; CLI `labschctl` | Cel mai apropiat de Guardian. Blochează pe 3 niveluri: fișierul hosts, politici de browser și **IFEO**, adică programul nici nu pornește, nu e închis după ce a pornit. Pornește/oprește camera și sunetul. Comenzi la distanță cu confirmare. Descărcări verificate cu SHA-256. Profiluri de reguli („Laborator”, „Examen”). Agentul se protejează să nu fie oprit | `v0.4.0-test10`: autorul îl numește versiune de test. Un singur token de administrator, ținut în `localStorage`. Nu are conturi, nu e multi-tenant, nu are RLS. Depinde de Cloudflare |
| [OpenPath](https://github.com/balejosg/OpenPath) | AGPL-3.0 | 9 | 2025-10 → 2026-11 | Agenți pe Windows și Linux, panou web, extensie Firefox | Whitelist de **site-uri** pe sală, administrat de profesor, aplicat prin DNS și firewall. Elevii pot cere deblocarea unui site. Pe Windows folosește **AppLocker** ca să blocheze browserele nepermise și fișierele `.exe` pornite din Downloads/Desktop/Temp | Se ocupă de site-uri, nu de lista de aplicații. Fluxul complet merge doar în Firefox. Se poate folosi împreună cu Guardian |
| [LabControl](https://github.com/OTFC-ONTU/LabControl) | MIT | 0 | 2026-09 → 2026-10 | Consolă .NET pentru profesor (Win/macOS/Linux), agent ca serviciu Windows, mTLS + gRPC în LAN, fără server și fără cloud, până la 30 de PC-uri | Mozaic de ecrane, control de la distanță, Wake-on-LAN, scripturi și fișiere trimise tuturor, instalare de pachete fără internet, **mod examen cu „programe permise”** (o whitelist doar pe durata examenului), instalare de pe stick USB | O singură sală, doar în LAN. Whitelist-ul există doar în modul examen, nu ca politică permanentă |
| [NSTU](https://github.com/Khoasoma/nstu) | MIT | 5 | 2026-08 → 2026-11 | Server C++ pentru profesor + clienți, în LAN | Examene (kiosk pe WebView2, blochează Win/Alt+Tab), capturi de ecran, chat, transmite ecranul profesorului | Autorul spune „Do not deploy … as a security control in a real school”. Nu are o politică de aplicații ca Guardian |
| [ClassMonitor](https://github.com/aamergamal318-source/ClassMonitor) | MIT | 0 | 2026-11 | Consolă C# + agent în tray | Miniaturi ale ecranelor, control de la distanță, blocare ecran | Creat acum o săptămână |
| [POps](https://github.com/adity982/POps) | Apache-2.0 | 0 | 2026-10 (o singură zi) | Agent .NET + panou PHP | Inventar, mod examen, instalare de programe prin winget, actualizări semnate | Un singur push, probabil doar schelet |
| [classroom-control](https://github.com/ziomciopoziomcio/classroom-control) | fără licență | 0 | 2026-10 (o singură zi) | FastAPI + agent Go ca serviciu Windows | Monitorizare, tichete de helpdesk | Fără licență, deci nu e open source în sens juridic |

## Parental control / blocare locală

| Proiect | Licență | Stare | Notă |
|---|---|---|---|
| [whip4brats](https://github.com/rozensoftware/whip4brats) | Apache-2.0 | 10★, abandonat din 2023-08 | Serviciu Windows (C# + Rust), control parental pentru un singur calculator |
| [ZED Guardian](https://github.com/r00tmebaby/ZED-Guardian) | MIT | 0★, un singur commit (2025-01) | Mai multe PC-uri, închide procese și servicii, dar are și **keylogger** și capturi de ecran. Nu e un model de urmat |
| [Restrict Application](https://sourceforge.net/p/restrictapp), [Application Blocker](https://sourceforge.net/projects/applicationblocker/) | SourceForge | vechi | Blocare locală cu parolă, fără server |

## Infrastructura Windows (alternativa „fără produs”)

- **AppLocker / WDAC** (incluse în Windows, se gestionează prin GPO/Intune) blochează programul
  la pornire, la nivel de sistem. E mai greu de ocolit decât un `taskkill`, dar cere domeniu sau
  MDM și nu se schimbă pe sală în câteva secunde.
- [AaronLocker](https://github.com/microsoft/AaronLocker) (MIT, 711★, de la Microsoft, oprit din
  2022): scripturi care generează politici AppLocker/WDAC.
- [Harden-Windows-Security / AppControl Manager](https://github.com/HotCakeX/Harden-Windows-Security)
  (MIT, 4842★, activ): interfață grafică pentru politicile WDAC pe un singur PC.

## Concluzii

1. **Nu există un echivalent open source matur.** Combinația lui Guardian — politică de aplicații
   pe sală + server multi-tenant cu conturi, oaspeți și jurnale + aplicație mobilă + fail-secure
   offline — nu apare în niciun proiect găsit. Cel mai aproape e LabSCH, care e la nivel de
   versiune de test și are un singur administrator.
2. **„Mai mult” fac Veyon și LabControl**, dar la supravegherea clasei (ecrane, control de la
   distanță, fișiere, alimentare, examene), nu la blocarea aplicațiilor. Funcțiile acestea sunt
   amânate în Guardian (vezi decizia din 2026-10-09: rămân doar regulile și jurnalele).
3. **Idei care merită luate:**
   - **IFEO sau AppLocker în loc de kill-loop** (LabSCH, OpenPath): programul nu pornește deloc,
     deci elevul nu apucă să-l vadă nici o secundă și nu mai rămâne fereastra între două
     verificări.
   - **Profiluri de reguli** („Lecție”, „Examen”, „Liber”) activate dintr-un click (LabSCH).
   - **Agentul se protejează de oprire** (LabSCH: scheduled task + cheia Run).
   - **Cererea de deblocare trimisă de elev** și aprobată de profesor pe loc (OpenPath).
   - **Instalare de pe stick USB** care întreabă doar numărul PC-ului (LabControl), foarte
     aproape de planul pentru installer-ul Rust.
4. **Se completează cu Guardian, nu îl înlocuiesc:** Veyon (supravegherea ecranelor) și
   OpenPath (whitelist de site-uri) pot rula alături de agentul Guardian.

## Surse

- https://veyon.io/en/features/ , https://github.com/veyon/veyon
- https://github.com/epoptes/epoptes
- https://github.com/fajrisilmi12-cyber/labsch
- https://github.com/balejosg/OpenPath
- https://github.com/OTFC-ONTU/LabControl
- https://github.com/Khoasoma/nstu
- https://github.com/aamergamal318-source/ClassMonitor
- https://github.com/adity982/POps
- https://github.com/ziomciopoziomcio/classroom-control
- https://github.com/rozensoftware/whip4brats
- https://github.com/r00tmebaby/ZED-Guardian
- https://github.com/microsoft/AaronLocker
- https://github.com/HotCakeX/Harden-Windows-Security
- https://alternativeto.net/software/lanschool/?license=free
