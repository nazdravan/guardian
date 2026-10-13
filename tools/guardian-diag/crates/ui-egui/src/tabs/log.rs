//! Stub: Task 12 fills the Log tab.

use crate::GuardianApp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    All,
}

pub fn show(_: &mut GuardianApp, ui: &mut egui::Ui) {
    ui.label("Log");
}
