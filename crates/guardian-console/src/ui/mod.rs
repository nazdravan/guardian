//! The egui shell. It draws only from backend [`Event`]s and sends only
//! [`Action`]s; no business logic lives here.

mod confirm;
mod login;
mod rooms;
mod setup;
mod status;

use std::time::{Duration, Instant};

use egui::RichText;
use guardian_proto::v1::{Role, User};

use crate::backend::{Action, Backend, ConnState, Event};
use crate::connect_info;
use crate::embedded::EmbeddedState;
use crate::fleet::Fleet;
use crate::settings::{ConsoleSettings, Mode, Secret};

pub use rooms::ProfileChoice;

/// Which page of the main screen is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Rooms with their profile switches (everyone).
    Overview,
    /// Admin: rooms.
    Rooms,
    /// Admin: profiles.
    Profiles,
    /// Admin: computers.
    Computers,
    /// Admin: users.
    Users,
    /// Admin: school settings and the console's mode.
    Settings,
}

const ADMIN_TABS: [(Tab, &str); 5] = [
    (Tab::Rooms, "Rooms"),
    (Tab::Profiles, "Profiles"),
    (Tab::Computers, "Computers"),
    (Tab::Users, "Users"),
    (Tab::Settings, "Settings"),
];

#[derive(Debug, Default)]
struct PasswordForm {
    open: bool,
    old: String,
    new: String,
    repeat: String,
    error: Option<String>,
}

/// The console window.
pub struct ConsoleApp<B: Backend> {
    backend: B,
    mode: Option<Mode>,
    conn: ConnState,
    user: Option<User>,
    fleet: Fleet,
    embedded: EmbeddedState,
    firewall_blocked: Option<u16>,
    error: Option<String>,
    done: Option<(String, Instant)>,
    logout_reason: Option<String>,
    ever_online: bool,
    setup: setup::Setup,
    login: login::Login,
    pending_profile: Option<ProfileChoice>,
    password: PasswordForm,
    tab: Tab,
    /// The Connection info window is open.
    pub connect_open: bool,
}

impl<B: Backend> std::fmt::Debug for ConsoleApp<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsoleApp")
            .field("conn", &self.conn)
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

fn role_name(u: &User) -> &'static str {
    match Role::try_from(u.role) {
        Ok(Role::Admin) => "administrator",
        Ok(Role::Teacher) => "teacher",
        _ => "user",
    }
}

fn red(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(egui::Color32::from_rgb(0xb3, 0x26, 0x1e), text);
}

impl<B: Backend> ConsoleApp<B> {
    /// A window over `backend`, starting from the saved settings.
    pub fn new(backend: B, settings: ConsoleSettings) -> Self {
        ConsoleApp {
            backend,
            mode: settings.mode.clone(),
            conn: if settings.mode.is_some() {
                ConnState::Connecting
            } else {
                ConnState::Disconnected
            },
            user: None,
            fleet: Fleet::default(),
            embedded: EmbeddedState::Stopped,
            firewall_blocked: None,
            error: None,
            done: None,
            logout_reason: None,
            ever_online: false,
            setup: setup::Setup::default(),
            login: login::Login::with_login(settings.login.clone()),
            pending_profile: None,
            password: PasswordForm::default(),
            tab: Tab::Overview,
            connect_open: false,
        }
    }

    /// The backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    fn is_admin(&self) -> bool {
        self.user
            .as_ref()
            .is_some_and(|u| u.role == Role::Admin as i32)
    }

    fn handle(&mut self, e: Event) {
        match e {
            Event::Connection(c) => {
                if matches!(c, ConnState::NeedsLogin { .. }) {
                    self.user = None;
                }
                self.conn = c;
            }
            Event::LoggedIn(u) => {
                self.user = Some(u);
                self.logout_reason = None;
                if !self.is_admin() {
                    self.tab = Tab::Overview;
                }
            }
            Event::LoggedOut { reason } => {
                self.user = None;
                self.fleet = Fleet::default();
                self.logout_reason = reason;
                self.tab = Tab::Overview;
            }
            Event::Fleet(f) => {
                self.ever_online |= f.any_online();
                self.fleet = f;
            }
            Event::Embedded(s) => {
                if !matches!(s, EmbeddedState::Running { .. }) {
                    self.firewall_blocked = None;
                }
                self.embedded = s;
            }
            Event::FirewallBlocked(p) => self.firewall_blocked = Some(p),
            Event::Done(m) => {
                if m.starts_with("Port ") && m.contains("open in the firewall") {
                    self.firewall_blocked = None;
                }
                self.done = Some((m, Instant::now()));
            }
            Event::Error(m) => self.error = Some(m),
            Event::Profiles(_) | Event::Users(_) | Event::Settings(_) | Event::Code(_) => {}
        }
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = self.error.clone()
            && status::banner(ui, &e, &["Dismiss"]).is_some()
        {
            self.error = None;
        }
        if let Some(port) = self.firewall_blocked
            && status::banner(
                ui,
                &format!(
                    "Student computers cannot reach this server until port {port} is open in \
                     the Windows firewall."
                ),
                &["Open firewall port…"],
            )
            .is_some()
        {
            self.backend.send(Action::OpenFirewall);
        }
        if connect_info::no_agent_warning(&self.embedded, self.ever_online, Instant::now()) {
            let (name, _) = connect_info::this_computer();
            if status::banner(
                ui,
                &format!(
                    "No computer has connected yet. Agents may not find {name} on the network."
                ),
                &["Connection info…"],
            )
            .is_some()
            {
                self.connect_open = true;
            }
        }
    }

    fn setup_screen(&mut self, ui: &mut egui::Ui) {
        if let Some((mode, action)) = self.setup.show(ui) {
            self.mode = Some(mode);
            self.conn = ConnState::Connecting;
            self.backend.send(action);
        }
    }

    fn sign_in_screen(&mut self, ui: &mut egui::Ui) {
        status::card(ui, self.mode.as_ref(), &self.conn, &self.embedded);
        ui.add_space(10.0);
        self.banners(ui);
        if let Some(r) = &self.logout_reason {
            ui.label(r.as_str());
        }
        match self.conn {
            ConnState::NeedsLogin { can_init } => {
                if let Some(a) = self.login.show(ui, can_init) {
                    self.backend.send(a);
                }
            }
            ConnState::Connecting | ConnState::Disconnected => {
                ui.label("Connecting…");
            }
            ConnState::Reconnecting { .. } | ConnState::Online => {}
        }
    }

    fn password_window(&mut self, ui: &mut egui::Ui) {
        if !self.password.open {
            return;
        }
        let mut open = true;
        let mut send = None;
        egui::Window::new("Change password")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                let f = &mut self.password;
                for (label, value) in [
                    ("Current password", &mut f.old),
                    ("New password", &mut f.new),
                    ("Repeat the new password", &mut f.repeat),
                ] {
                    let l = ui.label(label);
                    ui.add(egui::TextEdit::singleline(value).password(true))
                        .labelled_by(l.id);
                }
                if ui.button("Change").clicked() {
                    if f.new.chars().count() < login::MIN_PASSWORD {
                        f.error = Some(format!(
                            "The password needs at least {} characters.",
                            login::MIN_PASSWORD
                        ));
                    } else if f.new != f.repeat {
                        f.error = Some("The passwords do not match.".into());
                    } else {
                        send = Some(Action::ChangePassword {
                            old: Secret::new(std::mem::take(&mut f.old)),
                            new: Secret::new(std::mem::take(&mut f.new)),
                        });
                        f.repeat.clear();
                        f.error = None;
                    }
                }
                if let Some(e) = &f.error {
                    red(ui, e);
                }
            });
        if let Some(a) = send {
            self.backend.send(a);
            open = false;
        }
        if !open {
            self.password = PasswordForm::default();
        }
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(u) = &self.user {
                ui.label(format!("Signed in as {} ({})", u.login, role_name(u)));
            }
            if ui.button("Change password…").clicked() {
                self.password.open = true;
            }
            if ui.button("Sign out").clicked() {
                self.backend.send(Action::Logout);
            }
            if ui.button("Connection info…").clicked() {
                self.connect_open = true;
            }
            if let Some((m, at)) = &self.done
                && at.elapsed() < Duration::from_secs(6)
            {
                ui.label(
                    RichText::new(m.as_str()).color(egui::Color32::from_rgb(0x1f, 0x7a, 0x3d)),
                );
            }
        });
    }

    fn profile_confirmation(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.pending_profile.clone() else {
            return;
        };
        let q = format!(
            "All {} computers in {} will run anything. Continue?",
            p.computers, p.room_name
        );
        match confirm::ask(ui, "free-profile", &q) {
            Some(true) => {
                self.backend.send(Action::SetRoomProfile {
                    room_id: p.room_id,
                    profile_id: p.profile_id,
                });
                self.pending_profile = None;
            }
            Some(false) => self.pending_profile = None,
            None => {}
        }
    }

    fn main_screen(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("bottom").show(ui, |ui| {
            ui.add_space(4.0);
            self.bottom_bar(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            status::card(ui, self.mode.as_ref(), &self.conn, &self.embedded);
            ui.add_space(8.0);
            self.banners(ui);
            if self.is_admin() {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.tab == Tab::Overview, "Overview")
                        .clicked()
                    {
                        self.tab = Tab::Overview;
                    }
                    for (tab, name) in ADMIN_TABS {
                        if ui.selectable_label(self.tab == tab, name).clicked() {
                            self.tab = tab;
                        }
                    }
                });
                ui.separator();
            }
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Overview => {
                    if let Some(c) = rooms::show(ui, &self.fleet, self.is_admin()) {
                        if c.free {
                            self.pending_profile = Some(c);
                        } else {
                            self.backend.send(Action::SetRoomProfile {
                                room_id: c.room_id,
                                profile_id: c.profile_id,
                            });
                        }
                    }
                }
                other => self.admin_tab(ui, other),
            });
        });
        self.profile_confirmation(ui);
        self.password_window(ui);
    }

    /// Placeholder until the admin tabs are drawn.
    fn admin_tab(&mut self, ui: &mut egui::Ui, _tab: Tab) {
        ui.label("…");
    }

    /// Draws one frame.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        for e in self.backend.poll() {
            self.handle(e);
        }
        if self.mode.is_none() {
            egui::CentralPanel::default().show(ui, |ui| self.setup_screen(ui));
        } else if self.user.is_none() {
            egui::CentralPanel::default().show(ui, |ui| self.sign_in_screen(ui));
        } else {
            self.main_screen(ui);
        }
        // Offline ages and the no-agent hint change with time.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }
}

impl<B: Backend> eframe::App for ConsoleApp<B> {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
