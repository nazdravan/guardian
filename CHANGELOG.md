# Changelog

All notable changes to this project will be documented in this file.

## [3.0.0]

Guardian 3.0 is a rewrite in Rust with a new architecture; it does not talk to 2.x agents, servers or the
Android app.

### Server
- gRPC API (`Auth`, `Admin`, `Control`, `Watch`) and a WebSocket hub that agents keep open, on one TLS port (8443)
- Accounts with roles (administrator, teacher), rooms, profiles per room (free / blacklist / whitelist), a
  per-computer exception and a school default; enrollment codes for agents and consoles; audit log
- Runs as a systemd service on Linux, a Windows service, or inside the teacher's console

### Agent
- Windows (32 and 64 bit) and Linux; process starts seen at once (ETW / proc connector) plus a full check every
  10 s; protected system programs are recognised by name **and** location
- Keeps enforcing the last profile when the server is away; a worker in the user's session
- Local fallback mode: when the server cannot be reached for a minute, the teacher's console can switch a room's
  profile directly over the school network (Noise channel on port 11500, signed discovery); the switch is
  undone and reported when the server is back

### Console (`guardian-console`, new)
- The teacher's window: connect to a server or run it on this computer; rooms, profiles, computers, users,
  enrollment codes and "Connection info…" with the exact agent install command; local fallback mode

### Setup (`guardian-setup`, new)
- Installs, checks, updates, re-enrolls and removes the Windows agent from a window or the command line;
  writes the local whitelist from the programs running at install time, opens port 11500 for the local network,
  removes Guardian 2.x when asked

### Removed
- The Go server, agent, Guardian Server for Windows and Guardian Console (2.x), the `.bat`/`.ps1` installers
  and the APK in the release archive (the Android app comes back after 3.0)

## [Unreleased] (2.x, Go)

### Guardian Server for Windows (new)
- `GuardianServer.exe`: the teacher runs the server on their own Windows PC; it runs while the window is open
- One screen: a coloured state card with a Turn protection on/off button, the rule in plain words (block the listed apps / allow only the listed apps), the current mode's applications, computers (connection, last seen, blocked/free), block or unblock all
- Computers have check boxes like applications (ticked = blocked); empty lists explain the next step; warnings appear in a banner with the fix next to them
- A red **Shut down…** button turns every blocked computer off (the `power` entry, as in the phone app), with a confirmation, a red banner while it is on, and **Allow computers to start** to end it
- The window no longer flickers on its 3-second refresh: only what changed is repainted
- Generates `TOKEN` and `ADMIN_TOKEN` on first run, opens its port in Windows Firewall for the local network, and shows what to copy into `agent.env` and the phone
- Warns when no agent has connected for 2 minutes
- Ships in `server-windows/` of the release archive

### Release archive
- The Linux server folder is now `server-linux/` (was `server/`), next to `server-windows/`

### Server
- Fixed random "Failed to update database" (`SQLITE_BUSY`) when ticking an application or a computer while agents sync: the database now waits up to 5 s for a lock and uses WAL
- The server is now the importable package `server/guardian`; the Linux binary behaves as before
- Blocking or unblocking a computer no longer resets its last sync time, so an offline PC no longer shows as online for a minute
- First automated tests (`cd server && go test ./...`)

### Guardian Console (new)
- Windows GUI (`tools/whitelist-gui`) for managing the agent on a single machine
- Install / Update agent / Uninstall, plus start, stop and restart of the ProcSentinelAgent service
- Diagnostics folding service state, executable, `.env`, `sync.json` freshness and `whitelist.txt` into one reported state
- Editor for the agent's local `whitelist.txt`, showing which running processes the agent would close
- Tail of the agent's Windows event log
- Ships as a single 32-bit `Guardian.exe` that runs on both 32- and 64-bit Windows and installs the matching agent
- Uninstall preserves `whitelist.txt` and `.env` by default, and refuses to delete anything that is not an agent folder

### Agent
- Add `guardian.exe` to the protected-process list, so the console is not closed by the agent in whitelist mode

### Release
- One release archive `guardian-<version>.zip`: server, Windows agent (with Guardian Console), Linux agent and the Android app, each with its config templates
- Linux agent package: `install.sh`, `uninstall.sh` and a `guardian-agent` systemd unit
- `scripts/package.sh` builds the archive; GitHub Actions runs it only on push to `main` and publishes a release when `VERSION` changes
- Binaries are no longer committed: `dist/` is build output, templates moved to `packaging/`
- The APK is signed with a release keystore from GitHub Secrets instead of the debug key
- Server `install.sh` no longer overwrites an existing `.env` and works when started without root

### Docs
- Add `specs/whitelist-gui.md`

## [2.1.0] - 2026-04-04

### Mobile (Guardian)
- Add shield button to Dashboard, System, and Computers app bars to toggle server active/inactive status
- Sync server enabled state across all screens via shared ValueNotifier
- Use IndexedStack for tab navigation to prevent screen rebuilds on tab switch
- Parallelize data fetches with Future.wait for faster loading
- Add dropdown menu with "Remove All" option on Dashboard
- Fix application update not sending mode (affected apps in both blacklist and whitelist)
- Sort applications: disabled first, then alphabetically by name
- Remove active/inactive status bar and switch from Dashboard
- Move snackbar notifications to bottom with 1-second duration

### Release archive
- The Linux server folder is now `server-linux/` (was `server/`), next to `server-windows/`

### Server
- No changes

### Agent
- No changes
