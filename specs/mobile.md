# Mobile App Architecture Specification

## Overview

Guardian for Android (`mobile/`, Flutter) is a cabinet in your pocket: it
signs in as a person, acts in one account at a time, and manages that
account's rooms, rules and computers through the same `/api/v1` the web
cabinet uses.

It replaced a client that held a shared `ADMIN_TOKEN` typed into a settings
field — one secret for the whole deployment, no identity behind a change, and
no way to revoke one phone without revoking every phone and every other
client with it.

## File Structure

| File | Purpose |
|---|---|
| `lib/main.dart` | app shell; `_Root` shows the login screen or the five tabs depending on whether a session exists |
| `lib/theme.dart` | the design system: `GuardianColors`, `GuardianFonts`, `monoStyle`, `buildGuardianTheme()` |
| `lib/services/settings_service.dart` | the whole API client: stored state, session auth, every endpoint, `ApiException`, `ProcessEventFilter`/`ProcessEventPage` |
| `lib/screens/login_screen.dart` | sign in, or register an account |
| `lib/screens/home_screen.dart` | **Rules** — one room's applications: add, switch off, delete |
| `lib/screens/log_screen.dart` | **Log** — the room's blocking log; also `ComputerLogScreen`, one machine's log |
| `lib/screens/system_screen.dart` | **Room** — protection, mode, power, rename, delete, create |
| `lib/screens/computers_screen.dart` | **Computers** — the account's pool: room assignment, lock, rename, unenrol, the machine's log, and minting an installer token |
| `lib/screens/settings_screen.dart` | server address, account switcher, sign out, version |
| `lib/widgets/block_log.dart` | the blocking-log list with its filters, shared by both log screens (the caller supplies the loader) |
| `lib/widgets/room_selector.dart` | the room dropdown shared by the room-scoped screens |
| `lib/widgets/ui.dart` | shared pieces: `EmptyState`, `StateBadge`, `StatusDot`, `SectionLabel`, `ErrorBanner` |
| `lib/utils/format.dart` | reason labels, computer names, relative times — kept in step with the cabinet's `format.js` |
| `lib/utils/snackbar_helper.dart` | `showSnackBarMessage` with a `MessageKind` (ok / warning / error) |
| `assets/fonts/` | Inter, Space Grotesk and JetBrains Mono as static TTF (OFL); sources in its `README.md` |
| `test/` | `flutter test` — API contract tests against a mock HTTP client, a blocking-log widget test, and a login-screen widget test |

## Authentication

`POST /api/v1/auth/login` with `"client": "mobile"`. That field is what asks
for the session token in the response body: a browser deliberately receives it
only as an `HttpOnly` cookie it cannot read, and this app has no cookie jar.

The token is stored in `shared_preferences` and sent as
`Authorization: Bearer <token>`. Signing out calls `/api/v1/auth/logout`,
which deletes the session server-side, and clears local state either way — a
sign-out that failed because the session was already gone is still a
sign-out.

`SettingsService` is a `ChangeNotifier` and the single source of truth for
"is there a session". `_Root` in `main.dart` listens to it, so signing in or
out anywhere swaps the screen without anybody navigating.

## Accounts

A user owns their own account and may also be an admin or a room guest of
somebody else's, so requests carry `X-Guardian-Account` naming which one they
act in. It selects among memberships the session has already proved — the
server rejects anything else with a 404 — see `specs/api.md`.

Signing in lands in the first account `/api/v1/me` returns, which is ordered
strongest-role-first: a user's own account when they have one, the account
shared with them when they do not. Settings offers a switcher when there is
more than one. Switching accounts clears the selected room, because rooms
belong to an account.

## Rooms

Policy is per-room, so the two room-scoped screens are always about the room
named in the app bar, kept in `SettingsService.roomId`. When the stored room
no longer exists the app falls back to the first one rather than showing
nothing (`pickRoom`).

## Blocking Log

The **Log** tab shows what the selected room's policy actually closed: programs
that tried to run and were killed. Allowed programs, system processes and
anything on a whitelist never appear — the agent records only a successful
kill. A computer's own log opens from its card (tap, or ⋮ → Blocking log) and
uses `GET /api/v1/computers/{id}/process-events`, which also finds a machine in
no room.

Each row is one aggregated run, as the agent sends it: the program (mono), why
(`whitelist` reads "Not on the list", `blacklist` "Blocked by the list",
`locked` "Computer locked" in amber, `overflow` "Dropped by the agent" in red),
how many times, the machine (room log only), when the server received it, and
the agent's own first–last span. Rows arrive with the agent's next sync, so
they can be ~20 s behind.

Filters mirror the cabinet's: a period (24 h by default, 7 days, 30 days, all),
a reason, a computer (room log only), and an exact program name. Changing any
filter reloads from the first page; **Load more** follows `next_cursor`, and its
absence is the end of the feed. An answer to an older filter that arrives late
is discarded rather than shown under the new one.

## Design

The cabinet's design system (`specs/cabinet.md`), dark only:

- **Type carries who is speaking.** Space Grotesk for headings, Inter for words
  a person reads, JetBrains Mono for what a machine reported — process names,
  hostnames, agent versions, times in the log, the server address.
- **Colour carries state and nothing else.** Green is enforcing or online, amber
  is locked or needs attention (protection off, a machine in no room, the lock
  switch itself), red is destructive or failed. Everything else is greyscale.

The palette is the cabinet's CSS tokens in `GuardianColors`. The fonts are
bundled TTF, not fetched at run time — Flutter does not read the cabinet's
`woff2`, and a school network may have no route to Google. Space Grotesk has
no SemiBold static, so 600 resolves to Bold.

## Error Handling

Every call throws `ApiException` carrying the server's own message and status.
The previous client caught everything and returned `false` or an empty list,
which made an outage, an expired session and "there is nothing here"
indistinguishable — the screen said the account was empty either way. Now a
failure is shown as what it was, and `isUnauthorized`/`isForbidden` let a
caller tell "sign in again" from "you are not an admin here".

## Testing

`flutter test` needs no Android SDK and no server:

- API contract tests drive `SettingsService` against `MockClient`
  (`package:http/testing.dart`), asserting the things a screen cannot see —
  that login asks for `"client": "mobile"`, that requests carry the Bearer
  token and the account header, that a server error message survives to the
  UI, that a transport failure is an exception rather than an empty list, and
  that switching accounts clears the room.
- A widget test renders the login screen.
- `test/block_log_test.dart` drives `BlockLog` with a scripted loader: rows,
  the default 24-hour period, paging by cursor, a filter change reloading from
  the first page, and an error that is not mistaken for an empty log.

`SettingsService.clientFactory` and `resetForTest`/`signInForTest` are
`@visibleForTesting` seams that exist for these.

## Build

`mobile/build.sh` → `flutter build apk --release` → `../dist/guardian.apk`.
Requires the Android SDK; `flutter analyze` and `flutter test` do not.
