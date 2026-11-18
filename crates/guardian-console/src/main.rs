//! `guardian-console`: the teacher's window.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use guardian_console::backend::{Action, Backend, RealBackend};
use guardian_console::instance::{InstanceError, InstanceLock};
use guardian_console::settings::{self, ConsoleSettings, Mode};
use guardian_console::ui::ConsoleApp;
use guardian_console::{VERSION, logging};

/// Guardian Console: rooms, profiles and computers of a Guardian server.
#[derive(Debug, Parser)]
#[command(name = "guardian-console", version = VERSION)]
struct Args {
    /// Settings file (development and tests).
    #[arg(long)]
    settings: Option<PathBuf>,
}

fn dialog(text: &str) {
    rfd::MessageDialog::new()
        .set_title("Guardian Console")
        .set_description(text)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

fn native_options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Guardian Console")
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([800.0, 520.0]);
    if let Ok(icon) =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/guardian-mark.png"))
    {
        viewport = viewport.with_icon(icon);
    }
    eframe::NativeOptions {
        viewport,
        renderer,
        ..Default::default()
    }
}

fn run(
    renderer: eframe::Renderer,
    settings_path: PathBuf,
    settings: ConsoleSettings,
) -> Result<(), eframe::Error> {
    eframe::run_native(
        "Guardian Console",
        native_options(renderer),
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let backend =
                RealBackend::start(settings_path, Box::new(move || ctx.request_repaint()));
            match &settings.mode {
                Some(Mode::Embedded { port }) => backend.send(Action::StartEmbedded {
                    data_dir: None,
                    port: *port,
                }),
                Some(Mode::Remote {
                    server,
                    cert_sha256,
                }) => backend.send(Action::Connect {
                    server: server.clone(),
                    cert_sha256: cert_sha256.clone(),
                }),
                None => {}
            }
            Ok(Box::new(ConsoleApp::new(backend, settings)))
        }),
    )
}

fn main() -> ExitCode {
    let args = Args::parse();
    let settings_path = args.settings.unwrap_or_else(settings::settings_path);
    let lock_dir = settings_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let _lock = match InstanceLock::acquire(&lock_dir) {
        Ok(l) => l,
        Err(InstanceError::AlreadyRunning) => {
            rfd::MessageDialog::new()
                .set_title("Guardian Console")
                .set_description("Guardian Console is already running.")
                .set_level(rfd::MessageLevel::Info)
                .show();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            dialog(&format!("Guardian Console cannot start: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let log = settings::log_path();
    logging::init_logging(&log);
    logging::log_panics();
    tracing::info!("guardian-console {VERSION} starting");
    let settings = match ConsoleSettings::load(&settings_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("{e}; starting with empty settings");
            ConsoleSettings::default()
        }
    };
    let force_gl = std::env::var("GUARDIAN_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("gl"));
    let first = if force_gl {
        eframe::Renderer::Glow
    } else {
        eframe::Renderer::Wgpu
    };
    let result = match run(first, settings_path.clone(), settings.clone()) {
        Err(e) if !force_gl => {
            tracing::warn!("the wgpu renderer failed ({e}); trying OpenGL");
            run(eframe::Renderer::Glow, settings_path, settings)
        }
        other => other,
    };
    match result {
        Ok(()) => {
            tracing::info!("guardian-console closed");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("cannot open the window: {e}");
            dialog(&format!(
                "Guardian Console cannot open its window: {e}\n\nLog: {}",
                log.display()
            ));
            ExitCode::FAILURE
        }
    }
}
