//! Stub: Task 11 fills the Configuration tab, its dialogs and the worker results.

use crate::GuardianApp;
use crate::worker::Done;

pub enum Dialog {
    DiscardOnRefresh,
}

pub fn show(_: &mut GuardianApp, ui: &mut egui::Ui) {
    ui.label("Configuration");
}

pub fn dialogs(_: &mut GuardianApp, _: &egui::Context) {}

pub fn on_done(_: &mut GuardianApp, _: Done, _: &egui::Context) {}
