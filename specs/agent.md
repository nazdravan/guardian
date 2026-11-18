# Guardian 3.0 Agent (Rust)

Status: stage 2 of [`2026-11-17-rust-rewrite-design.md`](2026-11-17-rust-rewrite-design.md) (§10), implemented on
branch `rust-rewrite` (plan: [`plans/2026-11-18-rust-3-stage2-agent.md`](plans/2026-11-18-rust-3-stage2-agent.md)).
It talks to the stage 1 server described in [`server.md`](server.md). No compatibility with the 2.x Go agent.

## Crates

| Crate | Role |
|---|---|
| `guardian-platform` | Everything the agent needs from the OS behind the `Platform` trait: processes, kill, process-start events, trusted roots, system info, active user session, spawning in that session; local IPC (`ipc`). Feature `testing` = `FakePlatform` |
| `guardian-agent` | Library (enroll, connection, engine, worker manager, files) + binary `guardian-agent` |
| `guardian-worker` | Library `run(endpoint, connect_timeout)` + binary `guardian-worker`, started in the user session. Stage 2: hello and ping/pong only |
| `guardian-core` | Unchanged role: the kill/allow decision (`enforce::decide`) and `TrustedRoots` live here |

`guardian-platform` modules: `linux/` (`procs.rs` — `/proc`; `events.rs` — proc connector via `netlink-sys`;
`session.rs` — `loginctl` + spawn as the user), `windows/` (`procs.rs` + `ffi_procs.rs` — ToolHelp, verified kill
on one handle; `events.rs` — ETW via `ferrisetw`; `session.rs` + `ffi_session.rs` — WTS + `CreateProcessAsUserW`;
`ffi_pipe.rs` — pipe client PID), `unsupported.rs` (macOS dev), `fake.rs`, `ipc.rs`. `unsafe` exists only in
`windows/ffi_*.rs`; Linux has none.

`guardian-agent` modules: `config.rs` (`AgentConfig`, `AgentPaths`), `store.rs` (atomic writes, key, state,
`SeenIds`, local whitelist), `engine.rs` (enforcement), `conn.rs` (enroll + WebSocket with backoff), `agent.rs`
(`run`, `AgentHandle`, state/command rules), `worker.rs` (`WorkerManager`), `local.rs` (local fallback rules,
switch and journal), `listener.rs` (the local fallback listener), `main.rs`, `service_windows.rs`. Local fallback
mode is described in [`local-fallback.md`](local-fallback.md).

## Files

| | Linux | Windows |
|---|---|---|
| Binaries | `/opt/guardian-agent/` | `%ProgramFiles%\Guardian\` (64-bit agent on 64-bit Windows) |
| Config `agent.toml` | `/etc/guardian-agent/` (0600, root) | `%ProgramData%\Guardian Agent\` |
| Data dir | `/var/lib/guardian-agent/` (0700, root) | `%ProgramData%\Guardian Agent\` (ACL: SYSTEM + Administrators) |
| Worker IPC | `/run/guardian-agent/worker.sock` (dir 0711, socket 0666) | `\\.\pipe\guardian-agent-worker` |

`agent.toml`: `server` (`wss://host:port`), `cert_sha256` (optional pin), `computer_id` (set by `enroll`),
`local_port` (optional: local fallback TCP + UDP port, default 11500, `0` = off; kept by a re-enrollment).
Data dir: `agent.key` (ed25519 seed, 32 bytes, private), `state.pb` (`StoredState`: last `AgentState`, trusted
consoles and users, save time, computer id), `seen.txt` (last 1000 command ids), `whitelist.txt` (local, one name
per line, `#` comments), `local.pb` (`LocalStore`: the local profile switch and the journal of local actions,
private), `agent.log` (Windows service only). Every state write is temp file + `sync_all` + `rename`.

## Startup and state rules

1. Load the config (no `computer_id` → `NotEnrolled`), the key, and `state.pb`. Without a saved state the default
   applies: profile `Lecție`, blacklist, empty list, id 0 — kills nothing, is not `free`.
2. The engine starts enforcing that state **before** any connection attempt (the Windows service reports `Running`
   at this point).
3. The connection starts (backoff 1 s → 60 s with ±20 % jitter, back to 1 s after an authenticated connection).
4. A received `AgentState` is applied only if its `version` is strictly greater than the current one (or the
   current one is the default, version 0); it is then saved and the engine rescans at once.
5. `Welcome` and trust updates replace the saved consoles/users.
6. Commands are deduplicated by id (`SeenIds`, 1000, saved). Stage 2 announces `capabilities = []`, so every new
   command id gets exactly one `CommandResult{RESULT_UNSUPPORTED}`.
7. `revoked` from the server: the connection stops, the last policy keeps being enforced.
8. A saved state whose `computer_id` is not this enrollment's is ignored (another server, or a re-enrollment),
   and a successful `enroll` deletes `state.pb`, `seen.txt` and `local.pb`.
9. Offline = no authenticated connection for 60 s; only then are local fallback requests accepted.
10. A local profile switch (`local.pb`) applies on top of the server state, also after a restart, until the
    server answers again: then the journal is sent (`LocalAudit`), the switch is dropped and the server state
    is re-applied even when its version did not change. Journal entries are deleted on `LocalAuditAck`.

## Enforcement (`engine.rs`)

- **Events mode**: Linux proc connector (exec events; needs root/`CAP_NET_ADMIN` on older kernels and the host
  network namespace — inside a container without `--network host` it is refused), Windows ETW
  (`Microsoft-Windows-Kernel-Process`, keyword `0x10`, event 1, session `GuardianAgentProcessTrace`; needs admin).
  Each start is looked up and judged at once.
- **Reconciliation**: full scan at start, on every new state and every 10 s.
- **Polling fallback**: if the event source cannot start or dies, a full scan every 1 s; the source is retried
  every 60 s; each switch logs one `warn!`. Lost events (`ENOBUFS`) trigger a full scan.
- Kill: re-read name, executable and start marker for the PID and kill only if unchanged (Linux `SIGKILL` after the
  check; Windows checks and terminates on the same handle) — no PID-reuse kills.
- Never killed: the agent itself, the current worker PID, protected processes (`guardian-core::protected`).
- A protected name counts only for an executable under the OS's **trusted roots** (`TrustedRoots`), so renaming a
  game to `svchost.exe` in Downloads does not protect it:
  - Linux: `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/opt/guardian-agent`. The path counts only if
    `/proc/<pid>/exe` is the same file (device + inode) as that path in the agent's own mount namespace — a
    process in its own namespace can make the link text say `/usr/bin/bash` for any file; otherwise the trust
    path is the untrusted `/proc/<pid>/exe` (the name still comes from the link text). A deleted executable
    (after a package upgrade) counts if it has no links left and sits on the reported directory's filesystem.
    Kernel threads (`PF_KTHREAD` in `/proc/<pid>/stat`) are never reported; a user process whose exe cannot be
    read is protected only by name, so an agent without `CAP_SYS_PTRACE` still enforces Exam (it just loses
    the trusted-path check). Lab machines should also disable unprivileged user namespaces.
  - Windows (`trusted_roots_from`): recursively `%SystemRoot%\System32`, `SysWOW64`, `SystemApps`,
    `ImmersiveControlPanel`, `%ProgramFiles%\WindowsApps`, `PowerShell`, `Windows Defender`, `Guardian`,
    `%ProgramData%\Microsoft\Windows Defender\Platform`; files directly in `%SystemRoot%`; excluded (users can
    write there): `System32\spool`, `System32\Tasks`, `System32\com\dmp`, `System32\Microsoft\Crypto`,
    `System32\Tasks_Migrated`, `System32\FxsTmp`, `SysWOW64\Tasks`, `SysWOW64\Tasks_Migrated`,
    `SysWOW64\com\dmp`, `SysWOW64\FxsTmp`, `Temp`, `Tracing`. Case-insensitive, `/` = `\`, `..` never trusted. A
    process that cannot be opened is protected only by name.
- Process names come from the executable's file name (Linux `comm` is cut to 15 bytes).
- On an OS without process control (macOS dev) the engine logs
  `process control is not supported on this OS` once and idles; the connection still works.

## Worker (`worker.rs`, `guardian-worker`)

- The agent listens on the IPC endpoint and checks the active session every 2 s:
  - Linux: `loginctl`, first session with `Active=yes`, `Remote=no`, `Class=user`, `Type` x11/wayland. The
    worker is spawned with the user's uid/gid, a clean environment (`XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`,
    `DISPLAY`/`WAYLAND_DISPLAY`, `GUARDIAN_SESSION`, `USER`), cwd `/`, stdio to `/dev/null`.
  - Windows: `WTSGetActiveConsoleSessionId` + user name; `WTSQueryUserToken` → primary token → environment block
    → `CreateProcessAsUserW` on `winsta0\default`, no window.
- Only the PID returned by the spawn is accepted (peer PID from `SO_PEERCRED` / `GetNamedPipeClientProcessId`);
  any other connection is closed with a `warn!` (spec §10.4). The first frame must be `WorkerHello` within 5 s.
- Ping every 5 s; no pong for 10 s → the connection closes and a new worker starts at the next check. A started
  worker that never connects within 15 s is replaced. No session / logout → the connection closes (the worker
  exits); a different session → restart in the new one.
- Windows pipe instances carry an explicit security descriptor
  (`D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12019b;;;IU)`: no network logons, interactive users read/write
  without creating pipe instances). The PID check is the real gate; anything the worker sends stays untrusted,
  since the worker runs as the student.
- Frames: big-endian u32 length + protobuf (`proto/guardian/v1/worker.proto`), at most 64 KiB.
- The worker runs only if `guardian-worker` sits next to the agent binary. Its exit codes: 0 when the agent closes
  the connection, 2 when it cannot reach the agent within `--connect-timeout` (10 s).

## Binary

```
guardian-agent [--config PATH] [--data-dir DIR] [--log FILTER] <command>
  enroll --server wss://host:port --code CODE [--cert-sha256 HEX]   # exit 1 if already enrolled
  run                                                               # foreground until SIGINT/SIGTERM
  status                                                            # saved config/state, local switch, no IPC
  service install|uninstall|run                                     # Windows only
```

Every option also reads `GUARDIAN_AGENT_*` (`CONFIG`, `DATA_DIR`, `LOG`, `SERVER`, `CODE`, `CERT_SHA256`).
To re-enroll, delete `agent.key` and the config.

## Install

**Linux** (`packaging/agent-linux/`): `sudo ./install.sh wss://server:8443 CODE [CERT_SHA256]` copies
`guardian-agent` and `guardian-worker` to `/opt/guardian-agent/`, enrolls only if the config has no
`computer_id`, installs `guardian-agent.service` (root, `Restart=always`, `ProtectSystem=full`,
`NoNewPrivileges`, State/Configuration directories 0700, Runtime directory 0711, bounding set `CAP_KILL`
`CAP_NET_ADMIN` `CAP_SETUID` `CAP_SETGID` `CAP_SYS_PTRACE` `CAP_DAC_READ_SEARCH`). No `ProtectHome`/`PrivateTmp`:
the worker inherits the unit's mount namespace and needs the real home and `/tmp`. `uninstall.sh [--purge]`
keeps the enrollment and state unless `--purge`.

**Windows:** `guardian-setup.exe` does it (see [`setup.md`](setup.md)): it copies the agent and worker for the
OS's architecture to `C:\Program Files\Guardian`, runs `guardian-agent enroll …`, writes `whitelist.txt`, opens
port 11500 for the local network and runs `guardian-agent service install` — service `GuardianAgent`
("Guardian Agent"), automatic start, LocalSystem, recovery: restart after 5 s three times (counter reset after
a day). Both `enroll` (before the key is written) and `service install` give the data and config directories
owner Administrators and the protected ACL (a student may have pre-created the folder in `%ProgramData%`);
`service install` refuses an unenrolled computer.

## Tests

- macOS/any: `cargo test --workspace --all-features` (fake platform, `TestServer`; the Unix worker and binary tests).
- Linux container (`podman`, image with clippy): the same, plus `tests/linux.rs`; with
  `--cap-add NET_ADMIN --network host --pid host` and `-- --include-ignored`, the real proc connector test.
- Real run: `cargo build --workspace --bins && cargo test -p guardian-agent --test real_linux` — real server, CLI
  and agent, a copy of `sleep` named `guardian-test-game` killed, kept alive under `Liber`, killed again with the
  server down. Passes as root, with the host network and as a non-root user (polling fallback).
- Windows: `tests/windows.rs` and the ETW test run only in CI (`.github/workflows/rust.yml`, job `windows`, which
  also builds the 32/64-bit `.exe` as the `agent-windows` artifact). Until the branch is pushed they have only
  been compiled and linted (`x86_64`/`i686-pc-windows-gnu`).

## Manual checks on Windows (before release)

1. Install the service with the 64-bit agent on 64-bit Windows and the 32-bit one on 32-bit Windows; both start
   and come online in the console/CLI.
2. `agent.log` shows events mode (ETW active), not the polling warning.
3. After a user logs in, `guardian-worker.exe` runs in that user's session (Task Manager → user column); after
   switching users it moves to the new session; after logout it is gone.
4. A blacklisted program is closed within a second.
5. In the exam profile, a program copied to Downloads and renamed `svchost.exe` is killed.
6. Reboot with the network unplugged: the saved state is enforced from boot (blacklisted program still closed).
