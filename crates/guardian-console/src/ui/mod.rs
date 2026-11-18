//! The egui shell. It draws only from backend [`Event`]s and sends only
//! [`Action`]s; no business logic lives here.

mod admin_computers;
mod admin_profiles;
mod admin_rooms;
mod admin_settings;
mod admin_users;
mod confirm;
mod connect_window;
mod fallback;
mod login;
mod rooms;
mod setup;
mod status;

use std::time::{Duration, Instant};

use egui::RichText;
use guardian_proto::v1::{EnrollCode, Profile, Role, Settings, User};

use crate::backend::{Action, Backend, ConnState, Event, FallbackState};
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

/// A destructive admin action waiting for "Yes".
#[derive(Debug, Clone, PartialEq, Eq)]
enum Confirm {
    DeleteRoom(i64, String),
    DeleteProfile(i64, String),
    Revoke(u64, String),
    DisableUser(i64, String),
    ChangeMode,
}

impl Confirm {
    fn question(&self) -> String {
        match self {
            Confirm::DeleteRoom(_, n) => format!("Delete {n}? Computers in it move to no room."),
            Confirm::DeleteProfile(_, n) => {
                format!("Delete the profile {n}? Rooms using it must be switched first.")
            }
            Confirm::Revoke(_, n) => {
                format!("Revoke {n}? It must be enrolled again with a new code.")
            }
            Confirm::DisableUser(_, n) => format!("Disable {n}? Their sessions end now."),
            Confirm::ChangeMode => "Change how this console connects? The embedded server \
                                    stops and you will need to sign in again."
                .into(),
        }
    }
}

/// State of the admin tabs.
#[derive(Debug)]
struct Admin {
    profiles: Vec<Profile>,
    users: Vec<User>,
    school: Option<Settings>,
    retention: String,
    code: Option<EnrollCode>,
    code_room: Option<i64>,
    new_room: String,
    rename_room: Option<(i64, String)>,
    edit: Option<Profile>,
    new_app: String,
    rename_pc: Option<(u64, String)>,
    new_login: String,
    new_password: String,
    new_role: Role,
    reset_pw: Option<(i64, String, String)>,
    confirm: Option<Confirm>,
}

impl Default for Admin {
    fn default() -> Self {
        Admin {
            profiles: Vec::new(),
            users: Vec::new(),
            school: None,
            retention: String::new(),
            code: None,
            code_room: None,
            new_room: String::new(),
            rename_room: None,
            edit: None,
            new_app: String::new(),
            rename_pc: None,
            new_login: String::new(),
            new_password: String::new(),
            new_role: Role::Teacher,
            reset_pw: None,
            confirm: None,
        }
    }
}

/// A button whose accessible name says which row it acts on.
fn row_button(ui: &mut egui::Ui, text: &str, a11y: &str) -> egui::Response {
    let r = ui.button(text);
    let name = a11y.to_string();
    r.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    r
}

/// A single-line field with a visible label.
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) -> egui::Response {
    let l = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(240.0),
    )
    .labelled_by(l.id)
}

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
    admin: Admin,
    /// The Connection info window is open.
    pub connect_open: bool,
    fb: fallback::FallbackUi,
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
            admin: Admin::default(),
            connect_open: false,
            fb: fallback::FallbackUi::default(),
        }
    }

    /// The backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    fn is_admin(&self) -> bool {
        !self.fb.is_active()
            && self
                .user
                .as_ref()
                .is_some_and(|u| u.role == Role::Admin as i32)
    }

    fn is_remote(&self) -> bool {
        matches!(self.mode, Some(Mode::Remote { .. }))
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
            Event::Fallback(st) => self.fb.set_state(st),
            Event::FallbackEnabled(id) => self.fb.enabled = id,
            Event::Done(m) => {
                if m.starts_with("Port ") && m.contains("open in the firewall") {
                    self.firewall_blocked = None;
                }
                self.done = Some((m, Instant::now()));
            }
            Event::Error(m) => self.error = Some(m),
            Event::Profiles(ps) => {
                if let Some(e) = &self.admin.edit {
                    let found = ps
                        .iter()
                        .find(|p| (e.id != 0 && p.id == e.id) || (e.id == 0 && p.name == e.name))
                        .cloned();
                    if found.is_some() {
                        self.admin.edit = found;
                    }
                }
                self.admin.profiles = ps;
            }
            Event::Users(us) => self.admin.users = us,
            Event::Settings(st) => {
                self.admin.retention = st.retention_days.to_string();
                self.admin.school = Some(st);
            }
            Event::Code(c) => self.admin.code = Some(c),
        }
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = self.error.clone()
            && status::banner(ui, &e, &["Dismiss"]).is_some()
        {
            self.error = None;
        }
        if self.fb.is_active() {
            if let Some(a) = self.fb.active_banner(ui) {
                self.backend.send(a);
            }
        } else if self.fb.is_available()
            && self.user.is_some()
            && status::banner(
                ui,
                "The server cannot be reached. You can work directly with the computers on this network.",
                &["Work locally (fallback mode)"],
            )
            .is_some()
        {
            self.backend.send(Action::EnterFallback);
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
        // Ways out that need no session: a typo in the address, a busy port, a
        // teacher who cannot open the admin Settings tab.
        ui.horizontal(|ui| {
            if let (EmbeddedState::Failed { .. }, Some(Mode::Embedded { port })) =
                (&self.embedded, &self.mode)
                && ui.button("Retry").clicked()
            {
                self.backend.send(Action::StartEmbedded {
                    data_dir: None,
                    port: *port,
                });
            }
            if ui.button("Change mode…").clicked() {
                self.admin.confirm = Some(Confirm::ChangeMode);
            }
            if self.fb.is_available() && ui.button("Work locally (fallback mode)…").clicked() {
                self.fb.login_open = true;
            }
        });
        if let Some(a) = self.fb.login_window(ui) {
            self.backend.send(a);
        }
        ui.add_space(6.0);
        if matches!(self.embedded, EmbeddedState::Failed { .. }) {
            self.admin_confirmation(ui);
            return;
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
        self.admin_confirmation(ui);
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
            if let FallbackState::Active { user, .. } = &self.fb.state {
                ui.label(format!("Signed in locally as {}", user.login));
            } else {
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
                if self.is_remote() && ui.button("Local fallback…").clicked() {
                    self.fb.window_open = true;
                }
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
                        if ui.selectable_label(self.tab == tab, name).clicked() && self.tab != tab {
                            self.tab = tab;
                            match tab {
                                Tab::Profiles => self.backend.send(Action::LoadProfiles),
                                Tab::Users => self.backend.send(Action::LoadUsers),
                                Tab::Settings => self.backend.send(Action::LoadSettings),
                                _ => {}
                            }
                        }
                    }
                });
                ui.separator();
            }
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Overview => {
                    let fleet = match &self.fb.state {
                        FallbackState::Active { fleet, .. } => fleet,
                        _ => &self.fleet,
                    };
                    let allow_free = !self.fb.is_active();
                    if let Some(c) = rooms::show(ui, fleet, self.is_admin(), allow_free) {
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
        self.admin_confirmation(ui);
        self.password_window(ui);
        self.connect_window(ui);
        self.rename_windows(ui);
        let admin = self
            .user
            .as_ref()
            .is_some_and(|u| u.role == Role::Admin as i32);
        if let Some(a) = self.fb.window(ui, admin) {
            self.backend.send(a);
        }
    }

    fn admin_tab(&mut self, ui: &mut egui::Ui, tab: Tab) {
        match tab {
            Tab::Overview => {}
            Tab::Rooms => self.rooms_tab(ui),
            Tab::Profiles => self.profiles_tab(ui),
            Tab::Computers => self.computers_tab(ui),
            Tab::Users => self.users_tab(ui),
            Tab::Settings => self.settings_tab(ui),
        }
    }

    /// Profiles known to the admin tabs (the fleet's until loaded).
    fn known_profiles(&self) -> Vec<Profile> {
        if self.admin.profiles.is_empty() {
            self.fleet.profiles().into_iter().cloned().collect()
        } else {
            let mut v = self.admin.profiles.clone();
            v.sort_by_key(|p| p.name.to_lowercase());
            v
        }
    }

    fn admin_confirmation(&mut self, ui: &mut egui::Ui) {
        let Some(c) = self.admin.confirm.clone() else {
            return;
        };
        match confirm::ask(ui, "admin-confirm", &c.question()) {
            Some(true) => {
                self.admin.confirm = None;
                match c {
                    Confirm::DeleteRoom(id, _) => self.backend.send(Action::DeleteRoom { id }),
                    Confirm::DeleteProfile(id, _) => {
                        self.admin.edit = None;
                        self.backend.send(Action::DeleteProfile { id });
                    }
                    Confirm::Revoke(id, _) => self.backend.send(Action::RevokeComputer { id }),
                    Confirm::DisableUser(id, _) => self.backend.send(Action::UpdateUser {
                        id,
                        role: None,
                        active: Some(false),
                        new_password: None,
                    }),
                    Confirm::ChangeMode => {
                        self.backend.send(Action::ForgetMode);
                        self.mode = None;
                        self.user = None;
                        self.fleet = Fleet::default();
                        self.embedded = EmbeddedState::Stopped;
                        self.conn = ConnState::Disconnected;
                        self.tab = Tab::Overview;
                        self.setup = setup::Setup::default();
                    }
                }
            }
            Some(false) => self.admin.confirm = None,
            None => {}
        }
    }

    /// Draws one frame.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        for e in self.backend.poll() {
            self.handle(e);
        }
        if self.mode.is_none() {
            egui::CentralPanel::default().show(ui, |ui| self.setup_screen(ui));
        } else if self.fb.is_active() {
            self.tab = Tab::Overview;
            self.main_screen(ui);
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
