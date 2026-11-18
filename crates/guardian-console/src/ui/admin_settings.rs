//! Admin: school settings, and the console's own mode.

use guardian_proto::v1::Settings;

use super::{Confirm, ConsoleApp, field};
use crate::backend::{Action, Backend};
use crate::embedded::EmbeddedState;
use crate::settings::Mode;

impl<B: Backend> ConsoleApp<B> {
    pub(super) fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.strong("School");
        if let Some(mut st) = self.admin.school {
            let profiles = self.known_profiles();
            let current = profiles
                .iter()
                .find(|p| p.id == st.default_profile_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "?".into());
            egui::ComboBox::new("default-profile", "Default profile")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for p in &profiles {
                        if ui
                            .selectable_label(p.id == st.default_profile_id, &p.name)
                            .clicked()
                        {
                            st.default_profile_id = p.id;
                        }
                    }
                });
            ui.label("Computers that are not in a room follow the default profile.");
            field(ui, "Retention (days)", &mut self.admin.retention, false);
            let days = self.admin.retention.trim().parse::<u32>().ok();
            match days {
                Some(d) if (1..=3650).contains(&d) => {
                    if ui.button("Save settings").clicked() {
                        self.backend.send(Action::SaveSettings(Settings {
                            default_profile_id: st.default_profile_id,
                            retention_days: d,
                        }));
                    }
                }
                _ => {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xb3, 0x26, 0x1e),
                        "Retention is a number of days from 1 to 3650.",
                    );
                }
            }
            self.admin.school = Some(st);
        } else {
            ui.label("Loading settings…");
        }
        ui.separator();
        ui.strong("This console");
        match &self.mode {
            Some(Mode::Remote {
                server,
                cert_sha256,
            }) => {
                ui.label(format!("Connected to a server: {server}"));
                if let Some(p) = cert_sha256 {
                    ui.label(format!("Certificate SHA-256: {p}"));
                }
            }
            Some(Mode::Embedded { port }) => {
                ui.label(format!("The server runs on this computer, port {port}."));
                if let EmbeddedState::Running { cert_sha256, .. } = &self.embedded {
                    ui.horizontal(|ui| {
                        ui.label(format!("Certificate SHA-256: {cert_sha256}"));
                        if ui.button("Copy fingerprint").clicked() {
                            ui.ctx().copy_text(cert_sha256.clone());
                        }
                    });
                }
                ui.label(format!(
                    "Data: {}",
                    crate::settings::embedded_data_dir().display()
                ));
            }
            None => {}
        }
        if ui.button("Change mode…").clicked() {
            self.admin.confirm = Some(Confirm::ChangeMode);
        }
    }
}
