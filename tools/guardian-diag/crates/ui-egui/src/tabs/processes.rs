//! Processes tab (spec R16–R19): what the agent's `enforce` would do to each running process.

use guardian_core::enforce::{ProcessVerdict, verdict_for};

use crate::state::{Tab, edit_lock};
use crate::{GuardianApp, theme};

fn colour(v: &ProcessVerdict) -> egui::Color32 {
    let p = theme::palette();
    match v {
        ProcessVerdict::WillClose { .. } => p.error,
        ProcessVerdict::Unknown(_) => p.muted,
        _ => p.text,
    }
}

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let Some(s) = app.snapshot.clone() else { return };
    let editable = edit_lock(&s).is_none();

    ui.label(theme::heading("Why is … closed?"));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.why_query).hint_text("program.exe").desired_width(260.0));
        if !app.why_query.trim().is_empty() {
            let v = verdict_for(&s, app.why_query.trim());
            ui.colored_label(colour(&v), v.label());
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.process_filter).hint_text("filter").desired_width(200.0));
        ui.checkbox(&mut app.only_closing, "only what will be closed");
    });
    if let Some(problem) = s.processes.problem() {
        ui.colored_label(theme::palette().muted, format!("Running processes: {problem}"));
        return;
    }
    ui.label(egui::RichText::new("Verdicts use the saved files, not unsaved edits.").color(theme::palette().muted));
    let mut names: Vec<String> = s.processes.as_ok().cloned().unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by_key(|n| n.to_lowercase());
    let filter = app.process_filter.to_lowercase();
    let mut add: Option<String> = None;
    egui::Grid::new("procs").num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
        for n in &names {
            let v = verdict_for(&s, n);
            if (!filter.is_empty() && !n.to_lowercase().contains(&filter)) || (app.only_closing && !v.will_close()) {
                continue;
            }
            ui.label(theme::mono(n));
            ui.colored_label(colour(&v), v.label());
            // Only a whitelist-mode close: in blacklist mode whitelist.txt does not help.
            if editable && matches!(v, ProcessVerdict::WillClose { rule: None }) && ui.small_button("Add to local whitelist").clicked() {
                add = Some(n.clone());
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    if let Some(n) = add {
        add_and_open_configuration(app, &n);
    }
}

/// Shared by the Processes and Log tabs: put the name in the draft and show it there.
pub fn add_and_open_configuration(app: &mut GuardianApp, name: &str) {
    if let Err(e) = crate::tabs::configuration::add_entry(app, name) {
        app.notice = Some((guardian_core::diagnose::Status::Error, e.message));
    }
    app.tab = Tab::Configuration;
}
