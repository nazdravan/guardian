# Guardian Server for Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Windows GUI app, `GuardianServer.exe`, that runs the existing Guardian server in-process while its window is open and lets the teacher manage applications and computers from one screen.

**Architecture:** `server/` is split into an importable package `server/guardian` (all handlers, DB, models) plus a thin `server/main.go` for Linux. A new module `tools/server-gui` imports it via a `replace` directive: package `core` (OS-independent, tested anywhere) holds settings, the runner, an HTTP client and the view logic; package `main` (Windows only, `lxn/walk`) holds the window, firewall, single-instance lock. The window talks to the in-process server over HTTP on `127.0.0.1` with `ADMIN_TOKEN`, through the same handlers the phone uses.

**Tech Stack:** Go 1.25, `go-chi/chi`, `modernc.org/sqlite`, `lxn/walk` (same version as `tools/whitelist-gui`), `akavel/rsrc@v0.10.2`, `netsh advfirewall`.

**Spec:** `specs/2026-11-13-windows-server-gui-design.md`

## Global Constraints

- Linux server behaviour and HTTP API stay identical, except the `PUT /manage/computers` fix in Task 2.
- Every Go build is `CGO_ENABLED=0`; the GUI is `GOOS=windows GOARCH=386`, `-ldflags "-s -w -H windowsgui"`, so the release still builds on the Linux CI runner.
- UI text is **English**. Window title: `Guardian Server`.
- Computers are shown as `PC <identity>`; no names.
- Default port `8080`. Data dir: `%ProgramData%\Guardian Server\` holding `guardian.db` and `.env` (`PORT`, `TOKEN`, `ADMIN_TOKEN`).
- Tokens: 32 bytes from `crypto/rand`, `base64.RawURLEncoding` (43 chars).
- Online rule: last sync less than 60 s before the server's `current_time` (same as `computers_screen.dart`).
- Refresh interval: 3 s. No-agent warning: 2 min after start with no computer synced since start.
- Firewall rule name `Guardian Server (TCP <port>)`, `dir=in action=allow protocol=TCP localport=<port> remoteip=LocalSubnet profile=any`.
- Manifest `requireAdministrator` + Common Controls 6.0, like `tools/whitelist-gui/whitelist-gui.manifest`.
- No agent runs on the teacher's computer; nothing detects or excludes one.
- Project rule: update `specs/server.md`, `specs/api.md`, `CLAUDE.md`, `README.md`, `CHANGELOG.md` with the change. Commits: no attribution trailer.

## Review Focus

1. **Port 8080 already in use** (a second copy, another program): the window must show "Server stopped: port 8080 is in use", never "running". → Task 4, `TestStartReportsPortInUse`.
2. **Blocking an offline PC** must not make it look online for 60 s. → Task 2, `TestUpdateComputerKeepsLastSync`.
3. **Hand-edited `.env` with one token missing or blank**: fill only the missing value, never replace an existing token (agents and phone already use it). → Task 3, `TestLoadOrCreateFillsOnlyMissing`.
4. **Application typed with spaces or as a single letter**: the agent matches blacklist names by substring (`agent/main.go:288`), so `" chrome.exe "` silently never matches and `"a"` kills almost everything. Trim, and reject names shorter than 3 characters in the GUI. → Task 5, `TestValidateAppName`.
5. **Closing the window during a refresh**: stopping twice or calling the API after stop returns an error, no panic. → Task 4, `TestStopIsIdempotent`.

---

### Task 1: Make the server importable

**Files:**
- Create: `server/guardian/` — move `db.go`, `handlers.go`, `helpers.go`, `middleware.go`, `models.go`, `routes.go` here with `git mv`, `package guardian`
- Create: `server/guardian/server.go` — `Server` struct, `Config`, `New`, `Handler`, `Close` (from `main.go`)
- Modify: `server/main.go` — thin entry point
- Test: `server/guardian/server_test.go`, `server/main_test.go`

**Interfaces:**
- Produces:
  - `type Config struct { DBPath, ClientToken, AdminToken string }`
  - `func New(cfg Config) (*Server, error)` — opens/migrates the DB at `cfg.DBPath`
  - `func (s *Server) Handler() http.Handler` — the chi router (former `setupRoutes`)
  - `func (s *Server) Close() error`
  - `func LocalIP() string` — former `getLocalIP`
  - exported models unchanged: `Application`, `StatusResponse`, `ApplicationsResponse`, `Computer`, `ComputersResponse`
- In `main` package: `func configFromEnv() guardian.Config` — `DBPath "./guardian.db"`, tokens from `TOKEN` / `ADMIN_TOKEN` with the existing fallback `mILp9n6shk3G9SGSaS2nmP6YlLHwsP1Z`; `loadEnvFile` stays in `main`.

- [ ] **Step 1: Write the failing tests**

`server/guardian/server_test.go` (temp DB via `t.TempDir()`, `httptest.NewServer(s.Handler())`):
- `TestHealthNeedsNoToken`: `GET /health` → 200.
- `TestAdminRoutesUseAdminToken`: `GET /status` with `Bearer admin` → 200; with `Bearer client` → 401; config `{ClientToken:"client", AdminToken:"admin"}`.
- `TestSyncUsesClientToken`: `GET /client/sync` with `Bearer client` → 200, body has `"mode":"free"` (fresh DB is disabled).

`server/main_test.go`:
- `TestConfigFromEnvKeepsFallbackTokens`: with `TOKEN`/`ADMIN_TOKEN` unset, both equal `mILp9n6shk3G9SGSaS2nmP6YlLHwsP1Z`; with `TOKEN=x`, `ClientToken == "x"`.

- [ ] **Step 2: Run to verify they fail**

Run: `cd server && go test ./...` — Expected: FAIL, package `guardian` / `configFromEnv` undefined.

- [ ] **Step 3: Move the code and implement**

Middleware takes the token as a parameter (`authMiddleware(token string)`) instead of reading the environment per request; `New` wires `cfg.ClientToken` / `cfg.AdminToken`. `main.go` keeps slog setup, `.env` loading, `SERVER_ADDRESS` → listen address, graceful shutdown, now using `guardian.New(configFromEnv())` and `srv.Handler()`.

- [ ] **Step 4: Run tests, vet, and the Linux build**

Run: `cd server && gofmt -l . && go vet ./... && go test ./... && CGO_ENABLED=0 GOOS=linux go build -o /dev/null .`
Expected: no gofmt output, PASS, build succeeds.

- [ ] **Step 5: Commit** — `refactor(server): move the server into an importable guardian package`

### Task 2: Blocking a computer keeps its last sync time

**Files:**
- Modify: `server/guardian/handlers.go` (`handleUpdateComputer`)
- Test: `server/guardian/server_test.go`

- [ ] **Step 1: Write the failing test** `TestUpdateComputerKeepsLastSync`: insert a computer with `datetime` 10 minutes ago directly via SQL; `PUT /manage/computers {"identity":7,"blocked":true}`; then `GET /manage/computers` → computer 7 has `blocked == true` and the same `datetime` as inserted. Also `PUT` for unknown identity 9 creates it with `blocked` set.
- [ ] **Step 2: Run** `cd server && go test ./guardian -run TestUpdateComputerKeepsLastSync` — Expected: FAIL (datetime is now).
- [ ] **Step 3: Implement** — `INSERT INTO computers (identity, blocked) VALUES (?, ?) ON CONFLICT(identity) DO UPDATE SET blocked = excluded.blocked`.
- [ ] **Step 4: Run** `cd server && go test ./...` — Expected: PASS.
- [ ] **Step 5: Commit** — `fix(server): blocking a computer no longer resets its last sync`

### Task 3: Settings — data dir, `.env`, tokens

**Files:**
- Create: `tools/server-gui/go.mod` (`module server-gui`, `require server v0.0.0`, `replace server => ../../server`, `require github.com/lxn/walk` at the console's version)
- Create: `tools/server-gui/core/settings.go`
- Test: `tools/server-gui/core/settings_test.go`

**Interfaces:**
- Produces:
  - `type Settings struct { Port int; Token, AdminToken string }`
  - `func LoadOrCreate(dir string) (Settings, error)` — creates `dir` and `dir/.env` if missing; fills only missing/blank `PORT` (8080), `TOKEN`, `ADMIN_TOKEN`; writes back only if something was filled; keeps unknown lines and comments
  - `func NewToken() string`
  - `func (s Settings) DBPath(dir string) string` → `filepath.Join(dir, "guardian.db")`

- [ ] **Step 1: Write the failing tests**
  - `TestLoadOrCreateGeneratesOnFirstRun`: empty dir → `Port == 8080`, both tokens 43 chars, distinct; a second call returns identical values.
  - `TestLoadOrCreateFillsOnlyMissing`: `.env` with `TOKEN=keep-me` and `ADMIN_TOKEN=` → `Token == "keep-me"`, `AdminToken` generated, file now contains `TOKEN=keep-me`.
  - `TestLoadOrCreateRejectsBadPort`: `PORT=abc` → error mentioning `PORT`; `PORT=70000` → error.
- [ ] **Step 2: Run** `cd tools/server-gui && go test ./core` — Expected: FAIL, undefined.
- [ ] **Step 3: Implement** `settings.go`.
- [ ] **Step 4: Run** `cd tools/server-gui && go test ./core` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(server-gui): settings with generated tokens`

### Task 4: Runner — start and stop the embedded server

**Files:**
- Create: `tools/server-gui/core/runner.go`, `core/addrinuse_windows.go`, `core/addrinuse_other.go`
- Test: `tools/server-gui/core/runner_test.go`

**Interfaces:**
- Consumes: `guardian.New`, `Server.Handler`, `Server.Close` (Task 1); `Settings` (Task 3)
- Produces:
  - `func Start(dir string, s Settings) (*Runner, error)` — `net.Listen("tcp", ":"+port)` synchronously (port 0 allowed, for tests), then serves in a goroutine with the timeouts of `server/main.go` (15 s / 15 s / 60 s)
  - `func (r *Runner) Port() int` — actual port
  - `func (r *Runner) StartedAt() time.Time`
  - `func (r *Runner) Stop() error` — graceful shutdown (5 s), closes the DB; second call returns nil
  - `func IsAddrInUse(err error) bool` — `syscall.EADDRINUSE` on Unix, `WSAEADDRINUSE` (10048) on Windows, via `errors.Is`
  - `func StartError(err error, port int) string` — `"port 8080 is in use"` when `IsAddrInUse`, otherwise `err.Error()`

- [ ] **Step 1: Write the failing tests**
  - `TestStartServesHealth`: `Start(t.TempDir(), Settings{Port:0,...})`, `GET http://127.0.0.1:<Port()>/health` → 200.
  - `TestStartReportsPortInUse`: start one runner, start a second on its `Port()` → error, `IsAddrInUse(err)` true, `StartError(err, p) == fmt.Sprintf("port %d is in use", p)`.
  - `TestStopIsIdempotent`: `Stop()` twice → both nil; `GET /health` afterwards fails.
- [ ] **Step 2: Run** `cd tools/server-gui && go test ./core -run 'Start|Stop'` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cd tools/server-gui && go test ./core && GOOS=windows GOARCH=386 go vet ./core` — Expected: PASS, no vet errors.
- [ ] **Step 5: Commit** — `feat(server-gui): run the server in-process`

### Task 5: API client and view logic

**Files:**
- Create: `tools/server-gui/core/client.go`, `tools/server-gui/core/view.go`
- Test: `tools/server-gui/core/client_test.go`, `tools/server-gui/core/view_test.go`

**Interfaces:**
- Consumes: `Runner` (Task 4) in tests; `guardian.Application`, `guardian.StatusResponse`, `guardian.Computer`, `guardian.LocalIP` (Task 1)
- Produces (`client.go`, `type Client struct`, all methods return `error` last):
  - `func NewClient(port int, adminToken string) *Client` — base `http://127.0.0.1:<port>`, 5 s timeout
  - `Status() (guardian.StatusResponse, error)`, `SetStatus(enabled bool, mode string) error`
  - `Apps() ([]guardian.Application, error)`, `AddApp(name, mode string) error`, `SetAppEnabled(name, mode string, enabled bool) error`, `RemoveApp(name, mode string) error`
  - `Computers() ([]guardian.Computer, time.Time, error)` — the time is `current_time`
  - `SetBlocked(identity int, blocked bool) error`, `BlockAll() error` (`PUT /manage/computers/block_all`), `UnblockAll() error` (`DELETE /manage/computers/reset`)
  - non-2xx → error carrying the server's `error` field
- Produces (`view.go`, pure):
  - `func ValidateAppName(name string) (string, error)` — trims; error if shorter than 3 characters
  - `func SortApps(apps []guardian.Application)` — disabled first, then by name, case-insensitive
  - `func IsOnline(c guardian.Computer, now time.Time) bool` — parse `c.DateTime` as RFC 3339; `|now - t| < 60s`
  - `func OnlineSummary(cs []guardian.Computer, now time.Time) string` — `"9/12 online"`
  - `func LastSync(cs []guardian.Computer) (time.Time, bool)`
  - `func NoAgentWarning(startedAt time.Time, cs []guardian.Computer, now time.Time) bool` — `now - startedAt >= 2m` and no `datetime` after `startedAt`
  - `type ConnectInfo struct { AgentLine, Token, PhoneByName, PhoneByIP, AdminToken string }`, `func NewConnectInfo(hostname, ip string, s Settings) ConnectInfo` — `AgentLine = "SERVER_ADDRESS=http://<HOSTNAME>:<port>"`, phone URLs `http://<HOSTNAME>:<port>` and `http://<ip>:<port>`

- [ ] **Step 1: Write the failing tests**
  - `client_test.go` against a real `Start` runner: `TestClientAppsRoundTrip` (add `chrome.exe` blacklist → listed, enabled; disable → `Enabled == false`; remove → gone); `TestClientStatus` (`SetStatus(true,"whitelist")` → `Status()` matches); `TestClientComputers` (insert identity 7 by calling `GET /client/sync?identity=7` with the client token, then `SetBlocked(7,false)`, `Computers()` shows 7 unblocked; `BlockAll` → blocked; `UnblockAll` → unblocked); `TestClientWrongTokenErrors` (client with a wrong admin token → `Apps()` returns an error).
  - `view_test.go`: `TestValidateAppName` (`" chrome.exe "` → `"chrome.exe"`; `"a"`, `"  "` → error); `TestSortApps`; `TestIsOnline` (59 s → true, 61 s → false); `TestOnlineSummary`; `TestNoAgentWarning` (1 min, none → false; 2 min, none → true; 2 min, one synced after start → false; one synced before start only → true); `TestNewConnectInfo` (hostname `PROFESOR-PC`, ip `192.168.1.37`, port 8080 → exact strings above).
- [ ] **Step 2: Run** `cd tools/server-gui && go test ./core` — Expected: FAIL.
- [ ] **Step 3: Implement** `client.go` and `view.go`.
- [ ] **Step 4: Run** `cd tools/server-gui && go test ./core` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(server-gui): API client and view logic`

### Task 6: Windows window, firewall, single instance

**Files:**
- Create: `tools/server-gui/main.go` (startup), `tools/server-gui/window.go` (layout B), `tools/server-gui/connect.go` (Connect dialog), `tools/server-gui/firewall.go`, `tools/server-gui/instance.go`, `tools/server-gui/window_test.go`
- Create: `tools/server-gui/server-gui.manifest`, `tools/server-gui/server-gui-test.manifest` (copies of the console's, identity `Guardian.Server`), `tools/server-gui/guardian.ico` (copy of `tools/whitelist-gui/guardian.ico`)
- Create: `tools/server-gui/build.ps1`, `tools/server-gui/test.ps1` (as the console's, output `dist/server-windows/GuardianServer.exe`)
- Modify: `tools/server-gui/core/` add `func FirewallArgs(port int) []string` + test `TestFirewallArgs`

**Interfaces:**
- Consumes: everything from Tasks 3–5.
- Startup order in `main.go`: single-instance lock (named mutex `Local\GuardianServer`; if held, find the `Guardian Server` window, bring it to front, exit) → data dir `filepath.Join(os.Getenv("ProgramData"), "Guardian Server")` → `LoadOrCreate` → ensure firewall rule (`netsh advfirewall firewall show rule name=...`; if it exits non-zero, run `netsh` with `FirewallArgs`; failure is shown in the window, not fatal) → `Start` → window. A `LoadOrCreate` or `Start` error still opens the window, showing "Server stopped: <StartError>".
- Window (layout B, spec "Decided by the owner"): top bar — state label, "Protection" checkbox (`SetStatus`), mode combo Blacklist/Whitelist, buttons "Block all PCs", "Unblock all", "Connect…", "Last agent sync: N s ago" / warning when `NoAgentWarning`; left — Applications table (enabled checkbox, name, mode) with "+" (input dialog → `ValidateAppName` → `AddApp` with the current mode) and "Remove"; right — Computers table (`PC <identity>`, online/blocked/offline) with "Block"/"Unblock" for the selection, header `OnlineSummary`. Refresh every 3 s on a goroutine, UI updates via `mw.Synchronize`; an API error shows in the status line and the next tick retries. Closing the window calls `Runner.Stop()`.
- Connect dialog shows `ConnectInfo` fields with Copy buttons; `AdminToken` hidden until "Show"; the `NoAgentWarning` text points at the IP line.

- [ ] **Step 1: Write the failing tests** — `TestFirewallArgs` in `core` (port 8080 → exactly `advfirewall firewall add rule name=Guardian Server (TCP 8080) dir=in action=allow protocol=TCP localport=8080 remoteip=LocalSubnet profile=any` as separate args); `window_test.go` `TestBuildWindow` gated on `PS_GUI_TEST`, like the console's: builds the window against a runner on port 0, populates both tables, checks row counts.
- [ ] **Step 2: Run** `cd tools/server-gui && go test ./core -run FirewallArgs` — Expected: FAIL.
- [ ] **Step 3: Implement** the files above.
- [ ] **Step 4: Verify the build** — `cd tools/server-gui && go test ./core && GOOS=windows GOARCH=386 go vet ./... && go run github.com/akavel/rsrc@v0.10.2 -manifest server-gui.manifest -ico guardian.ico -arch 386 -o rsrc_windows_386.syso && CGO_ENABLED=0 GOOS=windows GOARCH=386 go build -ldflags "-s -w -H windowsgui" -o /tmp/GuardianServer.exe . ; rm -f rsrc_windows_386.syso` — Expected: PASS, exe builds, `file` reports `PE32 executable (GUI) Intel 80386`.
- [ ] **Step 5: Commit** — `feat(server-gui): Guardian Server window for Windows`

### Task 7: Release archive, CI and docs

**Files:**
- Modify: `scripts/package.sh` — build `tools/server-gui` like the console (rsrc + `-H windowsgui`) into `$OUT/server-windows/GuardianServer.exe`; copy `packaging/server-windows/README.txt`
- Create: `packaging/server-windows/README.txt` — first run and admin prompt; what to copy into `agent.env`; do not install an agent on this computer; the phone may need the IP; Linux agents resolve the name via systemd-resolved, otherwise use the IP or a DHCP reservation
- Modify: `.github/workflows/release.yml` — Windows job also runs `./tools/server-gui/test.ps1 -SkipGui`; Linux job runs `(cd server && go test ./...)` and `(cd tools/server-gui && go test ./core)` before packaging; add `tools/server-gui/go.sum` to the Go cache paths
- Modify: `CLAUDE.md` (component list, build commands, archive layout, notes: server module now has tests), `README.md` (archive tree, "Server (Windows)" section), `specs/server.md` (package layout, `PUT /manage/computers` keeps `datetime`), `specs/api.md` (same), `CHANGELOG.md` (Unreleased)
- Modify: `VERSION` → `2.3.0`

- [ ] **Step 1: Run the archive build** — `SKIP_APK=1 ./scripts/package.sh && unzip -l dist/guardian-2.3.0.zip | grep server-windows` — Expected: `GuardianServer.exe` and `README.txt` listed; `file` on the exe reports GUI 80386.
- [ ] **Step 2: Validate the workflow** — `python3 -c "import yaml;yaml.safe_load(open('.github/workflows/release.yml'))"` — Expected: no error.
- [ ] **Step 3: Commit** — `build: ship Guardian Server for Windows in the release archive`
