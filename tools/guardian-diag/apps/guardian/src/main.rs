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
        Ok(args::Command::Window { tab: _, dir: _ }) => {
            let _ = writeln!(std::io::stderr(), "the window is not built yet");
            2
        }
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
        let msg = match save_report_next_to_exe(&s.machine.hostname, &r) {
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

fn save_report_next_to_exe(host: &str, r: &guardian_core::report::Report) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no exe folder")?.join("GuardianReports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = format!("{host}-{}", chrono::Local::now().format("%Y%m%d-%H%M"));
    std::fs::write(dir.join(format!("{stem}.txt")), &r.text).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_string_pretty(&r.json).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{stem}.txt")))
}
