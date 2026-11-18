//! Admin: profiles (mode and application list).

use guardian_proto::v1::{Mode as PMode, Profile};

use super::{Confirm, ConsoleApp, field, row_button};
use crate::backend::{Action, Backend};

const MODES: [(PMode, &str); 3] = [
    (PMode::Free, "Nothing is blocked"),
    (PMode::Blacklist, "Block the listed apps"),
    (PMode::Whitelist, "Allow only the listed apps"),
];

impl<B: Backend> ConsoleApp<B> {
    pub(super) fn profiles_tab(&mut self, ui: &mut egui::Ui) {
        let profiles = self.known_profiles();
        ui.horizontal(|ui| {
            let current = self
                .admin
                .edit
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "Choose…".into());
            egui::ComboBox::new("profile-to-edit", "Profile to edit")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for p in &profiles {
                        let sel = self.admin.edit.as_ref().is_some_and(|e| e.id == p.id);
                        if ui.selectable_label(sel, &p.name).clicked() {
                            self.admin.edit = Some(p.clone());
                        }
                    }
                });
            if ui.button("New profile").clicked() {
                self.admin.edit = Some(Profile {
                    id: 0,
                    name: "New profile".into(),
                    mode: PMode::Blacklist as i32,
                    apps: vec![],
                });
            }
        });
        ui.separator();
        let Some(mut p) = self.admin.edit.take() else {
            ui.label("Choose a profile to edit, or create a new one.");
            return;
        };
        let mut keep = true;
        field(ui, "Profile name", &mut p.name, false);
        ui.horizontal(|ui| {
            for (mode, text) in MODES {
                if ui.selectable_label(p.mode == mode as i32, text).clicked() {
                    p.mode = mode as i32;
                }
            }
        });
        if p.mode != PMode::Free as i32 {
            ui.add_space(6.0);
            let mut remove = None;
            egui::Grid::new("profile-apps")
                .striped(true)
                .show(ui, |ui| {
                    for (i, app) in p.apps.iter().enumerate() {
                        ui.label(app);
                        if row_button(ui, "Remove", &format!("Remove {app}")).clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                p.apps.remove(i);
            }
            if p.apps.is_empty() {
                ui.label("No applications yet.");
            }
            ui.horizontal(|ui| {
                field(ui, "Application name", &mut self.admin.new_app, false);
                if ui.button("Add application").clicked() {
                    let a = self.admin.new_app.trim().to_string();
                    if !a.is_empty() && !p.apps.iter().any(|x| x.eq_ignore_ascii_case(&a)) {
                        p.apps.push(a);
                    }
                    self.admin.new_app.clear();
                }
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Save profile").clicked() {
                self.backend.send(Action::SaveProfile(p.clone()));
            }
            if p.id != 0 && ui.button("Delete profile").clicked() {
                self.admin.confirm = Some(Confirm::DeleteProfile(p.id, p.name.clone()));
            }
            if ui.button("Close").clicked() {
                keep = false;
            }
        });
        if keep {
            self.admin.edit = Some(p);
        }
    }
}
