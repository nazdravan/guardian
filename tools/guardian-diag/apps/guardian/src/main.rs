//! Guardian.exe: the window, or `report` for the same diagnosis without one.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// A double-click must not open a console window.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod args;

use std::io::Write;

use guardian_core::diagnose::diagnose;
use guardian_core::report::{exit_code, report};
use guardian_winhost::{CollectOptions, collect};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match args::parse(&argv) {
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "{e}\n\n{}", args::USAGE);
            2
        }
        Ok(args::Command::Report { json, dir, offline }) => run_report(json, dir, offline),
        Ok(args::Command::Window { tab, dir }) => match guardian_ui::run(tab, dir) {
            Ok(()) => 0,
            Err(e) => {
                rfd::MessageDialog::new()
                    .set_title("Guardian")
                    .set_description(format!("Guardian could not open its window ({e}). Run: Guardian.exe report > report.txt"))
                    .show();
                1
            }
        },
    };
    std::process::exit(code);
}

fn run_report(json: bool, dir: Option<std::path::PathBuf>, offline: bool) -> i32 {
    let s = collect(&CollectOptions { manual_dir: dir, probe_health: !offline });
    let d = diagnose(&s);
    let r = report(&s, &d);
    let body = if json { serde_json::to_string_pretty(&r.json).unwrap_or_default() } else { r.text.clone() };
    // A GUI-subsystem exe has no console of its own: output reaches a redirect or pipe
    // (`Guardian.exe report > report.txt`), but not a bare console. If stdout is
    // unusable, save the report next to the exe instead and say where (spec R25).
    if !print_report(&body) {
        let msg = match guardian_winhost::save_report(&s) {
            Ok(p) => format!("Saved the report to {}", p.display()),
            Err(e) => format!("Could not print or save the report: {e}"),
        };
        rfd::MessageDialog::new().set_title("Guardian report").set_description(msg).show();
    }
    exit_code(&d)
}

/// Whether the process has a stdout at all. Rust's std swallows ERROR_INVALID_HANDLE, so a
/// GUI-subsystem exe started from a bare console would "write" successfully into nothing.
#[cfg(windows)]
fn has_stdout() -> bool {
    use std::os::windows::io::AsRawHandle;
    !std::io::stdout().as_raw_handle().is_null()
}

#[cfg(not(windows))]
fn has_stdout() -> bool {
    true
}

/// True when the report was written to stdout; false means use the fallback.
fn print_report(body: &str) -> bool {
    if !has_stdout() {
        return false;
    }
    let mut out = std::io::stdout();
    out.write_all(body.as_bytes()).and_then(|_| out.flush()).is_ok()
}
