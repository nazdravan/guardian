# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Guardian 3.0 (formerly ProcSentinel) controls which programs run on the computers of a school class. It is
written in Rust (branch `rust-rewrite`; the 2.x Go code was removed in stage 3c and lives in git history):

- **Server** (`guardian-server`) — gRPC API + WebSocket hub for agents + SQLite; a Linux systemd service, a
  Windows service, or embedded in the console
- **Agent** (`guardian-agent` + `guardian-worker`) — Windows (32/64-bit) and Linux service that enforces the
  computer's profile (blacklist/whitelist) and keeps doing so offline; the worker runs in the user's session
- **Console** (`guardian-console`) — the teacher's egui window: remote or embedded server, rooms, profiles,
  computers, users, enrollment, local fallback mode
- **Setup** (`guardian-setup`) — Windows installer/diagnostics for the agent, window + command line
- **CLI** (`guardian-cli`) — the gRPC API for scripts
- **Mobile** (`mobile/`, Flutter) — the 2.x Android app; it does not work with 3.0 and is not in the release
  archive until it is rewritten

Design: [`specs/2026-11-17-rust-rewrite-design.md`](specs/2026-11-17-rust-rewrite-design.md). Per component:
[`specs/server.md`](specs/server.md), [`specs/agent.md`](specs/agent.md), [`specs/console.md`](specs/console.md),
[`specs/local-fallback.md`](specs/local-fallback.md), [`specs/setup.md`](specs/setup.md), API reference
[`specs/api.md`](specs/api.md). Plans in `specs/plans/`.

## Workspace

Cargo workspace at the repo root (`Cargo.toml`, `crates/*`, `proto/`), Rust edition 2024, MSRV 1.95 (egui 0.37):

| Crate | What |
|---|---|
| `crates/guardian-proto` | `proto/guardian/v1/*.proto` compiled with `protox` in `build.rs` (no `protoc`) |
| `crates/guardian-core` | policy, local fallback rules, protected processes, kill/allow decision, ed25519 keys, codes, feature registry |
| `crates/guardian-server` | server library + `guardian-server` binary; feature `testing` = `TestServer`, `SimAgent` |
| `crates/guardian-client` | TLS with certificate pin, gRPC channel, token interceptor, WebSocket connector |
| `crates/guardian-local` | local fallback channel: Noise IK over TCP with the ed25519 keys, signed UDP discovery |
| `crates/guardian-cli` | `guardian-cli` |
| `crates/guardian-platform` | OS access behind the `Platform` trait (Linux `/proc` + proc connector + `loginctl`, Windows ToolHelp + ETW + WTS), local IPC, Windows elevation/console helpers; feature `testing` = `FakePlatform` |
| `crates/guardian-agent` | agent library + `guardian-agent` (`enroll`, `run`, `status`, Windows `service`), local fallback listener |
| `crates/guardian-worker` | `guardian-worker`, started by the agent in the user session |
| `crates/guardian-console` | `guardian-console` (egui); feature `testing` = `FakeBackend` for `egui_kittest` |
| `crates/guardian-setup` | `guardian-setup` (egui + CLI, Windows); feature `testing` = `FakeHost` |
| `crates/guardian-applog` | log rotation and the panic hook of the desktop programs |

```sh
cargo build --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo deny check
```

- UI tests need `--features testing` (`cargo test -p guardian-console --features testing`,
  `cargo test -p guardian-setup --features testing`); `--all-features` above covers them. The console's
  `real_embedded` and `real_fallback` (~75 s) tests need `cargo build --workspace --bins` first.
- Linux-only tests run in a container from macOS (repo mounted at `/w`, `CARGO_TARGET_DIR=/w/target/linux`,
  image = `rust` + clippy/rustfmt); the real proc connector test needs `--cap-add NET_ADMIN --network host
  --pid host` and `-- --include-ignored`. The full real run: `cargo build --workspace --bins && cargo test -p
  guardian-agent --test real_linux`.
- Windows code is checked locally with `cargo clippy --target x86_64-pc-windows-gnu` / `i686-pc-windows-gnu`;
  its tests (incl. ETW and `guardian-setup`'s `winhost`) run in the CI `windows` job.
- Scripts: `bash scripts/test-package.sh` (archive layout), `python3 -I scripts/check-workflows.py`,
  `bash scripts/check-no-go.sh --docs`, `bash scripts/check-docs.sh` (links and the landing page's i18n keys).

## Rules

- The `.proto` files are the contract: field numbers never change.
- `guardian-core` is the only place policy logic lives (also the local fallback rules).
- Libraries and binaries are `#![forbid(unsafe_code)]`; `unsafe` only in `guardian-platform/src/windows/ffi_*.rs`,
  with a `// SAFETY:` comment on each block.
- No `unwrap`/`expect`/`panic` outside tests (the single exception is `guardian_core::random::random_bytes`);
  anything from the network must never panic.
- Dependencies must pass `cargo deny` (no GPL/MPL).
- System tools (`netsh`, `icacls`, `powershell`, …) are started by absolute path under `%SystemRoot%\System32`.
- **Update the specs with the code:** after any change, update the matching file in `specs/` (`server.md`,
  `agent.md`, `console.md`, `local-fallback.md`, `setup.md`, `api.md`) and this `AGENTS.md` (file structure,
  commands, build output paths).

## Release

`VERSION` (repo root) is the release version and matches `version` in `Cargo.toml`. `dist/` is build output and
not tracked. `scripts/package.sh --linux DIR --win64 DIR --win32 DIR [--out DIR]` packs already built binaries
with the templates in `packaging/`:

```
guardian-<VERSION>.zip
└── guardian-<VERSION>/
    ├── server-linux/    # guardian-server, guardian-cli, guardian-server.service, install.sh
    ├── server-windows/  # guardian-server.exe, guardian-cli.exe (64-bit), README.txt
    ├── console-windows/ # guardian-console.exe (32-bit), README.txt
    ├── agent-windows/   # guardian-setup.exe (32-bit), README.txt, bin/x64/ and bin/x86/ (agent + worker)
    └── agent-linux/     # guardian-agent, guardian-worker, guardian-agent.service, install.sh, uninstall.sh
```

`.github/workflows/release.yml` runs **only on push to `main`**: a `linux` and a `windows` job build and test,
`package` packs the zip and publishes a GitHub Release only when `VERSION` names a version with no tag yet.
Windows binaries are signed (Authenticode) only when the `WINDOWS_CERT_PFX_BASE64` / `WINDOWS_CERT_PASSWORD`
secrets exist. `.github/workflows/rust.yml` runs the checks on every push touching Rust, packaging or scripts.

Install paths: the Linux server binary `/usr/local/bin/guardian-server` (data `/var/lib/guardian`), the Linux
agent `/opt/guardian-agent/` (config `/etc/guardian-agent/agent.toml`, data `/var/lib/guardian-agent`), the
Windows agent `C:\Program Files\Guardian` (data `%ProgramData%\Guardian Agent`), the Windows server data
`%ProgramData%\Guardian Server`, the embedded server data `%ProgramData%\Guardian\server`.

## Landing page (`docs/`)

- Static marketing page: one self-contained `docs/index.html`, Tailwind via CDN (config inlined), fonts from
  Google Fonts (Space Grotesk / Inter / JetBrains Mono), dark "terminal" look with an animated hero (vanilla JS,
  respects `prefers-reduced-motion`); served by GitHub Pages from `docs/`
- Texts are i18n keys in five languages (en, ro, ru, es, de) in the `I18N` object; `scripts/check-docs.sh`
  checks every key exists in all five
- Brand assets in `docs/assets/` (also used by `README.md`); the `#app` section's screenshots are the 2.x Android
  app (`docs/assets/screenshots/`), the `#video` section the 2.x Windows install (YouTube `CkksD9YpDVw`, in
  Romanian), both labelled as 2.x
- Preview locally: `cd docs && python3 -m http.server 8099` → http://localhost:8099
