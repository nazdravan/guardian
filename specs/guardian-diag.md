# Guardian Diag

Design and rationale: `specs/2026-10-10-guardian-diag-design.md` (Russian, requirements R1-R33 and the 2026-10-15 revision). This file is the as-built description.

## Purpose

`tools/guardian-diag/` is a portable Windows app (Rust + egui, one `Guardian.exe`, no installer, no DLLs, static CRT, `i686-pc-windows-msvc`, runs on 32- and 64-bit Windows 10+). Run from a USB stick on a machine with the ProcSentinel agent, it answers "is protection working, and if not, why?", and lets an administrator edit the agent's `.env` and `whitelist.txt`. It never talks to the server and never kills processes. The manifest is `asInvoker`; without administrator rights everything readable is shown, unreadable fields say "no access", and Configuration is read-only with a "Run as administrator to edit" banner and a "Restart as administrator" button (relaunch via UAC on the same tab, `--tab`).

## Tabs

- **Diagnostics** (first) - one-line verdict (Protection is on / stale policy / Not enrolled / Service stopped / Not installed / Free mode) and a list of checks, each with status, fact and "what to do". "Not installed" is also the verdict for a missing service with the folder still present, and for a running service whose exe is missing. Refresh re-reads everything; there is no auto-refresh.
- **Configuration** - `.env` settings (`SERVER_ADDRESS`, `CHECK_INTERVAL`, `BINDING_TOKEN`) and the local whitelist (Add, Add from running, Remove). Edits are a draft with Save / Revert. Secrets are never shown: only present/absent and the last 4 hex chars of the SHA-256.
- **Processes** - `tasklist` joined with the saved policy, computed like the agent's `enforce`: Protected by the system / Allowed (whitelist.txt) / Allowed (policy) / Will be closed / Not affected; "Why is ... closed?" box; filters. In `blacklist` mode a process is closed only when a rule equals its name (case-insensitive): the agent runs `taskkill /F /IM <rule>` after `strings.Contains(tasklistText, rule)`. A substring-only match makes the agent retry `<rule>` every second ("Failed to kill process ...") and does not touch the other process; a rule with `*` never fires. These cases appear as notes.
- **Log** - `ProcSentinelAgent` records from the Application Event Log (`wevtutil qe ... /f:xml`, at most 2000), filters Kills / Errors and warnings / Sync / All, text search, period, and a "Kill summary" mode. Save report is here.

## What it writes, and when

Only: the agent's `.env` and `whitelist.txt`, their `<file>.bak-YYYYMMDD-HHMMSS(-N)` copies, and `GuardianReports\` next to the exe. Never the registry, `%APPDATA%`, `agent_credentials.json` or `sync.json`; `whitelist.txt` is never deleted (the agent regenerates it from running processes if absent).

Save (administrator only): validate -> confirmation dialog with a diff -> every file is read again and checked against its snapshot hash (a read failure is reported as such, not as "changed on disk"; nothing is written until all files pass; changed on disk offers Reload / Overwrite) -> per file: backup from the same checked bytes, temp file in the same folder, `sync_all`, `std::fs::rename` (MoveFileExW REPLACE_EXISTING; no `unsafe`), read back and compare -> rotation keeps the 5 newest `.bak-*` per file (only names of that exact shape, never the one just made) -> "Restart ProcSentinelAgent now?" (stop + start via SCM, 30 s per phase). The agent reads both files once at startup, so edits apply only after a restart. A hand-picked folder can be edited but the service restart is not offered.

Save report writes `GuardianReports\<host>-<YYYYMMDD-HHMM>.txt` and `.json`; an existing report is never overwritten (`-1`, `-2`, ... suffix). If the drive is write-protected the button says so and offers another folder.

## CLI

```
Guardian.exe [--tab diagnostics|configuration|processes|log] [--dir <agent folder>]
Guardian.exe report [--json] [--dir <agent folder>] [--offline]
```

`report` prints the same report as Save report, without a window. It writes nothing to the machine, except when `Guardian.exe` has no console/stdout (double-clicked, or cmd without a redirect): then it saves the report to `GuardianReports\` next to the exe and shows a message box. Guardian.exe is a GUI-subsystem program, so cmd does not wait for it; read the output and exit code with `start /wait Guardian.exe report > report.txt` (then `%ERRORLEVEL%`).

Exit codes: `0` protection works (On); `10` warnings or a verdict of Stale / Free / Unknown (anything >= 10); `11` errors or NotInstalled / Stopped / NotEnrolled / NeverSynced; `2` usage error. The code is the maximum of the checks' severity and the verdict's.

## Layout, build, tests

Workspace: `crates/core` (pure: parsing, diagnosis, edit rules, enforce, report; reads data only through `Snapshot`), `crates/winhost` (Windows: SCM, registry, `tasklist`, `wevtutil`, files, apply, report file), `crates/ui-egui` (eframe with `glow`), `apps/guardian` (the `Guardian.exe` binary, manifest/icon via `winresource`).

```sh
cd tools/guardian-diag
cargo build --release -p guardian --target i686-pc-windows-msvc   # Guardian.exe; set GUARDIAN_REQUIRE_WINRES=1 to fail if icon/manifest cannot be embedded
cargo test --workspace
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
```

Workspace lints forbid `unsafe` and deny `unwrap`/`expect`/`panic` outside tests. Linux needs `libxkbcommon-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgtk-3-dev` (eframe, rfd).

CI (`.github/workflows/diag.yml`): `core` job on Linux (fmt, clippy, tests); `windows` job on `windows-latest` (tests for core/winhost/guardian on the i686 target, release build, smoke test of `report --offline` on the `healthy` fixture with an exit-code, VERDICT and secret-leak check, `dumpbin /dependents` check that the CRT is static; uploads the `Guardian.exe` artifact).

## Agent-consistency tests

`crates/core/src/agent.rs` mirrors the agent: protected-process list, file names, `.env` keys, default intervals and log strings. `crates/core/tests/agent_consistency.rs` reads `agent/main.go`, `agent/agent.go`, `agent/client.go` and fails when they drift. When one fails, the agent changed: update `agent.rs` to match (and the behaviour that depends on it, then this file and the design spec); do not weaken the test. Also keep `tools/whitelist-gui/builtin.go` in sync by hand.

## Two `Guardian.exe`

The installer archive (`server/installer.go`) still ships the Go console (`tools/whitelist-gui`) as `Guardian.exe`: installer and updater. Guardian Diag builds a different program with the same file name (the agent's protected list contains `guardian.exe`, so the window survives whitelist mode). They are separate artifacts; replacing the archive's console is an open decision (design spec, Open questions).

## Manual verification (Windows 10 x64, and x86 if available)

CI cannot drive the GUI. Record results here or in the PR description.

| # | Check | Result |
|---|---|---|
| 1 | Non-admin double-click from a USB stick: window opens, verdict shown, Configuration read-only with "Run as administrator to edit"; unreadable fields say "no access". | |
| 2 | "Restart as administrator" -> UAC -> reopens on the same tab; cancel UAC -> old window stays with "Elevation was cancelled". | |
| 3 | Admin: add `winword.exe`, Save -> confirm -> `whitelist.txt` updated, `whitelist.txt.bak-*` present, restart dialog -> Restart -> Processes shows `winword.exe` "Allowed (whitelist.txt)"; Event Log shows a new "Server address:" start record. | |
| 4 | Change `CHECK_INTERVAL` to 15 -> Save -> Restart -> `Synced:` records ~15 s apart. | |
| 5 | Change `SERVER_ADDRESS` to another host -> the drop-out warning appears; Cancel. | |
| 6 | Open `.env` in Notepad between load and Save, edit, save -> Guardian's Save shows "changed on disk" -> Reload. | |
| 7 | Agent in whitelist mode: the window survives (protected name `guardian.exe`). | |
| 8 | 32-bit `Guardian.exe` on 64-bit Windows: paths shown as `System32`, edits land in the real folder (check with 64-bit Explorer). | |
| 9 | `start /wait Guardian.exe report > r.txt` from cmd: file has the report and `%ERRORLEVEL%` matches the verdict. | |
| 10 | (a) `crates/core/tests/fixtures/wevtutil.xml` is hand-written: confirm it matches real `wevtutil qe Application "/q:*[System[Provider[@Name='ProcSentinelAgent']]]" /f:xml /rd:true` output, and that event text decodes correctly on a Cyrillic (ru-RU) Windows. | |
| 11 | (b) `Guardian.exe report` run from a bare cmd without a redirect saves the file to `GuardianReports\` and shows the message box with the path. | |
