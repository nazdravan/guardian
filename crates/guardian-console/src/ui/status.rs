//! The state card at the top and the banner under it.

use egui::{Color32, RichText};

use crate::backend::ConnState;
use crate::embedded::EmbeddedState;
use crate::settings::Mode;

const GREEN: Color32 = Color32::from_rgb(0x1f, 0x7a, 0x3d);
const AMBER: Color32 = Color32::from_rgb(0x9a, 0x6a, 0x00);
const RED: Color32 = Color32::from_rgb(0xb3, 0x26, 0x1e);
const GREY: Color32 = Color32::from_rgb(0x55, 0x5b, 0x63);

/// Title, detail and colour of the card.
pub fn card_text(
    mode: Option<&Mode>,
    conn: &ConnState,
    embedded: &EmbeddedState,
) -> (String, Option<String>, Color32) {
    if let EmbeddedState::Failed { reason } = embedded {
        return ("Server is not running".into(), Some(reason.clone()), RED);
    }
    if let ConnState::Reconnecting { attempt, error } = conn {
        return (
            format!("Reconnecting… (attempt {attempt})"),
            Some(error.clone()),
            AMBER,
        );
    }
    match (mode, embedded) {
        (Some(Mode::Embedded { .. }), EmbeddedState::Running { port, .. }) => (
            format!("Server running on this computer · port {port}"),
            None,
            GREEN,
        ),
        (Some(Mode::Embedded { .. }), _) => ("Starting the server…".into(), None, GREY),
        (Some(Mode::Remote { server, .. }), _) => match conn {
            ConnState::Online | ConnState::NeedsLogin { .. } => {
                (format!("Connected to {server}"), None, GREEN)
            }
            _ => (format!("Connecting to {server}…"), None, GREY),
        },
        (None, _) => ("Not connected".into(), None, GREY),
    }
}

/// Draws the card.
pub fn card(ui: &mut egui::Ui, mode: Option<&Mode>, conn: &ConnState, embedded: &EmbeddedState) {
    let (title, detail, colour) = card_text(mode, conn, embedded);
    egui::Frame::new()
        .fill(colour)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(title)
                    .strong()
                    .size(16.0)
                    .color(Color32::WHITE),
            );
            if let Some(d) = detail {
                ui.label(RichText::new(d).color(Color32::WHITE));
            }
        });
}

/// An amber strip with a message and optional buttons; returns the clicked
/// button's index.
pub fn banner(ui: &mut egui::Ui, text: &str, buttons: &[&str]) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(Color32::from_rgb(0xff, 0xf4, 0xd6))
        .stroke(egui::Stroke::new(1.0, AMBER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(Color32::from_rgb(0x4a, 0x35, 0x00)));
                for (i, b) in buttons.iter().enumerate() {
                    if ui.button(*b).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}
