//! Admin: computers (room, exception, rename, revoke).

use super::{Confirm, ConsoleApp, row_button};
use crate::backend::{Action, Backend};

/// A combo box with an accessible name and no visible label (the column says
/// what it is).
fn combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    a11y: &str,
    current: String,
    items: &[(i64, String)],
) -> Option<i64> {
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (id, name) in items {
                if ui.selectable_label(false, name).clicked() {
                    picked = Some(*id);
                }
            }
        });
    let name = a11y.to_string();
    r.response
        .widget_info(move || egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
    picked
}

impl<B: Backend> ConsoleApp<B> {
    pub(super) fn computers_tab(&mut self, ui: &mut egui::Ui) {
        let computers = self.fleet.computers();
        if computers.is_empty() {
            ui.label("No computers yet — use Connection info… to enroll them.");
            return;
        }
        let mut rooms: Vec<(i64, String)> = vec![(0, "— no room —".into())];
        rooms.extend(self.fleet.rooms().into_iter().map(|r| (r.id, r.name)));
        let mut exceptions: Vec<(i64, String)> = vec![(0, "— none —".into())];
        exceptions.extend(
            self.known_profiles()
                .into_iter()
                .map(|p| (p.id, p.name.clone())),
        );
        let name_of = |list: &[(i64, String)], id: i64| {
            list.iter()
                .find(|(i, _)| *i == id)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "?".into())
        };
        egui::Grid::new("admin-computers")
            .striped(true)
            .num_columns(7)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for h in [
                    "Computer",
                    "Room",
                    "Exception",
                    "Connection",
                    "System",
                    "",
                    "",
                ] {
                    ui.strong(h);
                }
                ui.end_row();
                for c in computers {
                    ui.label(&c.name);
                    if let Some(room) = combo(
                        ui,
                        ("room-of", c.id),
                        &format!("Room of {}", c.name),
                        name_of(&rooms, c.room_id),
                        &rooms,
                    ) && room != c.room_id
                    {
                        self.backend.send(Action::UpdateComputer {
                            id: c.id,
                            name: None,
                            room_id: Some(room),
                            exception_profile_id: None,
                        });
                    }
                    if let Some(p) = combo(
                        ui,
                        ("exception-of", c.id),
                        &format!("Exception of {}", c.name),
                        name_of(&exceptions, c.exception_profile_id),
                        &exceptions,
                    ) && p != c.exception_profile_id
                    {
                        self.backend.send(Action::UpdateComputer {
                            id: c.id,
                            name: None,
                            room_id: None,
                            exception_profile_id: Some(p),
                        });
                    }
                    ui.label(&c.connection);
                    ui.label(format!("{} {}", c.os, c.agent_version).trim().to_string());
                    if row_button(ui, "Rename…", &format!("Rename {}", c.name)).clicked() {
                        self.admin.rename_pc = Some((c.id, c.name.clone()));
                    }
                    if row_button(ui, "Revoke…", &format!("Revoke {}", c.name)).clicked() {
                        self.admin.confirm = Some(Confirm::Revoke(c.id, c.name.clone()));
                    }
                    ui.end_row();
                }
            });
    }
}
