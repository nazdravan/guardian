# Guardian 3.0 API reference

The server speaks **gRPC** (tonic, HTTP/2, TLS) to consoles, the CLI and scripts, and a **WebSocket** protocol
(`/agent/ws`, protobuf frames) to agents, both on one port (default **8443**). The contract is the `.proto` files
in [`proto/guardian/v1/`](../proto/guardian/v1/); field numbers never change. gRPC reflection is on, so
`grpcurl -insecure` (or with the pinned certificate) lists every service. Behaviour, validation and limits are in
[`server.md`](server.md); the agent side is in [`agent.md`](agent.md).

Authentication: `authorization: Bearer <session token>` metadata, from `Auth.Login` or `Auth.Init`. Sessions last
7 days. Errors are gRPC status codes: `UNAUTHENTICATED` (no or expired session), `PERMISSION_DENIED` (role or
room), `INVALID_ARGUMENT` (validation, with the field in the message), `NOT_FOUND`, `ALREADY_EXISTS`,
`FAILED_PRECONDITION` (e.g. deleting a profile in use, demoting the last admin), `RESOURCE_EXHAUSTED` (rate limit).

## `Auth` (`api.proto`)

| RPC | Session | What |
|---|---|---|
| `Init(InitRequest{bootstrap_code, login, password})` → `LoginResponse` | no | the first administrator, once, with the code from the server log |
| `Login(LoginRequest{login, password})` → `LoginResponse{token, expires_at_unix, user}` | no | rate limited per IP after 10 failures a minute |
| `Logout(Empty)` | yes | ends this session |
| `ChangePassword(ChangePasswordRequest{old_password, new_password})` | yes | ends the user's other sessions |
| `WhoAmI(Empty)` → `User{id, login, role, active, room_ids}` | yes | |
| `EnrollConsole(EnrollConsoleRequest{code, public_key, name})` → `{console_id}` | no | a console code; the key is ed25519, unique |

## `Admin` (administrators only)

| RPC | What |
|---|---|
| `ListUsers`, `CreateUser{login, password, role}`, `UpdateUser{id, role?, active?, new_password?}`, `SetUserRooms{user_id, room_ids}` | accounts; teachers are given rooms |
| `ListRooms`, `CreateRoom{name}`, `RenameRoom{id, name}`, `DeleteRoom{id}` | rooms (a deleted room's computers keep running, without a room) |
| `ListProfiles`, `CreateProfile(Profile)`, `UpdateProfile(Profile)`, `DeleteProfile{id}` | profiles: name, mode (`MODE_FREE`, `MODE_BLACKLIST`, `MODE_WHITELIST`), app names |
| `ListComputers`, `UpdateComputer{id, name?, room_id?, exception_profile_id?}`, `RevokeComputer{id}` | `0` removes a room or an exception; a revoked computer's key is refused |
| `CreateEnrollCode{kind, room_id, max_uses, ttl_seconds}` → `EnrollCode` | `ENROLL_AGENT` (one computer, or a room code) or `ENROLL_CONSOLE` |
| `ListConsoles`, `RevokeConsole{id}` | consoles enrolled for local fallback mode |
| `GetSettings`, `UpdateSettings(Settings{default_profile_id, retention_days})` | school default profile, audit/command retention |

## `Control`

| RPC | Who | What |
|---|---|---|
| `SetRoomProfile{room_id, profile_id}` → `Room` | admin, or a teacher of the room | the room's active profile; pushed to its agents at once |
| `SendCommand{room_id \| computers, command, ttl_seconds}` → `{command_id}` | admin, or a teacher of every target | one-shot commands (`message`, `program_start`, `site_open`, `shutdown`, `restart`); agents report no capabilities before stage 4, so targets answer `RESULT_UNSUPPORTED` |
| `GetCommand{command_id}` → `CommandStatus` | admin, or the sender | per-computer results |

## `Watch`

`Watch(Empty)` → stream of `WatchEvent`: first a `Snapshot{rooms, computers, profiles}`, then `computer`
(online/offline/changed), `room`, `command_result`, `computer_removed`, `room_removed`. Teachers see only their
rooms; admins also see computers without a room. `Computer.public_key` is the agent's ed25519 key (for local
fallback mode). Profile edits are not streamed: reload them with `Admin.ListProfiles`.

## Agent WebSocket (`agent.proto`)

One protobuf frame per binary message (`AgentFrame` from the agent, `ServerFrame` from the server), at most 1 MiB:

1. `Enroll{code, public_key, info}` → `Enrolled{computer_id}` (first time only).
2. `Hello{computer_id, protocol_version: 1, capabilities, info}` → `Challenge{nonce}` →
   `ChallengeResponse{signature}` (ed25519 over `"guardian/v1/agent-auth" ‖ nonce ‖ computer_id`).
3. `Welcome{state, pending, consoles, users}`, then pushes: `State` (higher `version`), `TrustUpdate`, `Command`;
   the agent sends `CommandResult` and, after local fallback mode, `LocalAudit` (answered by `LocalAuditAck`).
4. Errors: `ServerError{code}` with `invalid_code`, `unauthorized`, `protocol`, `rate_limited`, `revoked`.

The local channel between consoles and agents (`local.proto`: `LocalRequest`, `LocalReply`, discovery) is in
[`local-fallback.md`](local-fallback.md).

## CLI

`guardian-cli` wraps the gRPC API for scripts (`guardian-cli --help`): `init`, `login`, `logout`, `whoami`,
`passwd`, `user`, `room`, `profile`, `computer`, `code`, `console`, `send`, `command`, `watch`, `settings`.
