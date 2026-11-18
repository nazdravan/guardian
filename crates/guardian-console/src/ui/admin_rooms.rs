//! Admin: rooms.

use super::{Confirm, ConsoleApp, field, row_button};
use crate::backend::{Action, Backend};

impl<B: Backend> ConsoleApp<B> {
    pub(super) fn rooms_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            field(ui, "New room", &mut self.admin.new_room, false);
            if ui.button("Add room").clicked() {
                let name = self.admin.new_room.trim().to_string();
                if !name.is_empty() {
                    self.backend.send(Action::CreateRoom { name });
                    self.admin.new_room.clear();
                }
            }
        });
        ui.add_space(8.0);
        let rooms = self.fleet.rooms();
        if rooms.is_empty() {
            ui.label("No rooms yet.");
        }
        egui::Grid::new("admin-rooms")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for r in rooms {
                    ui.label(&r.name);
                    ui.label(format!("{} computers", r.computers.len()));
                    if row_button(ui, "Rename…", &format!("Rename {}", r.name)).clicked() {
                        self.admin.rename_room = Some((r.id, r.name.clone()));
                    }
                    if row_button(ui, "Delete", &format!("Delete {}", r.name)).clicked() {
                        self.admin.confirm = Some(Confirm::DeleteRoom(r.id, r.name.clone()));
                    }
                    ui.end_row();
                }
            });
    }

    /// The small rename windows of the Rooms and Computers tabs.
    pub(super) fn rename_windows(&mut self, ui: &mut egui::Ui) {
        if let Some((id, mut name)) = self.admin.rename_room.take() {
            let mut open = true;
            let mut send = false;
            egui::Window::new("Rename room")
                .collapsible(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    field(ui, "New room name", &mut name, false);
                    send = ui.button("Rename").clicked();
                });
            if send && !name.trim().is_empty() {
                self.backend.send(Action::RenameRoom {
                    id,
                    name: name.trim().to_string(),
                });
            } else if open {
                self.admin.rename_room = Some((id, name));
            }
        }
        if let Some((id, mut name)) = self.admin.rename_pc.take() {
            let mut open = true;
            let mut send = false;
            egui::Window::new("Rename computer")
                .collapsible(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    field(ui, "New computer name", &mut name, false);
                    send = ui.button("Rename").clicked();
                });
            if send && !name.trim().is_empty() {
                self.backend.send(Action::UpdateComputer {
                    id,
                    name: Some(name.trim().to_string()),
                    room_id: None,
                    exception_profile_id: None,
                });
            } else if open {
                self.admin.rename_pc = Some((id, name));
            }
        }
    }
}
