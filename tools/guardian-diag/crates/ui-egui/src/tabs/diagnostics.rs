use guardian_core::diagnose::Status;

use crate::GuardianApp;
use crate::theme;

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let (Some(s), Some(d)) = (&app.snapshot, &app.diagnosis) else { return };
    ui.add_space(8.0);
    ui.label(theme::heading(&d.verdict.text).color(theme::status_color(d.worst_for_verdict())).size(26.0));
    if !d.verdict.hint.is_empty() {
        ui.label(&d.verdict.hint);
    }
    if guardian_core::enforce::looks_locked(s) {
        ui.colored_label(theme::palette().warn, "Looks like this computer is locked in the cabinet (or the room allows nothing).");
    }
    if s.is_manual_dir() {
        ui.label(format!("Files from {} (chosen manually); service and log are this computer's.", s.install.path.display()));
    }
    ui.add_space(12.0);
    egui::Grid::new("checks").num_columns(3).spacing([16.0, 8.0]).striped(true).show(ui, |ui| {
        for c in &d.checks {
            let mark = match c.status {
                Status::Ok => "●  ok",
                Status::Warn => "●  warning",
                Status::Error => "●  error",
                Status::Unknown => "○  not checked",
            };
            ui.colored_label(theme::status_color(c.status), mark);
            ui.label(egui::RichText::new(c.title).strong());
            ui.vertical(|ui| {
                ui.label(theme::mono(&c.fact));
                if c.status != Status::Ok && !c.todo.is_empty() {
                    ui.label(egui::RichText::new(&c.todo).color(theme::palette().muted));
                }
            });
            ui.end_row();
        }
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!app.busy(), egui::Button::new("Choose agent folder…")).clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            app.manual_dir = Some(dir);
            app.refresh(ui.ctx());
        }
        if ui.add_enabled(!app.busy(), egui::Button::new("Save report")).clicked()
            && let Some(s) = app.snapshot.clone()
        {
            app.worker = Some(crate::worker::Worker::spawn(crate::worker::Job::SaveReport(Box::new(s)), ui.ctx().clone()));
        }
    });
}
