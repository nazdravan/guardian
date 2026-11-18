//! Sign in, or create the first administrator.

use crate::backend::Action;
use crate::settings::Secret;

/// Minimum password length checked before sending.
pub const MIN_PASSWORD: usize = 8;

/// The sign-in form.
#[derive(Debug, Default)]
pub struct Login {
    login: String,
    password: String,
    repeat: String,
    error: Option<String>,
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) -> egui::Response {
    let l = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(280.0),
    )
    .labelled_by(l.id)
}

impl Login {
    /// Pre-fills the last login.
    pub fn with_login(login: Option<String>) -> Login {
        Login {
            login: login.unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Draws the form; returns the action to send.
    pub fn show(&mut self, ui: &mut egui::Ui, can_init: bool) -> Option<Action> {
        let mut out = None;
        if can_init {
            ui.heading("Create the administrator account");
            ui.label("This is a new server. The first account can manage everything.");
        } else {
            ui.heading("Sign in to Guardian");
        }
        ui.add_space(8.0);
        field(ui, "Login", &mut self.login, false);
        let pw = field(ui, "Password", &mut self.password, true);
        let mut enter = pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if can_init {
            let r = field(ui, "Repeat the password", &mut self.repeat, true);
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        ui.add_space(6.0);
        let label = if can_init {
            "Create account"
        } else {
            "Sign in"
        };
        if ui.button(label).clicked() || enter {
            let login = self.login.trim().to_string();
            if login.is_empty() {
                self.error = Some("Enter the login.".into());
            } else if can_init && self.password.chars().count() < MIN_PASSWORD {
                self.error = Some(format!(
                    "The password needs at least {MIN_PASSWORD} characters."
                ));
            } else if can_init && self.password != self.repeat {
                self.error = Some("The passwords do not match.".into());
            } else if self.password.is_empty() {
                self.error = Some("Enter the password.".into());
            } else {
                self.error = None;
                let password = Secret::new(std::mem::take(&mut self.password));
                self.repeat.clear();
                out = Some(if can_init {
                    Action::Init {
                        bootstrap_code: String::new(),
                        login,
                        password,
                    }
                } else {
                    Action::Login { login, password }
                });
            }
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(0xb3, 0x26, 0x1e), e);
        }
        out
    }
}
