//! "Connection info": the enrollment code and the exact commands for agents.

use egui::RichText;

use super::ConsoleApp;
use crate::backend::{Action, Backend};
use crate::connect_info::{self, agent_target, lan_fallback, linux_command, windows_command};
use crate::embedded::EmbeddedState;
use crate::settings::Mode;

impl<B: Backend> ConsoleApp<B> {
    pub(super) fn connect_window(&mut self, ui: &mut egui::Ui) {
        if !self.connect_open {
            return;
        }
        let Some(mode) = self.mode.clone() else {
            self.connect_open = false;
            return;
        };
        let (hostname, addrs) = connect_info::this_computer();
        let mut target = agent_target(&mode, &hostname, None);
        if let (Mode::Embedded { .. }, EmbeddedState::Running { cert_sha256, .. }) =
            (&mode, &self.embedded)
        {
            target.cert_sha256 = Some(cert_sha256.clone());
        }
        let rooms = self.fleet.rooms();
        let room = self
            .admin
            .code_room
            .or_else(|| rooms.first().map(|r| r.id))
            .unwrap_or(0);
        let mut open = true;
        egui::Window::new("Connection info")
            .collapsible(false)
            .default_width(620.0)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(format!("Agents connect to {}", target.server));
                if let Some(p) = &target.cert_sha256 {
                    ui.label(format!("Certificate SHA-256: {p}"));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let name = if room == 0 {
                        "One computer (15 min)".to_string()
                    } else {
                        rooms
                            .iter()
                            .find(|r| r.id == room)
                            .map(|r| r.name.clone())
                            .unwrap_or_default()
                    };
                    egui::ComboBox::new("code-room", "Room for the code")
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            for r in &rooms {
                                if ui.selectable_label(r.id == room, &r.name).clicked() {
                                    self.admin.code_room = Some(r.id);
                                }
                            }
                            if ui
                                .selectable_label(room == 0, "One computer (15 min)")
                                .clicked()
                            {
                                self.admin.code_room = Some(0);
                            }
                        });
                    if ui.button("New code").clicked() {
                        self.admin.code = None;
                        self.backend.send(if room == 0 {
                            Action::CreateAgentCode {
                                room_id: 0,
                                max_uses: 1,
                                ttl_seconds: 0,
                            }
                        } else {
                            Action::CreateAgentCode {
                                room_id: room,
                                max_uses: 50,
                                ttl_seconds: 86400,
                            }
                        });
                    }
                });
                let Some(code) = self.admin.code.clone() else {
                    ui.label("Create a code, then run the command on each student computer.");
                    return;
                };
                ui.add_space(6.0);
                ui.label(format!(
                    "Code {} · {} use{}",
                    code.code,
                    code.max_uses,
                    if code.max_uses == 1 { "" } else { "s" }
                ));
                let win = windows_command(&target, &code.code);
                let lin = linux_command(&target, &code.code);
                ui.strong("Windows (run as administrator, from the unpacked agent-windows folder)");
                ui.label(RichText::new(&win).monospace());
                if ui.button("Copy Windows command").clicked() {
                    ui.ctx().copy_text(win.clone());
                }
                ui.strong("Linux (in the unpacked agent folder)");
                ui.label(RichText::new(&lin).monospace());
                if ui.button("Copy Linux command").clicked() {
                    ui.ctx().copy_text(lin.clone());
                }
                if let Mode::Embedded { port } = mode {
                    let ips = lan_fallback(&addrs, port);
                    if !ips.is_empty() {
                        ui.add_space(6.0);
                        ui.label(format!(
                            "Use one of these if the agents cannot find {hostname}:"
                        ));
                        for ip in ips {
                            ui.label(RichText::new(ip).monospace());
                        }
                    }
                }
            });
        self.connect_open = open;
    }
}
