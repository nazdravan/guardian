# Guardian 3.0 Console (Rust, egui)

Status: stage 3a of [`2026-11-17-rust-rewrite-design.md`](2026-11-17-rust-rewrite-design.md) (§9.6, §14 point 3),
implemented on branch `rust-rewrite` (plan: [`plans/2026-11-18-rust-3-stage3a-console.md`](plans/2026-11-18-rust-3-stage3a-console.md)).
It replaces `tools/server-gui` (Go). It talks to the server of [`server-v3.md`](server-v3.md); agents are
described in [`agent-v3.md`](agent-v3.md). The UI is in English.

## Layers (`crates/guardian-console`)

| Module | What | Depends on egui |
|---|---|---|
| `settings` | `console.toml`, `Mode` (remote / embedded), `Secret`, file locations, URL and pin validation | no |
| `fleet` | `Fleet`: rooms, computers, profiles built from `Watch` events; `RoomView`, `ComputerView`, `connection_text` | no |
| `connect_info` | `AgentTarget`, the Windows/Linux enrollment commands, LAN fallback addresses, the "no agent yet" rule | no |
| `embedded` | `start_embedded` (the `guardian-server` library in process), `EmbeddedState`, data-dir ACL | no |
| `instance` | single instance: `console.lock` taken with `File::try_lock` | no |
| `firewall` | the Windows Firewall rule (`netsh`, elevated through `Start-Process -Verb RunAs` when needed) | no |
| `backend` | a thread with its own tokio runtime: `Action` in, `Event` out; connection, login, `Watch`, admin calls | no |
| `ui` | `ConsoleApp<B: Backend>`: draws only from events, sends only actions | yes |
| `logging` | `console.log` rotation (1 MiB, two old files), panic hook | no |
| `testing` (feature) | `FakeBackend` for UI tests | no |

Every `Action` runs under `catch_unwind`: a panic becomes `Event::Error("Internal error in …")` and the window
stays up. No `unsafe` in the crate.

## Files

| | Windows | Other OS (development) |
|---|---|---|
| Settings `console.toml` (0600) | `%APPDATA%\Guardian\console.toml` | `<config dir>/guardian/console.toml` |
| Lock `console.lock` | next to the settings | next to the settings |
| Embedded server data | `%ProgramData%\Guardian Server\` (ACL: SYSTEM, Administrators, the creating user) | `<data dir>/guardian-server/` (0700) |
| Log | `%LOCALAPPDATA%\Guardian\console.log` | `<cache dir>/guardian/console.log` |

`console.toml` keys: `mode` (`remote` | `embedded`), `server` and `cert_sha256` (remote), `port` (embedded,
default 8443), `login` (last login), `token` (the session; removed on sign-out and when the server ends the
session). The token never reaches the log or the screen.

## Modes

- **Remote:** `https://host:port` (port 8443 added when missing; `http://` is refused) and an optional
  certificate SHA-256 (64 hex digits, `:` and spaces ignored). A brand-new remote server is initialised from
  `guardian-cli init`.
- **Embedded:** the console starts `guardian-server` on `0.0.0.0:<port>` with a self-signed certificate and
  talks to it on `https://127.0.0.1:<port>` with that certificate's pin. On the first start the console asks for
  the first administrator itself (it holds the bootstrap code). The server stops when the window closes; agents
  then keep their last state. If the port is taken the card says so in words and never shows "running".
  On Windows, when the firewall rule `Guardian Console server` (TCP inbound, any profile,
  `remoteip=localsubnet`) is missing, a banner offers **Open firewall port…**.
- Both modes reconnect with backoff 1 s → 30 s and keep showing the last state with "Reconnecting… (attempt N)".
  `UNAUTHENTICATED` (expired session, disabled user) sends the user back to sign-in with the reason.

## Screens

```
┌ Guardian Console ─────────────────────────────────────────────────────────┐
│ ▇ Server running on this computer · port 8443            (green/amber/red)│
│ ▁ No computer has connected yet… [Connection info…]           (banners)   │
│ Overview  Rooms  Profiles  Computers  Users  Settings     (admin only)    │
│ ┌ Sala 1   12/14 online        Profile [Lecție ▾] ┐                        │
│ │ pc-01   ● Online             Lecție             │                        │
│ │ pc-02   ● Offline · 5 min ago Lecție             │                        │
│ └──────────────────────────────────────────────────┘                        │
│ Signed in as prof (teacher) [Change password…] [Sign out] [Connection info…]│
└───────────────────────────────────────────────────────────────────────────┘
```

- **Setup** (no mode yet): **Connect to a server** or **Run the server on this computer**.
- **Sign in** / **Create the administrator account** (embedded first run; passwords of 8+ characters, typed
  twice).
- **Overview** (everyone): rooms by name with "n/m online", the active profile switch, computers (online first).
  Admins also see "Not in a room".
- **Admin tabs:** Rooms (add, rename, delete) · Profiles (mode in words, application list, new/delete) ·
  Computers (room, exception, rename, revoke) · Users (add, role, rooms, disable/enable, reset password) ·
  Settings (default profile, retention days; the console's mode, port, fingerprint, data dir; **Change mode…**).
- **Connection info…:** a room code (50 uses, 1 day) or a single-computer code (15 min), then the exact
  commands with **Copy**:
  - Windows: `guardian-agent.exe enroll --server wss://<host>:<port> [--cert-sha256 <pin>] --code <code>` and
    `guardian-agent.exe service install`;
  - Linux: `sudo ./install.sh wss://<host>:<port> <code> [<pin>]`;
  - embedded: the PC's private IPv4 addresses as a fallback when agents cannot resolve its name.

## Roles and confirmations

A teacher sees only their rooms (the server filters `Watch`) and only the profile switch. Every refused call
shows the server's answer ("You are not allowed to do that."). Confirmations, all with **No** as the safe
answer: switching a room to a `free` profile ("All N computers in <room> will run anything. Continue?"),
deleting a room or profile, revoking a computer, disabling a user, changing the console's mode.

## Known limitations

- `Watch` does not send profile edits: another admin's profile changes appear when the Profiles tab is opened
  again (the console reloads profiles on opening it and after its own edits).
- One-shot commands (message, shutdown, …) have no buttons yet: agents report no capabilities until stage 4,
  and each feature brings its own button then.
- The window was opened on macOS only (wgpu renderer, embedded server running); it has not been seen on
  Windows.
- The `●` glyph is not in egui's bundled fonts, so the online dot is drawn as a shape.

## Tests

`cargo test -p guardian-console --features testing`:
- `settings`, `fleet`, `connect_info`, `logging`, `binary` — pure;
- `backend_conn`, `backend_actions`, `embedded` — `RealBackend` against `TestServer` / the embedded server;
- `ui_main`, `ui_admin` — `egui_kittest` with `FakeBackend`, headless. kittest also fails on missing glyphs;
- `real_embedded` (Unix) — the embedded server and the real `guardian-agent` (enroll, run); on Linux a copy of
  `sleep` named `guardian-test-game` is killed under "Lecție" and lives under "Liber". Needs
  `cargo build --workspace --bins`.

CI: the `linux` job runs them all; the `windows` job runs the crate's tests and builds
`guardian-console.exe` for `i686-pc-windows-msvc` (artifact `console-windows`).

## Manual checks on Windows (before release)

1. As a user **without** administrator rights: remote mode connects, signs in, switches a room profile.
2. Embedded mode: the firewall banner appears; **Open firewall port…** shows the UAC prompt; afterwards
   `netsh advfirewall firewall show rule name="Guardian Console server"` shows `remoteip=LocalSubnet`.
3. An agent on another PC enrolls with the command from Connection info and appears online.
4. Closing the window stops the server; the agent keeps its last profile.
5. A second start shows "Guardian Console is already running."
6. "Lecție" and other diacritics display correctly.
7. On a PC without a GPU driver the window still opens (wgpu WARP or the glow fallback); otherwise the error
   dialog names the log file.
