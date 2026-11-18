//! The teacher's view: each room with its profile switch and its computers.

use egui::RichText;
use guardian_proto::v1::Mode as PMode;

use crate::fleet::{ComputerView, Fleet, RoomView};

/// A profile change the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    /// Room.
    pub room_id: i64,
    /// Room name.
    pub room_name: String,
    /// Chosen profile.
    pub profile_id: i64,
    /// Whether it is a `free` profile (needs confirmation).
    pub free: bool,
    /// Computers in the room.
    pub computers: usize,
}

fn computers_table(ui: &mut egui::Ui, id: &str, list: &[ComputerView]) {
    egui::Grid::new(id)
        .striped(true)
        .num_columns(3)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            for c in list {
                ui.label(&c.name);
                let colour = if c.online {
                    egui::Color32::from_rgb(0x1f, 0x7a, 0x3d)
                } else {
                    egui::Color32::GRAY
                };
                ui.horizontal(|ui| {
                    // A dot drawn as a shape: "●" is not in egui's built-in fonts.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, colour);
                    ui.label(RichText::new(&c.connection).color(colour));
                });
                let p = c.profile.clone().unwrap_or_default();
                if c.exception {
                    ui.label(format!("{p} (exception)"));
                } else {
                    ui.label(p);
                }
                ui.end_row();
            }
        });
}

fn room_card(
    ui: &mut egui::Ui,
    fleet: &Fleet,
    r: &RoomView,
    allow_free: bool,
) -> Option<ProfileChoice> {
    let mut choice = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&r.name).strong().size(16.0));
            ui.label(format!("{}/{} online", r.online, r.computers.len()));
            ui.add_space(16.0);
            let current = r
                .profile
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "—".into());
            egui::ComboBox::new(("room-profile", r.id), "Profile")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for p in fleet.profiles() {
                        if !allow_free && p.mode == PMode::Free as i32 {
                            continue;
                        }
                        let selected = r.profile.as_ref().is_some_and(|c| c.id == p.id);
                        if ui.selectable_label(selected, &p.name).clicked() && !selected {
                            choice = Some(ProfileChoice {
                                room_id: r.id,
                                room_name: r.name.clone(),
                                profile_id: p.id,
                                free: p.mode == PMode::Free as i32,
                                computers: r.computers.len(),
                            });
                        }
                    }
                });
        });
        ui.add_space(4.0);
        if r.computers.is_empty() {
            ui.label("No computers in this room yet — use Connection info… to enroll them.");
        } else {
            computers_table(ui, &format!("room-computers-{}", r.id), &r.computers);
        }
    });
    choice
}

/// Draws every room; returns a profile switch the user picked.
/// `allow_free: false` (fallback mode) leaves the free profiles out.
pub fn show(
    ui: &mut egui::Ui,
    fleet: &Fleet,
    admin: bool,
    allow_free: bool,
) -> Option<ProfileChoice> {
    let mut choice = None;
    let rooms = fleet.rooms();
    if rooms.is_empty() {
        ui.label(if admin {
            "No rooms yet. An administrator creates them in Rooms."
        } else {
            "You have no rooms yet. Ask an administrator to give you one."
        });
    }
    for r in &rooms {
        if let Some(c) = room_card(ui, fleet, r, allow_free) {
            choice = Some(c);
        }
        ui.add_space(8.0);
    }
    if admin {
        let loose = fleet.unassigned();
        if !loose.is_empty() {
            ui.label(RichText::new("Not in a room").strong());
            computers_table(ui, "unassigned", &loose);
        }
    }
    choice
}
