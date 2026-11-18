//! First run: connect to a server, or run one on this computer.

use crate::backend::Action;
use crate::settings::{DEFAULT_PORT, Mode, normalize_pin, validate_server_url};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Remote,
    Embedded,
}

/// The setup form.
#[derive(Debug)]
pub struct Setup {
    choice: Option<Choice>,
    server: String,
    pin: String,
    port: String,
    error: Option<String>,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            choice: None,
            server: String::new(),
            pin: String::new(),
            port: DEFAULT_PORT.to_string(),
            error: None,
        }
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    let l = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(360.0))
        .labelled_by(l.id)
}

impl Setup {
    /// Draws the form; returns the chosen mode and the action to send.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<(Mode, Action)> {
        let mut out = None;
        ui.heading("Welcome to Guardian Console");
        ui.label("How should this console reach the Guardian server?");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.choice == Some(Choice::Remote), "Connect to a server")
                .clicked()
            {
                self.choice = Some(Choice::Remote);
                self.error = None;
            }
            if ui
                .selectable_label(
                    self.choice == Some(Choice::Embedded),
                    "Run the server on this computer",
                )
                .clicked()
            {
                self.choice = Some(Choice::Embedded);
                self.error = None;
            }
        });
        ui.add_space(10.0);
        match self.choice {
            None => {}
            Some(Choice::Remote) => {
                ui.label("The server runs elsewhere (school server or cloud).");
                let r1 = field(ui, "Server address", &mut self.server);
                field(ui, "Certificate SHA-256 (optional)", &mut self.pin);
                let enter = r1.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Connect").clicked() || enter {
                    match (validate_server_url(&self.server), normalize_pin(&self.pin)) {
                        (Ok(server), Ok(pin)) => {
                            self.error = None;
                            out = Some((
                                Mode::Remote {
                                    server: server.clone(),
                                    cert_sha256: pin.clone(),
                                },
                                Action::Connect {
                                    server,
                                    cert_sha256: pin,
                                },
                            ));
                        }
                        (Err(e), _) | (_, Err(e)) => self.error = Some(e),
                    }
                }
            }
            Some(Choice::Embedded) => {
                ui.label(
                    "The server runs inside this console while it is open. \
                     Student computers connect to this PC by its name.",
                );
                field(ui, "Port", &mut self.port);
                if ui.button("Start the server").clicked() {
                    match self.port.trim().parse::<u16>() {
                        Ok(port) if port != 0 => {
                            self.error = None;
                            out = Some((
                                Mode::Embedded { port },
                                Action::StartEmbedded {
                                    data_dir: None,
                                    port,
                                },
                            ));
                        }
                        _ => self.error = Some("The port is a number from 1 to 65535.".into()),
                    }
                }
            }
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(0xb3, 0x26, 0x1e), e);
        }
        out
    }
}
