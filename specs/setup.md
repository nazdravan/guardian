# Guardian 3.0 Setup (`guardian-setup`, Windows)

Status: stage 3c of [`2026-11-17-rust-rewrite-design.md`](2026-11-17-rust-rewrite-design.md) (§8.1, §8.3, §10.3,
§11.3), plan [`plans/2026-11-18-rust-3-stage3c-setup-release.md`](plans/2026-11-18-rust-3-stage3c-setup-release.md).
It replaces the 2.x Guardian Console (`tools/whitelist-gui`, Go) and the `.bat`/`.ps1` installers. The agent it
installs is described in [`agent.md`](agent.md). The UI is in English.

`guardian-setup.exe` is one 32-bit program (`i686-pc-windows-msvc`, static CRT) that runs on 32- and 64-bit
Windows 10/11. It ships in `agent-windows/` of the release archive next to `bin\x64\` and `bin\x86\` (agent and
worker). Without arguments it opens a window; with a command it works as a command line.

## Layers (`crates/guardian-setup`)

| Module | What |
|---|---|
| `model` | pure: the architecture to install (`os_arch`), the file layout (`layout`, `WinPath`), `Sysnative` mapping, `netsh` arguments, the local `whitelist.txt` (`Whitelist`, `generated_whitelist`), `Secret` |
| `host` | the `Host` trait (services, child programs, processes, firewall, elevation); `UnsupportedHost` off Windows |
| `fake` (feature `testing`) | `FakeHost`: a temporary directory for the drives, services and rules in memory, a simulated `guardian-agent.exe` |
| `winhost` | `WinHost`: `windows-service`, `netsh`, `guardian-platform` (processes, elevation, `AttachConsole`) |
| `lifecycle` | `install`, `update`, `uninstall`, `reenroll`, `status`, the whitelist file |
| `ops` | the table of operations (`OPS`, `run_op`: lock + `catch_unwind`) used by the window and the CLI |
| `cli` | `clap`, text or `--json`, exit codes |
| `app` | the egui window (operations on worker threads) |
| `elevate` | the one-time UAC relaunch |

## Files on the student computer

| | Where |
|---|---|
| Agent and worker | `%ProgramW6432%\Guardian\` = `C:\Program Files\Guardian` (`%ProgramFiles%` on 32-bit Windows) |
| Agent config and data | `%ProgramData%\Guardian Agent\` (`agent.toml`, `agent.key`, `state.pb`, `local.pb`, `whitelist.txt`, `agent.log`) |
| Setup lock and log | `%ProgramData%\Guardian\setup.lock`, `setup.log` (non-elevated: `%LOCALAPPDATA%\Guardian\setup.log`) |
| Guardian 2.x | service `ProcSentinelAgent`, files in `%SystemRoot%\System32\ProcSentinel\agent` (opened through `Sysnative` from this 32-bit process) |

The architecture comes from `PROCESSOR_ARCHITEW6432`, else `PROCESSOR_ARCHITECTURE`: `AMD64` → `bin\x64`, `x86` →
`bin\x86`, `ARM64` → `bin\x64` (emulated).

## Operations

| Command | Admin | What |
|---|---|---|
| `status [--check]` | no | installed, service state, computer id, server, local fallback port and firewall rule, Guardian 2.x, agent version, problems |
| `install --server wss://… (--code C \| --code-file F) [--cert-sha256 H] [--local-port N] [--remove-legacy] [--no-start]` | yes | see below |
| `update` | yes | stops the service, replaces agent and worker with the bundled ones (old ones kept as `*.old`), starts it; if it does not start within 30 s the old binaries come back and are started |
| `uninstall --yes [--purge]` | yes | removes the service, the firewall rule and `Program Files\Guardian`; `--purge` also the data (enrollment, key, whitelist) |
| `reenroll (--code C \| --code-file F) [--server S] [--cert-sha256 H]` | yes | stops the service, deletes `agent.key`, the computer id, `state.pb`, `seen.txt`, `local.pb`, enrolls again, starts the service (§10.3) |
| `whitelist list` / `add NAME` / `remove NAME` | add/remove | the local whitelist (the agent re-reads it at its next scan) |
| `processes` | no | running program names |
| `service start` / `stop` / `restart` | yes | the `GuardianAgent` service |
| `logs [--lines N]`, `version` | no | CLI only |

`install`, in order:
1. administrator rights;
2. Guardian 2.x: refused with `--remove-legacy` missing; with it, its service is stopped and deleted and its folder removed;
3. agent and worker copied for this architecture (the running service is stopped first);
4. `guardian-agent enroll` only if `agent.toml` has no computer id (an existing enrollment is kept);
5. `local_port` written to `agent.toml` when given;
6. `whitelist.txt` written only if missing: the programs running now, except those started from `C:\Users\…`
   and those the agent protects anyway (§8.3);
7. the firewall rule `Guardian local fallback`: TCP and UDP on the local fallback port (default 11500),
   `remoteip=localsubnet`, any profile (§11.3); port `0` removes it;
8. `guardian-agent service install` (which starts it), or a start when the service exists.

The enrollment code never appears in output, JSON, the log or `Debug` (`Secret`). `--code-file -` reads it from
standard input so it does not show in the process list.

## Command line

Exit codes: `0` ok, `1` failed, `2` usage (including `uninstall` without `--yes`), `5` needs administrator
rights (`Run this from an elevated prompt (Run as administrator).`), `10` `status --check` found a problem.
`--json` prints `{"ok", "op", "result", "error"}`. A mutating operation takes `setup.lock`; a second one gets
`Another Guardian Setup operation is in progress.` Progress goes to standard error, the result to standard
output; started from Explorer with arguments (no console, nothing redirected), the result shows in a dialog.

## Window

The manifest is `asInvoker`. Started without administrator rights, the window starts itself once more through
UAC (`powershell Start-Process -Verb RunAs … --elevated-relaunch`); if that is refused it stays read-only with a
banner. Tabs: **Overview** (the install form — Server address, Certificate SHA-256 (optional), Enrollment code —
or the card `Computer #id · server` with Start, Stop, Restart, Update, Re-enroll…, Uninstall…), **Whitelist**,
**Processes**. Guardian 2.x is named in a banner and removed only after "Remove Guardian 2.x and install 3.0?".
Errors stay on screen; the window never closes on a failed operation.

## Tests

`cargo test -p guardian-setup --features testing`: `model`, `lifecycle` and `ops` on `FakeHost` (every step and its
failures, rollback, legacy, admin, busy, panic), `ui` (`egui_kittest`), `binary`. `tests/winhost.rs` runs on
Windows only (CI `windows` job).

## Manual checks on Windows (before release)

1. 64-bit Windows: the agent lands in `C:\Program Files\Guardian` (not `Program Files (x86)`), `bin\x64`'s agent runs.
2. 32-bit Windows: `bin\x86`'s agent.
3. A PC with Guardian 2.x: the banner, the question, then only `GuardianAgent` remains.
4. A wrong code: the error is shown, no service is registered; the right code then works.
5. Started by a standard user: one UAC prompt; refusing it leaves a read-only window.
6. `netsh advfirewall firewall show rule name="Guardian local fallback"` lists a TCP and a UDP rule with `LocalSubnet`.
7. `guardian-setup.exe status --json > s.json` writes the JSON to the file; `guardian-setup.exe status` in cmd prints to the console.
