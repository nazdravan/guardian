//! The setup window. It only draws and starts operations from `ops`; each one
//! runs on its own thread and reports back on a channel.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, RichText};

use crate::host::{Host, ServiceState};
use crate::lifecycle::{InstallArgs, Status};
use crate::model::Secret;
use crate::ops::{Op, Output, run_op};

/// Shown when the window has no administrator rights.
pub const NOT_ELEVATED: &str = "Guardian Setup runs without administrator rights: it can show the state but not change it. Start it again with \"Run as administrator\".";
/// Shown when Guardian 2.x is found.
pub const LEGACY_TEXT: &str =
    "Guardian 2.x is installed on this computer. It will be removed during the installation.";

enum Msg {
    Progress(String),
    Done {
        op: &'static str,
        result: Result<Output, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Whitelist,
    Processes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Legacy,
    Uninstall,
}

/// The window's state.
pub struct SetupApp {
    host: Arc<dyn Host>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: Option<egui::Context>,
    pending: usize,
    status: Option<Status>,
    tab: Tab,
    server: String,
    cert: String,
    code: String,
    error: Option<String>,
    progress: Vec<String>,
    confirm: Option<Confirm>,
    purge: bool,
    reenroll_open: bool,
    reenroll_code: String,
    whitelist: Vec<String>,
    new_name: String,
    processes: Vec<String>,
}

impl std::fmt::Debug for SetupApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupApp")
            .field("pending", &self.pending)
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

fn red(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(Color32::from_rgb(0xb3, 0x26, 0x1e), text);
}

fn amber(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(Color32::from_rgb(0x8a, 0x5a, 0x00), text);
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) {
    let l = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(420.0),
    )
    .labelled_by(l.id);
}

impl SetupApp {
    /// A window over `host`; it reads the state at once.
    pub fn new(host: Arc<dyn Host>) -> SetupApp {
        let (tx, rx) = channel();
        let mut app = SetupApp {
            host,
            tx,
            rx,
            ctx: None,
            pending: 0,
            status: None,
            tab: Tab::Overview,
            server: String::new(),
            cert: String::new(),
            code: String::new(),
            error: None,
            progress: Vec::new(),
            confirm: None,
            purge: false,
            reenroll_open: false,
            reenroll_code: String::new(),
            whitelist: Vec::new(),
            new_name: String::new(),
            processes: Vec::new(),
        };
        app.launch(Op::Status { check: false });
        app
    }

    /// An operation is running.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    fn launch(&mut self, op: Op) {
        self.pending += 1;
        let (host, tx, ctx) = (self.host.clone(), self.tx.clone(), self.ctx.clone());
        let name = op.name();
        let spawned = std::thread::Builder::new()
            .name(format!("setup {name}"))
            .spawn(move || {
                let ptx = tx.clone();
                let r = run_op(&*host, op, &|m| {
                    let _ = ptx.send(Msg::Progress(m.to_string()));
                });
                let _ = tx.send(Msg::Done {
                    op: name,
                    result: r.map_err(|e| e.to_string()),
                });
                if let Some(c) = ctx {
                    c.request_repaint();
                }
            });
        if let Err(e) = spawned {
            self.pending -= 1;
            self.error = Some(format!("Cannot start {name}: {e}"));
        }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Progress(p) => {
                    self.progress.push(p);
                    if self.progress.len() > 50 {
                        self.progress.remove(0);
                    }
                }
                Msg::Done { op, result } => {
                    self.pending = self.pending.saturating_sub(1);
                    match result {
                        Ok(Output::Status(s)) => self.status = Some(s),
                        Ok(Output::List(v)) if op == "whitelist list" => self.whitelist = v,
                        Ok(Output::List(v)) if op == "processes" => self.processes = v,
                        Ok(_) => {
                            self.error = None;
                            if op == "install" || op == "reenroll" {
                                self.code.clear();
                                self.reenroll_code.clear();
                            }
                            if op.starts_with("whitelist") {
                                self.launch(Op::WhitelistList);
                            }
                            self.launch(Op::Status { check: false });
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }

    fn install(&mut self, remove_legacy: bool) {
        let cert = self.cert.trim();
        self.launch(Op::Install(InstallArgs {
            server: self.server.trim().to_string(),
            code: Secret::new(self.code.trim()),
            cert_sha256: (!cert.is_empty()).then(|| cert.to_string()),
            local_port: None,
            remove_legacy,
            no_start: false,
        }));
    }

    fn install_form(&mut self, ui: &mut egui::Ui, legacy: bool) {
        ui.heading("Install the Guardian agent");
        ui.label("Copy the values from Connection info… in the teacher's console.");
        ui.add_space(6.0);
        if legacy {
            amber(ui, LEGACY_TEXT);
        }
        field(ui, "Server address", &mut self.server, false);
        field(ui, "Certificate SHA-256 (optional)", &mut self.cert, false);
        field(ui, "Enrollment code", &mut self.code, true);
        ui.add_space(6.0);
        let ready = !self.server.trim().is_empty() && !self.code.trim().is_empty();
        if ui
            .add_enabled(ready && !self.busy(), egui::Button::new("Install"))
            .clicked()
        {
            if legacy {
                self.confirm = Some(Confirm::Legacy);
            } else {
                self.install(false);
            }
        }
    }

    fn installed_card(&mut self, ui: &mut egui::Ui, s: &Status) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!(
                    "Computer #{} · {}",
                    s.computer_id.map_or("?".to_string(), |i| i.to_string()),
                    s.server.as_deref().unwrap_or("-")
                ))
                .strong()
                .size(16.0),
            );
            let service = match s.service {
                Some(ServiceState::Running) => "running",
                Some(ServiceState::Stopped) => "stopped",
                Some(ServiceState::Starting) => "starting",
                Some(ServiceState::Stopping) => "stopping",
                Some(ServiceState::Other) => "paused",
                None => "not registered",
            };
            ui.label(format!("Service: {service}"));
            if s.local_port == 0 {
                ui.label("Local fallback: off");
            } else {
                ui.label(format!(
                    "Local fallback port {}: {}",
                    s.local_port,
                    if s.firewall_rule { "open" } else { "closed" }
                ));
            }
            if let Some(v) = &s.agent_version {
                ui.label(v.as_str());
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(!self.busy(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Start").clicked() {
                        self.launch(Op::ServiceStart);
                    }
                    if ui.button("Stop").clicked() {
                        self.launch(Op::ServiceStop);
                    }
                    if ui.button("Restart").clicked() {
                        self.launch(Op::ServiceRestart);
                    }
                    if ui.button("Update").clicked() {
                        self.launch(Op::Update);
                    }
                    if ui.button("Re-enroll…").clicked() {
                        self.reenroll_open = true;
                    }
                    if ui.button("Uninstall…").clicked() {
                        self.confirm = Some(Confirm::Uninstall);
                    }
                });
            });
        });
    }

    fn overview(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.status.clone() else {
            ui.label("Checking this computer…");
            return;
        };
        if s.installed {
            self.installed_card(ui, &s);
        } else {
            self.install_form(ui, s.legacy_agent);
        }
        for p in &s.problems {
            if s.installed || !p.contains("not installed") {
                amber(ui, p);
            }
        }
    }

    fn whitelist_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Programs always allowed on this computer in exam (whitelist) mode.");
        ui.horizontal(|ui| {
            let l = ui.label("Program name");
            ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(240.0))
                .labelled_by(l.id);
            if ui
                .add_enabled(
                    !self.new_name.trim().is_empty() && !self.busy(),
                    egui::Button::new("Add"),
                )
                .clicked()
            {
                let n = std::mem::take(&mut self.new_name);
                self.launch(Op::WhitelistAdd(n));
            }
        });
        ui.separator();
        let names = self.whitelist.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for n in names {
                ui.horizontal(|ui| {
                    ui.label(n.as_str());
                    let label = format!("Remove {n}");
                    let r = ui.button("Remove");
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, label.as_str())
                    });
                    if r.clicked() {
                        self.launch(Op::WhitelistRemove(n.clone()));
                    }
                });
            }
        });
    }

    fn processes_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Programs running now (read only).");
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in &self.processes {
                ui.label(p.as_str());
            }
        });
    }

    fn dialogs(&mut self, ui: &mut egui::Ui) {
        if let Some(c) = self.confirm {
            let question = match c {
                Confirm::Legacy => "Remove Guardian 2.x and install 3.0?",
                Confirm::Uninstall => "Remove the Guardian agent from this computer?",
            };
            let mut answer = None;
            egui::Modal::new(ui.scope_id().with("setup-confirm")).show(ui.ctx(), |ui| {
                ui.label(question);
                if c == Confirm::Uninstall {
                    ui.checkbox(
                        &mut self.purge,
                        "Also delete the enrollment and the whitelist",
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("No").clicked() {
                        answer = Some(false);
                    }
                    if ui.button("Yes").clicked() {
                        answer = Some(true);
                    }
                });
            });
            match answer {
                Some(true) => {
                    self.confirm = None;
                    match c {
                        Confirm::Legacy => self.install(true),
                        Confirm::Uninstall => {
                            let purge = std::mem::take(&mut self.purge);
                            self.launch(Op::Uninstall { purge });
                        }
                    }
                }
                Some(false) => self.confirm = None,
                None => {}
            }
        }
        if self.reenroll_open {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Re-enroll this computer")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.label("The computer gets a new identity; the old one stays on the server until an administrator revokes it.");
                    field(ui, "New enrollment code", &mut self.reenroll_code, true);
                    if ui
                        .add_enabled(
                            !self.reenroll_code.trim().is_empty(),
                            egui::Button::new("Re-enroll"),
                        )
                        .clicked()
                    {
                        go = true;
                    }
                });
            if go {
                let code = Secret::new(self.reenroll_code.trim());
                self.launch(Op::Reenroll {
                    server: None,
                    code,
                    cert_sha256: None,
                });
                open = false;
            }
            if !open {
                self.reenroll_open = false;
            }
        }
    }

    /// Draws one frame.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if self.ctx.is_none() {
            self.ctx = Some(ui.ctx().clone());
        }
        self.poll();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Guardian Setup").strong().size(18.0));
                ui.label(env!("CARGO_PKG_VERSION"));
                if self.busy() {
                    ui.spinner();
                }
            });
            if !self.host.is_elevated() {
                amber(ui, NOT_ELEVATED);
            }
            if let Some(e) = self.error.clone() {
                ui.horizontal(|ui| {
                    red(ui, &e);
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            }
            ui.horizontal(|ui| {
                for (tab, name) in [
                    (Tab::Overview, "Overview"),
                    (Tab::Whitelist, "Whitelist"),
                    (Tab::Processes, "Processes"),
                ] {
                    if ui.selectable_label(self.tab == tab, name).clicked() && self.tab != tab {
                        self.tab = tab;
                        match tab {
                            Tab::Whitelist => self.launch(Op::WhitelistList),
                            Tab::Processes => self.launch(Op::Processes),
                            Tab::Overview => self.launch(Op::Status { check: false }),
                        }
                    }
                }
            });
            ui.separator();
            match self.tab {
                Tab::Overview => self.overview(ui),
                Tab::Whitelist => self.whitelist_tab(ui),
                Tab::Processes => self.processes_tab(ui),
            }
            if !self.progress.is_empty() {
                ui.separator();
                for p in self.progress.iter().rev().take(6).rev() {
                    ui.label(RichText::new(p.as_str()).weak());
                }
            }
        });
        self.dialogs(ui);
    }
}

impl eframe::App for SetupApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
