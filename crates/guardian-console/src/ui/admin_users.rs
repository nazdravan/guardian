//! Admin: users (role, rooms, enable/disable, password).

use guardian_proto::v1::Role;

use super::{Confirm, ConsoleApp, field, row_button};
use crate::backend::{Action, Backend};
use crate::settings::Secret;

fn role_text(r: Role) -> &'static str {
    match r {
        Role::Admin => "Administrator",
        _ => "Teacher",
    }
}

impl<B: Backend> ConsoleApp<B> {
    pub(super) fn users_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            field(ui, "New login", &mut self.admin.new_login, false);
            field(ui, "New password", &mut self.admin.new_password, true);
            egui::ComboBox::new("new-user-role", "Role")
                .selected_text(role_text(self.admin.new_role))
                .show_ui(ui, |ui| {
                    for r in [Role::Teacher, Role::Admin] {
                        if ui
                            .selectable_label(self.admin.new_role == r, role_text(r))
                            .clicked()
                        {
                            self.admin.new_role = r;
                        }
                    }
                });
            if ui.button("Add user").clicked() {
                let login = self.admin.new_login.trim().to_string();
                if !login.is_empty() && !self.admin.new_password.is_empty() {
                    self.backend.send(Action::CreateUser {
                        login,
                        password: Secret::new(std::mem::take(&mut self.admin.new_password)),
                        role: self.admin.new_role,
                    });
                    self.admin.new_login.clear();
                }
            }
        });
        ui.add_space(8.0);
        let rooms = self.fleet.rooms();
        let users = self.admin.users.clone();
        if users.is_empty() {
            ui.label("Loading users…");
        }
        egui::Grid::new("admin-users")
            .striped(true)
            .num_columns(5)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for u in users {
                    let role = Role::try_from(u.role).unwrap_or(Role::Teacher);
                    ui.label(&u.login);
                    let r = egui::ComboBox::from_id_salt(("role-of", u.id))
                        .selected_text(role_text(role))
                        .show_ui(ui, |ui| {
                            for r in [Role::Teacher, Role::Admin] {
                                if ui.selectable_label(r == role, role_text(r)).clicked()
                                    && r != role
                                {
                                    self.backend.send(Action::UpdateUser {
                                        id: u.id,
                                        role: Some(r),
                                        active: None,
                                        new_password: None,
                                    });
                                }
                            }
                        });
                    let name = format!("Role of {}", u.login);
                    r.response.widget_info(move || {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name)
                    });
                    ui.horizontal(|ui| {
                        for room in &rooms {
                            let mut on = u.room_ids.contains(&room.id);
                            if ui.checkbox(&mut on, &room.name).changed() {
                                let mut ids: Vec<i64> = u
                                    .room_ids
                                    .iter()
                                    .copied()
                                    .filter(|i| *i != room.id)
                                    .collect();
                                if on {
                                    ids.push(room.id);
                                }
                                self.backend.send(Action::SetUserRooms {
                                    user_id: u.id,
                                    room_ids: ids,
                                });
                            }
                        }
                    });
                    if u.active {
                        if row_button(ui, "Disable", &format!("Disable {}", u.login)).clicked() {
                            self.admin.confirm = Some(Confirm::DisableUser(u.id, u.login.clone()));
                        }
                    } else if row_button(ui, "Enable", &format!("Enable {}", u.login)).clicked() {
                        self.backend.send(Action::UpdateUser {
                            id: u.id,
                            role: None,
                            active: Some(true),
                            new_password: None,
                        });
                    }
                    if row_button(
                        ui,
                        "Reset password…",
                        &format!("Reset password of {}", u.login),
                    )
                    .clicked()
                    {
                        self.admin.reset_pw = Some((u.id, u.login.clone(), String::new()));
                    }
                    ui.end_row();
                }
            });
        if let Some((id, login, mut pw)) = self.admin.reset_pw.take() {
            let mut open = true;
            let mut send = false;
            egui::Window::new("Reset password")
                .collapsible(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    field(ui, &format!("New password for {login}"), &mut pw, true);
                    send = ui.button("Set password").clicked();
                });
            if send && !pw.is_empty() {
                self.backend.send(Action::UpdateUser {
                    id,
                    role: None,
                    active: None,
                    new_password: Some(Secret::new(pw)),
                });
            } else if open {
                self.admin.reset_pw = Some((id, login, pw));
            }
        }
    }
}
