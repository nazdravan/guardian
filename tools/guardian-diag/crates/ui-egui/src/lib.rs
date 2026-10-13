//! Guardian's window: four tabs over a Snapshot (spec R6–R23).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod state;
pub mod tabs;
pub mod theme;
pub mod worker;

use std::path::PathBuf;

use guardian_core::diagnose::{Diagnosis, diagnose};
use guardian_core::snapshot::Snapshot;
use guardian_winhost::CollectOptions;

use state::{Draft, Tab};
use worker::{Done, Job, Worker};

pub struct GuardianApp {
    pub tab: Tab,
    pub snapshot: Option<Snapshot>,
    pub diagnosis: Option<Diagnosis>,
    pub draft: Option<Draft>,
    pub manual_dir: Option<PathBuf>,
    pub worker: Option<Worker>,
    /// A message shown in the top bar until the next action (errors and confirmations).
    pub notice: Option<(guardian_core::diagnose::Status, String)>,
    pub dialog: Option<tabs::configuration::Dialog>,
    pub why_query: String,
    pub process_filter: String,
    pub only_closing: bool,
    pub log_filter: tabs::log::Filter,
    pub log_period: tabs::log::Period,
    pub log_search: String,
    pub kill_summary: bool,
    pub new_entry: String,
    fonts_ready: bool,
}

impl GuardianApp {
    pub fn new(cc: &eframe::CreationContext<'_>, tab: Option<String>, dir: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self::empty(tab.as_deref().and_then(Tab::from_arg).unwrap_or(Tab::Diagnostics));
        app.manual_dir = dir;
        app.refresh(&cc.egui_ctx);
        app
    }

    fn empty(tab: Tab) -> Self {
        GuardianApp {
            tab,
            snapshot: None,
            diagnosis: None,
            draft: None,
            manual_dir: None,
            worker: None,
            notice: None,
            dialog: None,
            why_query: String::new(),
            process_filter: String::new(),
            only_closing: false,
            log_filter: tabs::log::Filter::All,
            log_period: tabs::log::Period::All,
            log_search: String::new(),
            kill_summary: false,
            new_entry: String::new(),
            fonts_ready: false,
        }
    }

    /// Tests: a ready window over a fixed snapshot, no worker, no I/O.
    pub fn with_snapshot(s: Snapshot, tab: Tab) -> Self {
        let mut app = Self::empty(tab);
        app.set_snapshot(s);
        app
    }

    pub fn set_snapshot(&mut self, s: Snapshot) {
        self.diagnosis = Some(diagnose(&s));
        self.draft = Some(Draft::from_snapshot(&s));
        self.snapshot = Some(s);
    }

    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.worker = Some(Worker::spawn(Job::Collect(CollectOptions { manual_dir: self.manual_dir.clone(), probe_health: true }), ctx.clone()));
    }

    pub fn dirty(&self) -> usize {
        match (&self.draft, &self.snapshot) {
            (Some(d), Some(s)) => d.dirty_count(s),
            _ => 0,
        }
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let Some(done) = self.worker.as_ref().and_then(Worker::poll) else { return };
        self.worker = None;
        match done {
            Done::Collected(s) => self.set_snapshot(*s),
            other => tabs::configuration::on_done(self, other, ctx),
        }
    }
}

impl eframe::App for GuardianApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Named font families need one frame before first use.
        if !self.fonts_ready {
            self.fonts_ready = true;
            ctx.request_repaint();
            return;
        }
        self.poll_worker(&ctx);
        tabs::top_bar(self, ui);
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(_) = &self.snapshot else {
                ui.centered_and_justified(|ui| ui.label(self.worker.as_ref().and_then(Worker::progress).unwrap_or_default()));
                return;
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Diagnostics => tabs::diagnostics::show(self, ui),
                Tab::Configuration => tabs::configuration::show(self, ui),
                Tab::Processes => tabs::processes::show(self, ui),
                Tab::Log => tabs::log::show(self, ui),
            });
        });
        tabs::configuration::dialogs(self, &ctx);
    }
}

pub fn run(tab: Option<String>, dir: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default().with_title("Guardian").with_inner_size([1100.0, 720.0]).with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native("Guardian", options, Box::new(move |cc| Ok(Box::new(GuardianApp::new(cc, tab, dir))))).map_err(|e| e.to_string())
}
