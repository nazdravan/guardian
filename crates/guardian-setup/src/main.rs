//! `guardian-setup.exe`: without arguments the window, with a command the
//! command line (attached to the console it was started from).

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use guardian_setup::app::SetupApp;
use guardian_setup::cli::main_cli;
use guardian_setup::elevate::{
    AfterRelaunch, RELAUNCH_FLAG, after_relaunch, relaunch_args, should_relaunch,
};
use guardian_setup::host::Host;

fn host() -> Arc<dyn Host> {
    #[cfg(windows)]
    {
        Arc::new(guardian_setup::winhost::WinHost)
    }
    #[cfg(not(windows))]
    {
        Arc::new(guardian_setup::host::UnsupportedHost)
    }
}

fn log_path(h: &dyn Host) -> PathBuf {
    let base = if h.is_elevated() {
        h.env("ProgramData")
    } else {
        h.env("LOCALAPPDATA")
    };
    match base {
        Some(b) => PathBuf::from(b).join("Guardian").join("setup.log"),
        None => std::env::temp_dir().join("guardian-setup.log"),
    }
}

fn dialog(title: &str, text: &str, error: bool) {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_level(if error {
            rfd::MessageLevel::Error
        } else {
            rfd::MessageLevel::Info
        })
        .show();
}

/// Whether standard output already goes somewhere (a pipe or a file).
fn stdout_redirected() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        !std::io::stdout().as_raw_handle().is_null()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn cli(h: &dyn Host, args: Vec<OsString>) -> ExitCode {
    let redirected = stdout_redirected();
    #[cfg(windows)]
    let console = redirected || guardian_platform::windows_util::attach_parent_console();
    #[cfg(not(windows))]
    let console = redirected;
    let mut out = Vec::new();
    let code = main_cli(h, args, &mut out, &mut std::io::stdin());
    let text = String::from_utf8_lossy(&out).into_owned();
    if console {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    } else if !text.trim().is_empty() {
        dialog("Guardian Setup", text.trim(), code != 0);
    }
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn window(h: Arc<dyn Host>) -> ExitCode {
    let options = |renderer| eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Guardian Setup")
            .with_inner_size([820.0, 600.0])
            .with_min_inner_size([640.0, 460.0]),
        renderer,
        ..Default::default()
    };
    let run = |renderer| {
        let h = h.clone();
        eframe::run_native(
            "Guardian Setup",
            options(renderer),
            Box::new(move |_cc| Ok(Box::new(SetupApp::new(h)))),
        )
    };
    let force_gl = std::env::var("GUARDIAN_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("gl"));
    let result = if force_gl {
        run(eframe::Renderer::Glow)
    } else {
        match run(eframe::Renderer::Wgpu) {
            Err(e) => {
                tracing::warn!("the wgpu renderer failed ({e}); trying OpenGL");
                run(eframe::Renderer::Glow)
            }
            ok => ok,
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("cannot open the window: {e}");
            dialog(
                "Guardian Setup",
                &format!(
                    "Guardian Setup cannot open its window: {e}\n\nThe command line works without it: guardian-setup.exe --help"
                ),
                true,
            );
            ExitCode::FAILURE
        }
    }
}

/// `Some(success)` of the elevated relaunch, `None` when it could not run.
fn relaunch_elevated(h: &dyn Host) -> Option<bool> {
    let exe = std::env::current_exe().ok()?;
    let root = h.env("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let ps = PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut cmd = std::process::Command::new(ps);
    cmd.args(relaunch_args(&exe));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing behind the UAC prompt.
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.status() {
        Ok(s) => Some(s.success()),
        Err(e) => {
            tracing::warn!("cannot ask for administrator rights: {e}");
            None
        }
    }
}

fn main() -> ExitCode {
    let all: Vec<OsString> = std::env::args_os().collect();
    let h = host();
    guardian_applog::init_logging(&log_path(&*h));
    guardian_applog::log_panics();
    let args: Vec<OsString> = all
        .iter()
        .filter(|a| *a != RELAUNCH_FLAG)
        .cloned()
        .collect();
    if args.len() > 1 {
        return cli(&*h, args);
    }
    if cfg!(windows)
        && should_relaunch(h.is_elevated(), &all)
        && after_relaunch(relaunch_elevated(&*h)) == AfterRelaunch::Exit
    {
        // The elevated copy takes it from here; a refused prompt falls through
        // to the read-only window.
        return ExitCode::SUCCESS;
    }
    window(h)
}
