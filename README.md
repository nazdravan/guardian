<div align="center">

<p align="center">
  <img src="docs/assets/guardian-mark.png" alt="Guardian" width="130">
</p>

<img src="docs/assets/guardian-banner.png" alt="Guardian shield" />


**Guardian 3.0 controls which programs run on the computers of a class: profiles per room (free, blacklist,
whitelist), a teacher's console, agents on Windows and Linux that keep enforcing offline, and a local fallback
mode for when the internet is down.**

[![License](https://img.shields.io/badge/license-MIT-3b82f6?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/built%20with-Rust-B7410E?style=flat-square&logo=rust&logoColor=white)](Cargo.toml)
[![Agent](https://img.shields.io/badge/agent-Windows_%7C_Linux-0078D6?style=flat-square&logo=windows&logoColor=white)](specs/agent.md)

[**Quick Start**](#quick-start) · [How it works](#how-it-works) · [Build](#build) · [Docs](#documentation) · [Issues](../../issues)

</div>

<br />

## What it does

- **Profiles per room.** A profile is a mode plus a list of programs: *free* (nothing is closed), *blacklist*
  (the listed programs are closed) or *whitelist* (everything else is closed; system programs are always
  spared). Every room has an active profile; the teacher switches it with one click. A computer can have its own
  exception (the teacher's desk on "Liber"), and computers without a room get the school default.
- **Agents that act at once and keep going offline.** The agent sees every program start (ETW on Windows, the
  proc connector on Linux) and closes a forbidden one within a second; a full check every 10 s catches the
  rest. Without the server it keeps the last profile it got.
- **A console for the teacher.** `guardian-console` connects to a server, or runs the server itself while it is
  open. Rooms, profiles, computers, users and the enrollment codes are managed there.
- **Local fallback mode.** When the server cannot be reached, a teacher switches a room's profile directly over
  the school network (encrypted, mutually authenticated channel on port 11500). The switch is undone and
  written to the audit log when the server is back.
- **Accounts and roles.** Administrators manage everything; teachers control only their rooms.

## Quick Start

Download `guardian-<version>.zip` from the [releases](../../releases) and unpack it.

### 1. Server

- **Linux:** `cd server-linux && sudo ./install.sh` — a systemd service on port 8443 with a self-signed
  certificate. The script prints the certificate SHA-256 and a bootstrap code.
- **Windows (always on):** `server-windows/README.txt` — `guardian-server.exe service install`.
- **On the teacher's PC, only while the console is open:** skip this step and choose *Run the server on this
  computer* in the console.

Create the first administrator: in the console (*Connect to a server*, then the bootstrap code) or
`guardian-cli init --server https://SERVER:8443 --cert-sha256 SHA256 --code BOOTSTRAP --login admin` (the
password is read from standard input).

### 2. Console

Run `console-windows/guardian-console.exe`, connect, create rooms and profiles, and give teachers their rooms.
**Connection info…** creates an enrollment code and shows the exact install command for the agents.

### 3. Agents

- **Windows:** unpack `agent-windows/` on the student computer and run `guardian-setup.exe` as administrator
  (fill in the server, the certificate SHA-256 and the code), or
  `guardian-setup.exe install --server wss://SERVER:8443 --code CODE [--cert-sha256 SHA256]`.
  It finds and, if you agree, removes Guardian 2.x.
- **Linux:** `cd agent-linux && sudo ./install.sh wss://SERVER:8443 CODE [SHA256]`.

The computer appears in the console, in the room of its code, and follows the room's profile.

## How it works

```
                 guardian-server (gRPC + WebSocket, SQLite, TLS, port 8443)
              ▲ wss (agents keep one connection)     ▲ gRPC (console, CLI)
 ┌────────────┴───────────┐               ┌──────────┴──────────────┐
 │ guardian-agent          │               │ guardian-console         │
 │  └ guardian-worker      │◄──────────────┤ (or the server inside it)│
 └─────────────────────────┘  LAN, only when the agent is offline:    │
                              Noise channel, port 11500               │
```

The server pushes the effective profile to each agent the moment it changes; the agent saves it and enforces it
from boot, before any connection. Agents and consoles prove who they are with ed25519 keys created at
enrollment. The full design is in [`specs/2026-11-17-rust-rewrite-design.md`](specs/2026-11-17-rust-rewrite-design.md).

## Build

Rust 1.95 or newer:

```sh
cargo build --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Windows release binaries are built for `x86_64-pc-windows-msvc` (server, CLI, 64-bit agent) and
`i686-pc-windows-msvc` (32-bit agent, console, setup); CI builds them and `scripts/package.sh` packs the archive
(`.github/workflows/release.yml`).

## Documentation

| | |
|---|---|
| [`specs/server.md`](specs/server.md) | the server: accounts, rooms, profiles, enrollment, the agent protocol, TLS |
| [`specs/agent.md`](specs/agent.md) | the agent and worker: enforcement, offline state, install |
| [`specs/console.md`](specs/console.md) | the teacher's console |
| [`specs/local-fallback.md`](specs/local-fallback.md) | local fallback mode |
| [`specs/setup.md`](specs/setup.md) | `guardian-setup` on Windows |
| [`specs/api.md`](specs/api.md) | the gRPC API reference |

Guardian 3.0 does not work with 2.x agents, servers or the Android app; the Android app comes back after 3.0. The
2.x video walkthrough (in Romanian) is still on [YouTube](https://www.youtube.com/watch?v=CkksD9YpDVw).

## License

[MIT](LICENSE)
