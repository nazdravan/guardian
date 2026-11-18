//! Local fallback mode on screen: the "Local fallback" window (enrolling this
//! console), the local sign-in, and the fallback banner.

use crate::backend::{Action, FallbackState};
use crate::settings::Secret;

use super::status;

/// The fallback part of the window state.
#[derive(Debug)]
pub struct FallbackUi {
    /// As the backend reports it.
    pub state: FallbackState,
    /// This console's id when enrolled.
    pub enabled: Option<i64>,
    /// The "Local fallback" window is open.
    pub window_open: bool,
    code: String,
    /// The local sign-in window is open.
    pub login_open: bool,
    login: String,
    password: String,
}

impl Default for FallbackUi {
    fn default() -> FallbackUi {
        FallbackUi {
            state: FallbackState::Off,
            enabled: None,
            window_open: false,
            code: String::new(),
            login_open: false,
            login: String::new(),
            password: String::new(),
        }
    }
}

/// The banner while working locally.
pub const ACTIVE_TEXT: &str = "Fallback mode: the server cannot be reached. Changes go straight to the computers on this network and are undone when the server is back.";

impl FallbackUi {
    /// Working locally now.
    pub fn is_active(&self) -> bool {
        matches!(self.state, FallbackState::Active { .. })
    }

    /// The server is down and someone may work locally.
    pub fn is_available(&self) -> bool {
        matches!(self.state, FallbackState::Available)
    }

    /// New state from the backend.
    pub fn set_state(&mut self, s: FallbackState) {
        if !matches!(s, FallbackState::Available) {
            self.login_open = false;
            self.password.clear();
        }
        self.state = s;
    }

    /// The "Local fallback" window.
    pub fn window(&mut self, ui: &mut egui::Ui, admin: bool) -> Option<Action> {
        if !self.window_open {
            return None;
        }
        let mut open = true;
        let mut out = None;
        egui::Window::new("Local fallback")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| match self.enabled {
                Some(id) => {
                    ui.label(format!(
                        "Enabled (console #{id}). Users who have signed in here can work locally when the server is down."
                    ));
                }
                None => {
                    ui.label("Not enabled on this computer.");
                    let l = ui.label("Console code");
                    ui.add(egui::TextEdit::singleline(&mut self.code))
                        .labelled_by(l.id);
                    ui.horizontal(|ui| {
                        if ui.button("Enable").clicked() && !self.code.trim().is_empty() {
                            out = Some(Action::EnableFallback {
                                code: Some(self.code.trim().to_string()),
                            });
                        }
                        if admin && ui.button("Enable (create a code)").clicked() {
                            out = Some(Action::EnableFallback { code: None });
                        }
                    });
                }
            });
        if !open {
            self.window_open = false;
        }
        out
    }

    /// The local sign-in window.
    pub fn login_window(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.login_open {
            return None;
        }
        let mut open = true;
        let mut out = None;
        egui::Window::new("Sign in to fallback mode")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label("The server cannot be reached. Sign in to work with the computers on this network.");
                let l = ui.label("Login");
                ui.add(egui::TextEdit::singleline(&mut self.login))
                    .labelled_by(l.id);
                let p = ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true))
                    .labelled_by(p.id);
                if ui.button("Sign in").clicked() && !self.login.trim().is_empty() {
                    out = Some(Action::FallbackLogin {
                        login: self.login.trim().to_string(),
                        password: Secret::new(std::mem::take(&mut self.password)),
                    });
                }
            });
        if !open {
            self.login_open = false;
            self.password.clear();
        }
        out
    }

    /// The amber banner while working locally; `Some` when Leave was clicked.
    pub fn active_banner(&self, ui: &mut egui::Ui) -> Option<Action> {
        status::banner(ui, ACTIVE_TEXT, &["Leave fallback mode"]).map(|_| Action::LeaveFallback)
    }
}
