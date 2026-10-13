pub mod configuration;
pub mod diagnostics;
pub mod log;
pub mod processes;

use crate::state::{Tab, edit_lock};
use crate::theme;
use crate::{GuardianApp, worker::Worker};

pub fn top_bar(app: &mut GuardianApp, ui: &mut egui::Ui) {
    egui::Panel::top("top").show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::heading("Guardian"));
            if let (Some(s), Some(d)) = (&app.snapshot, &app.diagnosis) {
                ui.label(theme::mono(&s.machine.hostname));
                // The Diagnostics tab shows the verdict large; elsewhere it rides in the bar.
                if app.tab != Tab::Diagnostics {
                    ui.separator();
                    ui.colored_label(theme::status_color(d.worst_for_verdict()), &d.verdict.text);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = app.busy();
                if ui.add_enabled(!busy, egui::Button::new("Refresh")).clicked() {
                    if app.dirty() > 0 {
                        app.dialog = Some(configuration::Dialog::DiscardOnRefresh);
                    } else {
                        app.refresh(ui.ctx());
                    }
                }
                if let Some(s) = &app.snapshot
                    && edit_lock(s).is_some()
                    && ui.add_enabled(!busy, egui::Button::new("Restart as administrator")).clicked()
                {
                    let mut args = vec!["--tab".to_string(), app.tab.arg().to_string()];
                    if let Some(d) = &app.manual_dir {
                        args.extend(["--dir".to_string(), d.display().to_string()]);
                    }
                    match guardian_winhost::relaunch_elevated(&args) {
                        Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => app.notice = Some((guardian_core::diagnose::Status::Warn, e)),
                    }
                }
                if let Some(p) = app.worker.as_ref().and_then(Worker::progress) {
                    ui.spinner();
                    ui.label(p);
                }
            });
        });
        ui.horizontal(|ui| {
            for t in Tab::ALL {
                let label = if t == Tab::Configuration && app.dirty() > 0 { format!("{} •", t.title()) } else { t.title().to_string() };
                ui.selectable_value(&mut app.tab, t, label);
            }
        });
        if let Some((st, msg)) = &app.notice {
            ui.colored_label(theme::status_color(*st), msg);
        }
        ui.add_space(4.0);
    });
}
