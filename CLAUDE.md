# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ProcSentinel is a process monitoring and control system with these components:
- **Server** (Go) — REST API backend using SQLite, runs as a Linux systemd service; the code is the importable package `server/guardian`
- **Guardian Server for Windows** (Go + lxn/walk) — `tools/server-gui/`: a GUI the teacher opens on their own PC; it runs the server in-process while the window is open and manages apps and computers from one screen. See `specs/2026-11-13-windows-server-gui-design.md`
- **Agent** (Go) — Client-side process monitor/killer, runs as a Windows service (also supports Linux/macOS)
- **Mobile** (Flutter/Dart) — Android management app called "Guardian"
- **Guardian Console** (Go + lxn/walk) — Windows GUI in `tools/whitelist-gui/` for installing, diagnosing and updating the agent, and editing its local `whitelist.txt`

## Build Commands

### Server
```sh
cd server && ./build.sh        # Builds binary and creates ../release/procsentinel-server.tar.gz
```

### Agent (Windows, requires PowerShell)
```powershell
./agent/build64.ps1            # 64-bit build → dist/agent/bin/agent/procsentinel-agent64.exe
./agent/build32.ps1            # 32-bit build → dist/agent/bin/agent/procsentinel-agent32.exe
```

### Mobile
```sh
cd mobile && ./build.sh        # flutter build apk → ../release/guardian.apk
```

### Guardian Console (Windows, requires PowerShell)
```powershell
./tools/whitelist-gui/build.ps1   # 32-bit universal build → dist/agent/Guardian.exe
./tools/whitelist-gui/test.ps1    # tests, including the window smoke test
```
Built as 32-bit on purpose so one binary runs on both 32- and 64-bit Windows;
it detects the OS architecture at run time and installs the matching agent.
See `specs/whitelist-gui.md`.

### Release archive
```sh
./scripts/package.sh           # builds everything → dist/guardian-<VERSION>.zip (SKIP_APK=1 to skip Flutter)
```
Needs Go, zip and Flutter. All Go builds use `CGO_ENABLED=0` and cross-compile from any OS.
GitHub Actions (`.github/workflows/release.yml`) runs this **only on push to `main`**; it
publishes a GitHub Release only when `VERSION` names a version with no tag yet.

### Flutter icon generation
```sh
cd mobile && flutter pub run flutter_launcher_icons
```

## Architecture

### Server (`server/` — `main.go` + package `server/guardian`, ~1000 lines total)
- `main.go` — Linux entry point: `.env` loading, `configFromEnv()`, listen address, graceful shutdown
- `guardian/server.go` — `Server` struct, `Config`, `New`, `Handler`, `Close`
- `guardian/routes.go` — chi router setup with CORS, route groups (unauthenticated, client auth, admin auth)
- `guardian/handlers.go` — all HTTP handler methods
- `guardian/db.go` — SQLite database init, migrations, CRUD operations, cache loading
- `guardian/models.go` — request/response structs (Application, Computer, ClientEntry, etc.)
- `guardian/middleware.go` — Bearer token auth middleware; tokens come from `Config`
- `guardian/helpers.go` — utility functions (JSON writing, `LocalIP()`)
- Tests: `cd server && go test ./...`

### Guardian Server for Windows (`tools/server-gui/`, own module, imports `server` via `replace`)
- `core/` — OS-independent, tested anywhere (`go test ./core`): `settings.go` (`%ProgramData%\Guardian Server\.env`, generated tokens), `runner.go` (starts/stops the embedded server, "port in use"), `client.go` (HTTP client on 127.0.0.1 with ADMIN_TOKEN), `view.go` (online rule, sorting, no-agent warning, Connect texts), `firewall.go` (netsh args)
- `main.go`, `window.go`, `connect.go`, `platform.go` — Windows only (walk, firewall rule, single-instance mutex)
- Build: `./tools/server-gui/build.ps1` → `dist/server-windows/GuardianServer.exe` (32-bit); tests: `./tools/server-gui/test.ps1`
- `Server` struct holds DB connection and in-memory caches (`appsCache`, `enabledCache`, `modeCache`, `clientCache`) with a `sync.RWMutex`
- Uses `go-chi/chi` router and `modernc.org/sqlite` (pure Go, no CGO required)
- Two auth tiers via Bearer tokens: `TOKEN` (client/agent endpoints) and `ADMIN_TOKEN` (management endpoints)
- SQLite tables: `applications`, `server`, `client`, `computers`

### Agent (`agent/main.go` + platform-specific files)
- Polls server via `/client/sync` (10s console mode, 20s service mode configurable via `CHECK_INTERVAL`), checks processes every 1s
- Platform-specific process listing/killing: `tasklist`/`taskkill` on Windows, `ps`/`pkill` on Unix
- Windows service support via `service_windows.go`, `shutdown_windows.go`, `main_windows.go`
- Non-Windows stubs: `main_stub.go`, `service_stub.go`
- Special commands: `force_poweroff`, `force_shutdown`

### Mobile (`mobile/lib/`)
- 4 screens: `HomeScreen` (blocked apps), `SystemScreen`, `ComputersScreen`, `SettingsScreen`
- `SettingsService` handles all HTTP calls and local persistence via `shared_preferences`

### Landing page (`docs/`)
- Static marketing landing page — single self-contained `docs/index.html`
- Styled with Tailwind via CDN (config inlined in a `<script>`), fonts from Google Fonts (Space Grotesk / Inter / JetBrains Mono)
- Dark "terminal" aesthetic with an animated "fleet console" hero (vanilla JS, respects `prefers-reduced-motion`)
- Brand assets in `docs/assets/` (also used by `README.md`); served directly via GitHub Pages from the `docs/` folder
- App screenshots in `docs/assets/screenshots/` (720px-wide JPEGs of the Android app, v2.1.0), shown in the landing's `#app` section and in `README.md`; the section's captions are i18n keys `app_*`/`shot_*` in all five languages
- Preview locally: `cd docs && python3 -m http.server 8099` → http://localhost:8099

### API Endpoints
- `/client/sync` — agent fetches apps, mode, and client entries (TOKEN auth)
- `/manage/applications` — CRUD for blocked apps (ADMIN_TOKEN auth)
- `/manage/applications/reset` — clear all applications (ADMIN_TOKEN auth)
- `/status` — get/set server enabled state and mode (ADMIN_TOKEN auth)
- `/info` — server info (ADMIN_TOKEN auth)
- `/client` — get/set client entries like power (ADMIN_TOKEN auth)
- `/manage/computers` — computer management (ADMIN_TOKEN auth)
- `/manage/computers/reset` — unblock all computers (ADMIN_TOKEN auth)
- `/manage/computers/block_all` — block all computers (ADMIN_TOKEN auth)
- `/health` — health check (unauthenticated)

## Key Dependencies

- **Server Go:** `go-chi/chi` (router), `go-chi/cors`, `modernc.org/sqlite` (pure Go SQLite)
- **Agent Go:** `golang.org/x/sys` (Windows APIs)
- **Flutter:** `http`, `shared_preferences`

## Deployment

### Packaging
`dist/` is build output only and is **not tracked by git**. The templates and install scripts live
in `packaging/`; `scripts/package.sh` copies them next to the freshly built binaries:
```
guardian-<VERSION>.zip
└── guardian-<VERSION>/
    ├── server/          # guardian-server (linux/amd64), server.env, install.sh, systemd unit, Docker files
    ├── server-windows/  # GuardianServer.exe (GUI that runs the server on the teacher's PC), README.txt
    ├── agent-windows/   # Guardian.exe (console), Install/Uninstall .bat, agent.env,
    │                    # bin/ (Install/Uninstall-Agent32/64.ps1, agent/procsentinel-agent32/64.exe)
    ├── agent-linux/     # procsentinel-agent (linux/amd64), agent.env, install.sh, uninstall.sh,
    │                    # guardian-agent.service
    └── guardian.apk     # Mobile app, signed with the release keystore from GitHub Secrets
```
`VERSION` (repo root) is the release version; it is also the APK's `versionName`, and the CI
run number its `versionCode`. APK signing secrets: `ANDROID_KEYSTORE_BASE64`,
`ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`.

Server installs to `/usr/local/bin/guardian/` as the `guardian-server` systemd service; the Linux
agent installs to `/opt/guardian-agent/` as `guardian-agent`. Both install scripts write `.env`
only on first install, so an upgrade keeps the configured tokens. Agent and server both read `.env` files for configuration (`SERVER_ADDRESS`, `TOKEN`, `ADMIN_TOKEN`).

## Rules

- **Always update specs on code changes:** After any code change, update the corresponding files in `specs/` (`server.md`, `agent.md`, `api.md`) and this `CLAUDE.md` to keep documentation in sync. This includes API changes, schema changes, config changes, file structure changes, and build output paths.

## Notes

- Automated tests exist for the server (`server/`), Guardian Server for Windows (`tools/server-gui/core`) and Guardian Console (`tools/whitelist-gui`); the agent and mobile app have none
- `tools/whitelist-gui/builtin.go` mirrors the hardcoded protected-process list in `agent/main.go` and must be kept in sync by hand
- `tools/mkico` is a separate module (it needs `golang.org/x/image` only to build the console's icon)
- Server and agent have separate `go.mod` files (modules `server` and `agent`)
- Server uses pure Go SQLite (`modernc.org/sqlite`) — CGO is NOT required for server builds
- The agent has no cgo code; `build*.ps1` set `CGO_ENABLED=1`, but `scripts/package.sh` builds it with `CGO_ENABLED=0`
