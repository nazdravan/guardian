# Guardian 3.0 Server (Rust)

Status: stage 1 of [`2026-11-17-rust-rewrite-design.md`](2026-11-17-rust-rewrite-design.md), implemented on
branch `rust-rewrite` (plan: [`plans/2026-11-17-rust-3-stage1-server.md`](plans/2026-11-17-rust-3-stage1-server.md)).
No compatibility with 2.x: new protocol, new database, fresh install.

## Crates

| Crate | Role |
|---|---|
| `guardian-proto` | `proto/guardian/v1/*.proto` compiled with `protox` (no `protoc`): prost types, tonic stubs, `FILE_DESCRIPTOR_SET` for reflection |
| `guardian-core` | OS- and network-free logic: profiles and the effective-profile rule, protected process lists, the kill/allow decision, ed25519 challenge keys, enrollment codes, feature registry |
| `guardian-server` | Library (`start(ServerConfig) -> ServerHandle`) + binary `guardian-server` |
| `guardian-client` | TLS with optional certificate pin, gRPC channel, bearer-token interceptor, WebSocket connector |
| `guardian-cli` | `guardian-cli`, the admin/teacher command line |

`.proto` files are the contract. `guardian-core` is the only place policy logic lives (server, agent and
console all call it).

## Server modules (`crates/guardian-server/src`)

| Module | Purpose |
|---|---|
| `lib.rs` | `start()`, `ServerHandle` (`local_addr`, `bootstrap_code`, `cert_sha256`, `state`, `shutdown`), background tasks |
| `config.rs` | `ServerConfig` (data dir, listen, TLS mode, timeouts, auth rate), `TlsMode` |
| `db/` | `Db { read, write }`, migrations, one file per table group |
| `batch.rs` | `Batcher`: command results + audit entries written every 500 ms or 100 items, one transaction per batch |
| `auth.rs` | argon2id, session tokens, `Caller`, per-IP `RateLimiter`, login/password rules |
| `state.rs` | `AppState` and the hooks `policy_changed`, `trust_changed`, `computer_revoked`, command delivery/results/expiry |
| `hub.rs` | connected agents (`Hub`), `HubEvent` bus for `Watch`, `build_agent_state`, `build_trust` |
| `ws.rs` | `/agent/ws`: enrollment, challenge handshake, connection loop |
| `grpc/` | `Auth`, `Admin` (+ `admin_users`, `admin_computers`), `Control` (+ `commands`), `Watch` |
| `tls.rs` | certificate from files or self-signed (rcgen), SHA-256 pin |
| `net.rs` | one listener: optional TLS (ALPN `h2` + `http/1.1`), hyper-util auto (HTTP/2 for gRPC, HTTP/1.1 upgrade for WebSocket) |
| `retention.rs` | daily pruning |
| `testing.rs` | feature `testing`: `TestServer`, `SimAgent` |
| `main.rs`, `service_windows.rs` | binary, clap options, Windows service |

## Database (`migrations/0001_init.sql`)

Tables: `users`, `sessions`, `profiles`, `profile_apps`, `rooms`, `user_rooms`, `settings` (single row:
`default_profile_id`, `retention_days`, `state_version`), `computers`, `consoles`, `enroll_codes`,
`commands` (payload = encoded `CommandEnvelope`), `command_targets` (status `pending`, `delivered`, `ok`,
`failed`, `no_user`, `unsupported`, `expired`), `audit_log`. All times are Unix seconds.

IO rules: one writer connection, a 4-connection read-only pool, `journal_mode=WAL`, `synchronous=NORMAL`,
`busy_timeout=5000`, `auto_vacuum=INCREMENTAL`, `foreign_keys=ON`. `last_seen` is written on connect, on
disconnect and at most every 5 minutes; "online" comes from the hub. Final command statuses are never
overwritten.

First start seeds "Liber" (`free`), "Lecție" (`blacklist`) and "Examen" (`whitelist`); the school default
is "Lecție". With no user yet, a one-time **bootstrap code** is generated and logged.

## Accounts and roles

- `Auth.Init` (bootstrap code, once), `Login`, `Logout`, `ChangePassword` (ends other sessions), `WhoAmI`,
  `EnrollConsole`. Login errors never reveal which part was wrong; unknown logins still cost one argon2.
- Sessions: 32 random bytes (base64url), stored as SHA-256, valid 7 days. The caller and their rooms are
  re-read from the database on every call.
- Rate limit: `Init`, `Login`, `EnrollConsole` and agent enrollment, 10/min/IP.

| RPC | Who |
|---|---|
| `Admin.*` | admin |
| `Control.SetRoomProfile`, `Control.SendCommand` | admin, or a teacher for their rooms (every target must be in them) |
| `Control.GetCommand` | admin, or the teacher who sent it |
| `Watch.Watch` | any user; teachers see only their rooms, admins also see computers without a room |

Validation: logins `[a-z0-9._-]` 3–32; passwords ≥ 8; room/profile names 1–64, unique; app names
`[A-Za-z0-9 ._-]` 3–64, lowercased; a profile in use (room, exception, school default) cannot be deleted;
the last active admin cannot be demoted or disabled.

## Enrollment codes

8 characters from `ABCDEFGHJKMNPQRSTVWXYZ23456789`, compared ignoring case, spaces and `-`. Agent code
without a room: 1 use, 15 min. Room code: 1–500 uses (required), TTL 60 s–7 days (default 24 h). Console
code: 1 use, 15 min. Consumption is one atomic `UPDATE … RETURNING`. Unknown, used and expired codes all
answer `invalid or expired enrollment code`.

## Agent WebSocket protocol (`/agent/ws`)

Every binary message is one protobuf frame (`AgentFrame` / `ServerFrame`), at most 1 MiB.

1. The handshake must finish within 10 s.
2. `Enroll{code, public_key, info}`: rate limited; the key must be valid and new and the code valid
   (otherwise `ServerError{invalid_code}`); the computer is created in the code's room; reply
   `Enrolled{computer_id}`, then continue at step 3.
3. `Hello{computer_id, protocol_version, capabilities, info}`: version ≠ 1 → `protocol`; unknown id →
   `unauthorized`; otherwise `Challenge{nonce: 32 bytes}`.
4. `ChallengeResponse{signature}` over `"guardian/v1/agent-auth" ‖ nonce ‖ computer_id (u64 LE)`, verified
   strictly; failure → `unauthorized`.
5. The connection replaces any previous one for that id (the old socket is closed; the computer never
   appears offline). The server sends `Welcome{state, pending, consoles, users}` first; `pending` holds
   unexpired queued commands the agent's capabilities support (others become `unsupported`).
6. Loop: ping every 20 s, dropped after 60 s without traffic. `State` (on any policy change, with a
   higher `version`), `TrustUpdate` (consoles + authorized users: active admins and the room's teachers),
   `Command` are pushed; the agent sends `CommandResult` (`ok`, `failed`, `no_user`, `unsupported` only,
   and only for commands targeting it).
7. Revocation: `ServerError{revoked}`, the socket closes, the key is no longer accepted.

The list of peer computers in `Welcome` (spec §9.4) is not sent: nothing consumes it before stage 3.

## Commands

`SendCommand` targets a room or a list of computer ids. Arguments are validated (message title ≤ 100, text
1–2000; program path 1–1024, ≤ 32 args; site URL `http(s)://` ≤ 2048; countdown 0–3600). TTL default
600 s, max 86400 s. Per target: online with the feature → sent (`delivered`); online without it →
`unsupported`; offline → `pending`. Every 10 s unfinished targets of expired commands become `expired`; an
agent that comes back later does not get them.

## Watch

First event: `Snapshot` (visible rooms, their computers with `online` and effective profile, all
profiles); then `Computer`, `Room`, `ComputerResultEvent`, `computer_removed`, `room_removed`, filtered to
what the user may see. When the user's rooms or role change, the stream re-reads them and sends a new
`Snapshot`; a disabled user or expired session ends the stream with `UNAUTHENTICATED`. If the stream falls
behind the event bus it gets a fresh `Snapshot`.

## Network and TLS

One port (default **8443**) for gRPC and WebSocket. `TlsMode::Files` (PEM), `SelfSigned` (generated once
in `data_dir/tls/`, key `0600`, valid 10 years, for `localhost`, `127.0.0.1` and the hostname) or
`InsecurePlaintext` (tests / behind a TLS proxy only). The pin is the lowercase SHA-256 hex of the served
certificate. `guardian-client` accepts either the system roots or exactly the pinned certificate (names
and chain ignored, handshake signature still checked).

## Binary

```
guardian-server [--data-dir DIR] [--listen ADDR] [--tls-cert PEM --tls-key PEM]
                [--insecure-plaintext] [--log FILTER]
guardian-server service install|uninstall|run        # Windows only
```

Environment: `GUARDIAN_DATA_DIR`, `GUARDIAN_LISTEN`, `GUARDIAN_TLS_CERT`, `GUARDIAN_TLS_KEY`,
`GUARDIAN_INSECURE_PLAINTEXT=1`, `GUARDIAN_LOG`. Defaults: data dir `/var/lib/guardian` (Linux) or
`%ProgramData%\Guardian Server` (Windows), listen `0.0.0.0:8443`, self-signed TLS. Logs (stderr) always
contain `listening on <addr>`, `certificate sha256: <hex>` and, until the first admin exists,
`bootstrap code: <code>`. SIGINT/SIGTERM or a service stop shut it down cleanly.

- Linux: `packaging/server-linux-v3/guardian-server.service` (user `guardian`, `StateDirectory=guardian`,
  `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `CAP_NET_BIND_SERVICE`) and
  `install.sh` (idempotent; prints the pin and bootstrap code from the journal).
- Windows: `service install` registers `GuardianServer` (automatic start, LocalSystem) with the current
  options and adds the firewall rule `Guardian Server` for the port; `uninstall` removes both.

## Retention

Daily (first run one minute after start): finished commands and audit entries older than
`retention_days` (default 90), expired or used-up codes, expired sessions; then `PRAGMA
incremental_vacuum`. Commands with a `pending`/`delivered` target are kept.

## Safety properties

- Fail-secure defaults: the school default is "Lecție", never "Liber"; a revoked agent keeps its last state.
- Every authorization check runs on the server, on every call, from the database.
- Constant-time comparison for the bootstrap code; tokens are compared by SHA-256 lookup.
- Message limits: 1 MiB WebSocket, 4 MiB gRPC.
- `cargo deny` ignores RUSTSEC-2026-0459 (bundled SQLite in `libsqlite3-sys` < 0.39, pinned by sqlx 0.9):
  it needs a database file written by a less trusted party, and the server opens only its own.
