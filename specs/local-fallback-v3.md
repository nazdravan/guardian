# Guardian 3.0 Local fallback mode (Rust)

Status: stage 3b of [`2026-11-17-rust-rewrite-design.md`](2026-11-17-rust-rewrite-design.md) (§11), implemented on
branch `rust-rewrite` (plan: [`plans/2026-11-18-rust-3-stage3b-local-fallback.md`](plans/2026-11-18-rust-3-stage3b-local-fallback.md)).
When the internet (or the server) is down, a teacher switches a room's profile from the console directly over the
school network. Server: [`server-v3.md`](server-v3.md); agent: [`agent-v3.md`](agent-v3.md); console:
[`console-v3.md`](console-v3.md).

## When

- **Agent:** only when it has had no authenticated server connection for **60 s**
  (`guardian_core::local::OFFLINE_AFTER`). Before that, and whenever it is connected, every local request gets
  `online` ("use the server"). A revoked agent answers `revoked`.
- **Console:** only in **remote** mode (in embedded mode the server runs inside the console), only after it was
  enrolled for fallback, and only while it cannot reach the server. It leaves fallback mode by itself the moment
  the server answers again (within its reconnect backoff, at most 30 s); a sign-in that fails because the server
  is unreachable restarts that reconnect loop.

## Keys and the channel

- A console enrolls with an `ENROLL_CONSOLE` code (`Auth.EnrollConsole`); an admin can do it in one click (the
  console creates a one-use, 15-minute code itself). Every enrollment makes a fresh ed25519 key, saved in
  `console.key` next to `console.toml` (per Windows account, 0600) once the server accepted it, so enabling again
  after a revoke works; the console id and the cache live in `fallback.pb`.
- Agents get the non-revoked consoles' keys and the authorized users (active admins + the teachers of their room)
  in `Welcome` / `TrustUpdate` (stage 1). Consoles get the agents' keys in `Computer.public_key` through `Watch`.
- Channel (`crates/guardian-local`): **`Noise_IK_25519_ChaChaPoly_BLAKE2s`** (`snow`) over TCP. The X25519 keys
  are the ed25519 identities converted the standard way (`KeyPair::x25519_secret`, `PublicKey::x25519`). The
  console is the initiator (it knows the agent's key); its first payload is its console id, which must belong to
  the static key it used. Unknown consoles get nothing back. Frames: big-endian `u16` length + one Noise message
  carrying one protobuf message (`LocalRequest` / `LocalReply`, `proto/guardian/v1/local.proto`).
- Limits: the handshake **and the first request** must arrive within **5 s** (a recorded first handshake
  message replays fine but cannot be followed by a request, so it is dropped as fast as a silent peer); idle
  **30 s**; at most **16** pending handshakes (a new one evicts the oldest) and **4** per address (evicting that
  address's oldest), and at most **16** signed-in sessions.
- Users: the console checks the password (online: the server; offline: an argon2id verifier kept for users who
  signed in on this console while the server was reachable; or the user of the saved session, without a
  password). The agent then checks the claimed user (id **and** login) against its own `AuthorizedUser` list, so
  a teacher removed from the room before the outage is refused there.

## Discovery

The console keeps each computer's last LAN addresses. It first tries the address that answered last, then the
cached ones on its `fallback_port`; computers that do not answer are looked for once per round with a UDP
`DiscoverRequest` (signed by the console, `guardian/v1/discover`) sent to `255.255.255.255` and every interface's
broadcast address. Agents that trust the console answer `DiscoverReply` (signed, `guardian/v1/discover-reply`,
with their TCP port and whether they are online) to the sender; replies with another key or nonce are ignored.
Datagrams over 1 KiB are ignored; an agent answers each (console, nonce) once (the last 256 are remembered), at
most 5 replies a second to one address and 50 in all.

Port: **11500** TCP + UDP on both sides — `local_port` in `agent.toml` (`0` = off) and `fallback_port` in
`console.toml`; they must match across the school. A busy port turns the agent's listener off (logged); the agent
keeps enforcing.

## What can be done locally

Only **switching a room's profile**, to a profile the agent already has and that is not `free`. In stage 3b the
agents implement no one-shot commands, so a `command` request is answered `RESULT_UNSUPPORTED`; each stage-4
feature with `allowed_offline` adds itself.

| Refusal code | Text |
|---|---|
| `online` | This computer is connected to the server; use the server. |
| `revoked` | This computer was removed from the server. |
| `unauthorized` | This user may not control this computer. |
| `not_in_room` | This computer is not in that room. |
| `has_exception` | This computer has its own profile. |
| `unknown_profile` | This profile is not known on this computer. |
| `free_profile` | A free profile cannot be set in fallback mode. |
| `bad_request` | Malformed request. |

The console refuses a free profile before touching the network and reports "<Profile> applied on N of M
computers in <Room>." plus " K could not be reached." when some did not answer.

## The server returns

- The agent sends its journal (`LocalAudit`: who, what, when, from which console) right after `Welcome`; the
  server writes it to `audit_log` with `source = 'local'` and the action's own time and acks it
  (`LocalAuditAck`); acked entries leave `local.pb`.
- The local switch is dropped and the server state is applied again, even when its version did not change.
- A local switch survives an agent restart during the outage (fail-secure: an "Examen" set locally stays).

## Files

| | Where | What |
|---|---|---|
| Agent | data dir `local.pb` (private) | `LocalStore`: the switch, the journal (at most 1000, oldest dropped), next seq |
| Console | next to `console.toml`: `console.key`, `fallback.pb` (0600) | the key; `ConsoleCache`: server, console id, users with verifiers, rooms, computers (key, LAN addresses, exception), profiles |

`ForgetMode` deletes both console files; the console stays enrolled on the server until an admin revokes it.

## Tests

- `guardian-core` (`local.rs`): the rules; `guardian-local`: channel and discovery (garbage, timeouts, wrong keys).
- `guardian-server --test local_audit`; `guardian-agent --test local`, `--test listener`.
- `guardian-console --test fallback_cache`, `fallback_client`, `backend_fallback`, `ui_fallback`, and
  `real_fallback` (Unix, the real agent binary, ~75 s).

## Manual checks on Windows (before release)

1. Firewall: until the stage-3c installer adds it, open the port on each student PC as administrator:
   `netsh advfirewall firewall add rule name="Guardian local fallback" dir=in action=allow protocol=TCP localport=11500 remoteip=localsubnet`
   and the same with `protocol=UDP`.
2. Unplug the school's internet: after a minute the console offers fallback mode; switch a room to "Examen";
   the computers apply it; plug it back: they return to the server's profile and the audit shows the switch as
   local.
3. Two teachers on the same PC (same Windows account): each signs in locally with their own password.
4. A student PC whose IP changed (DHCP) is still found (discovery).

## Known limitations

- Only the profile switch works locally until stage 4 brings the one-shot commands.
- A revoked console is not told; agents simply refuse it.
- A lost `LocalAuditAck` resends the journal, so `audit_log` may hold duplicates; the server acks once the
  entries are queued (a crash in the next 0.5 s would lose them).
- One port for the whole school.
- The console key belongs to the Windows account that enabled fallback; a PC with one Windows account per teacher
  needs one enrollment per account.
