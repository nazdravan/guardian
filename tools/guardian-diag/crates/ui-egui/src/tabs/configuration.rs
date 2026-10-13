//! Configuration tab (spec R9–R15): view everything, edit .env and whitelist.txt as admin.

use guardian_core::agent;
use guardian_core::diagnose::Status;
use guardian_core::edit::plan::{WritePlan, plan};
use guardian_core::edit::{Field, Issue, Severity, TokenEdit, check_new_entry, validate};
use guardian_core::read::Read;

use crate::state::edit_lock;
use crate::worker::{Done, Job, Worker};
use crate::{GuardianApp, theme};

pub enum Dialog {
    DiscardOnRefresh,
    Confirm { plan: WritePlan, warnings: Vec<Issue> },
    ChangedOnDisk { plan: WritePlan },
    Restart,
    Error(String),
}

pub fn add_entry(app: &mut GuardianApp, raw: &str) -> Result<Vec<Issue>, Issue> {
    let Some(d) = app.draft.as_mut() else { return Ok(vec![]) };
    let (name, warnings) = check_new_entry(&d.whitelist.entries, raw)?;
    d.whitelist.entries.push(name);
    Ok(warnings)
}

pub fn begin_save(app: &mut GuardianApp) {
    let (Some(s), Some(d)) = (&app.snapshot, &app.draft) else { return };
    let env = d.effective_env();
    match plan(s, &env, &d.whitelist) {
        Ok(p) if p.is_empty() => app.notice = Some((Status::Ok, "Nothing to save.".into())),
        Ok(p) => {
            let warnings = validate(s, &env, &d.whitelist).into_iter().filter(|i| i.severity == Severity::Warning).collect();
            app.dialog = Some(Dialog::Confirm { plan: p, warnings });
        }
        Err(errors) => app.notice = Some((Status::Error, errors.into_iter().map(|e| e.message).collect::<Vec<_>>().join(" "))),
    }
}

fn issues_for(issues: &[Issue], f: Field) -> impl Iterator<Item = &Issue> {
    issues.iter().filter(move |i| i.field == f)
}

fn show_issues(ui: &mut egui::Ui, issues: &[Issue], f: Field) {
    for i in issues_for(issues, f) {
        let c = if i.severity == Severity::Error { theme::palette().error } else { theme::palette().warn };
        ui.colored_label(c, &i.message);
    }
}

fn read_problem<T>(ui: &mut egui::Ui, what: &str, r: &Read<T>) {
    if let Some(p) = r.problem() {
        ui.colored_label(theme::palette().muted, format!("{what}: {p}"));
    }
}

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let Some(s) = app.snapshot.clone() else { return };
    let lock = edit_lock(&s);
    let editable = lock.is_none();
    let p = theme::palette();

    if let Some(msg) = lock {
        ui.colored_label(p.warn, msg);
    }
    // R10: pending restart banner
    if let Some(c) = app.diagnosis.as_ref().and_then(|d| d.checks.iter().find(|c| c.id == guardian_core::diagnose::CheckId::PendingRestart))
        && c.status == Status::Warn
    {
        ui.horizontal(|ui| {
            ui.colored_label(p.warn, "Changes apply after the service restarts.");
            if editable && !s.is_manual_dir() && ui.add_enabled(!app.busy(), egui::Button::new("Restart service")).clicked() {
                app.worker = Some(Worker::spawn(Job::Restart, ui.ctx().clone()));
            }
        });
    }
    ui.label(
        egui::RichText::new("whitelist.txt applies only in whitelist mode. The agent reads .env and whitelist.txt once, when the service starts.")
            .color(p.muted),
    );

    // Save bar
    let dirty = app.dirty();
    if editable && dirty > 0 {
        ui.horizontal(|ui| {
            ui.colored_label(p.warn, if dirty == 1 { "1 unsaved change".to_string() } else { format!("{dirty} unsaved changes") });
            if ui.add_enabled(!app.busy(), egui::Button::new("Save")).clicked() {
                begin_save(app);
            }
            if ui.button("Revert").clicked() {
                app.draft = Some(crate::state::Draft::from_snapshot(&s));
            }
        });
    }

    let issues = app.draft.as_ref().map(|d| validate(&s, &d.effective_env(), &d.whitelist)).unwrap_or_default();

    // Agent settings
    ui.add_space(8.0);
    ui.label(theme::heading("Agent settings"));
    ui.label(theme::mono(s.install.path.join(agent::ENV_FILE).display().to_string()));
    read_problem(ui, ".env", &s.env);
    if let Some(e) = s.env.as_ok() {
        if e.parsed.encoding == guardian_core::read::Encoding::Utf8Bom {
            ui.colored_label(p.warn, "The file starts with a byte-order mark the agent cannot read past; the first line is ignored.");
        }
        for (k, n) in &e.parsed.duplicates {
            ui.colored_label(p.warn, format!("{k} is defined {n} times; the last one is used."));
        }
    }
    if let Some(d) = app.draft.as_mut() {
        egui::Grid::new("env").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("SERVER_ADDRESS");
            ui.add_enabled(editable, egui::TextEdit::singleline(&mut d.env.server_address).font(egui::TextStyle::Monospace).desired_width(420.0));
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| {
                show_issues(ui, &issues, Field::ServerAddress);
                if let Some(c) = s.credentials.as_ok() {
                    let cur = guardian_core::edit::canonical_server_address(&d.env.server_address);
                    if editable && !c.server_address.is_empty() && c.server_address != cur && ui.button(format!("Use {}", c.server_address)).clicked()
                    {
                        d.env.server_address = c.server_address.clone();
                    }
                }
            });
            ui.end_row();

            ui.label("CHECK_INTERVAL");
            ui.horizontal(|ui| {
                ui.add_enabled(editable, egui::TextEdit::singleline(&mut d.env.check_interval).font(egui::TextStyle::Monospace).desired_width(80.0));
                ui.label(format!("seconds (empty = agent default, {} s)", agent::DEFAULT_SERVICE_INTERVAL_SECS));
            });
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| show_issues(ui, &issues, Field::CheckInterval));
            ui.end_row();

            ui.label("BINDING_TOKEN");
            ui.vertical(|ui| {
                let current = s.env.as_ok().and_then(|e| e.parsed.binding_token.clone());
                let state = match (&d.env.binding_token, &current) {
                    (TokenEdit::Remove, _) => "will be removed".to_string(),
                    (_, Some(t)) => format!("present (…{})", t.0),
                    (_, None) => "absent".into(),
                };
                ui.label(theme::mono(state));
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        editable,
                        egui::TextEdit::singleline(&mut d.token_field).password(true).hint_text("paste a new binding token").desired_width(320.0),
                    );
                    if current.is_some() && editable {
                        let removing = d.env.binding_token == TokenEdit::Remove;
                        if ui.button(if removing { "Keep" } else { "Remove" }).clicked() {
                            d.env.binding_token = if removing { TokenEdit::Keep } else { TokenEdit::Remove };
                        }
                    }
                });
                show_issues(ui, &issues, Field::BindingToken);
            });
            ui.end_row();
        });
        if let Some(e) = s.env.as_ok()
            && !e.parsed.unknown.is_empty()
        {
            ui.collapsing("Other keys (ignored by the agent)", |ui| {
                for (k, v) in &e.parsed.unknown {
                    ui.label(theme::mono(format!("{k}={v}")));
                }
            });
        }
    }

    // Enrollment and policy (read-only)
    ui.add_space(12.0);
    ui.label(theme::heading("Enrollment"));
    read_problem(ui, "agent_credentials.json", &s.credentials);
    if let Some(c) = s.credentials.as_ok() {
        ui.label(theme::mono(format!("machine {}\nserver  {}\nsince   {}", c.machine_guid, c.server_address, c.enrolled_at)));
    }
    ui.add_space(12.0);
    ui.label(theme::heading("Policy (from the last sync)"));
    read_problem(ui, "sync.json", &s.sync);
    if let Some(f) = s.sync.as_ok() {
        let at = f.mtime.map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        ui.label(theme::mono(format!("mode {} · synced {at}", f.parsed.mode.as_str())));
        for a in &f.parsed.applications {
            ui.label(theme::mono(&a.name));
        }
    }

    // Local whitelist
    ui.add_space(12.0);
    ui.label(theme::heading("Local whitelist"));
    ui.label(theme::mono(s.install.path.join(agent::WHITELIST_FILE).display().to_string()));
    match &s.whitelist {
        Read::Missing => {
            ui.colored_label(p.muted, "absent — the agent generates it from the running processes on its next start");
        }
        other => read_problem(ui, "whitelist.txt", other),
    }
    if editable {
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut app.new_entry).hint_text("program.exe or a pasted path").desired_width(320.0));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Add").clicked() || enter {
                let raw = std::mem::take(&mut app.new_entry);
                match add_entry(app, &raw) {
                    Ok(w) if w.is_empty() => app.notice = None,
                    Ok(w) => app.notice = Some((Status::Warn, w.into_iter().map(|i| i.message).collect::<Vec<_>>().join(" "))),
                    Err(e) => {
                        app.new_entry = raw;
                        app.notice = Some((Status::Error, e.message));
                    }
                }
            }
            ui.menu_button("Add from running", |ui| {
                let running = s.processes.as_ok().cloned().unwrap_or_default();
                let mut names: Vec<String> = running.iter().map(|n| n.to_lowercase()).collect();
                names.sort();
                names.dedup();
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for n in names {
                        let have = app.draft.as_ref().is_some_and(|d| d.whitelist.entries.contains(&n));
                        let label = if agent::is_protected(&n) {
                            format!("{n}  (protected)")
                        } else if have {
                            format!("{n}  (in list)")
                        } else {
                            n.clone()
                        };
                        if ui.add_enabled(!have && !agent::is_protected(&n), egui::Button::new(theme::mono(label))).clicked() {
                            let _ = add_entry(app, &n);
                        }
                    }
                });
            });
        });
    }
    if let Some(d) = app.draft.as_mut() {
        let mut remove = None;
        for (i, e) in d.whitelist.entries.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(theme::mono(e));
                if agent::is_protected(e) {
                    ui.colored_label(p.muted, "already protected");
                }
                if editable && ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            d.whitelist.entries.remove(i);
        }
    }

    ui.add_space(12.0);
    ui.collapsing("Built-in protected processes", |ui| {
        for n in agent::PROTECTED {
            ui.label(theme::mono(*n));
        }
    });
}

pub fn dialogs(app: &mut GuardianApp, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.take() else { return };
    let mut keep = true;
    egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
        ui.set_width(520.0);
        match &dialog {
            Dialog::DiscardOnRefresh => {
                ui.label("Discard unsaved changes?");
                ui.horizontal(|ui| {
                    if ui.button("Discard and refresh").clicked() {
                        keep = false;
                        app.refresh(ctx);
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::Confirm { plan, warnings } => {
                ui.label(theme::heading("Save changes?"));
                for f in &plan.files {
                    ui.label(egui::RichText::new(f.kind.file_name()).strong());
                    for l in &f.summary {
                        ui.label(theme::mono(l));
                    }
                }
                for w in warnings {
                    ui.colored_label(theme::palette().warn, &w.message);
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        keep = false;
                        app.worker = Some(Worker::spawn(Job::Apply { plan: plan.clone(), overwrite: false }, ctx.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::ChangedOnDisk { plan } => {
                let names: Vec<&str> = plan.files.iter().map(|f| f.kind.file_name()).collect();
                ui.label(format!("{} changed on disk since it was loaded.", names.join(", ")));
                ui.horizontal(|ui| {
                    if ui.button("Reload").clicked() {
                        keep = false;
                        app.draft = None;
                        app.refresh(ctx);
                    }
                    if ui.button("Overwrite").clicked() {
                        keep = false;
                        app.worker = Some(Worker::spawn(Job::Apply { plan: plan.clone(), overwrite: true }, ctx.clone()));
                    }
                });
            }
            Dialog::Restart => {
                ui.label(theme::heading("Restart ProcSentinelAgent now?"));
                ui.label("Saved files take effect when the service restarts.");
                if app.snapshot.as_ref().and_then(|s| s.sync.as_ok()).is_some_and(|f| f.parsed.mode == guardian_core::parse::Mode::Whitelist) {
                    ui.colored_label(
                        theme::palette().warn,
                        "In whitelist mode the computer is unprotected for a few seconds while the service restarts.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Restart").clicked() {
                        keep = false;
                        app.worker = Some(Worker::spawn(Job::Restart, ctx.clone()));
                    }
                    if ui.button("Later").clicked() {
                        keep = false;
                        app.refresh(ctx);
                    }
                });
            }
            Dialog::Error(m) => {
                ui.colored_label(theme::palette().error, m);
                if ui.button("OK").clicked() {
                    keep = false;
                }
            }
        }
    });
    // A button that opened a follow-up (worker → on_done) leaves app.dialog for on_done to set.
    if keep && app.dialog.is_none() {
        app.dialog = Some(dialog);
    }
}

pub fn on_done(app: &mut GuardianApp, done: Done, ctx: &egui::Context) {
    match done {
        // lib.rs handles these itself.
        Done::Collected(_) | Done::Relaunched(_) | Done::Crashed(_) => {}
        Done::Applied(Ok(_)) => {
            // Keep the token out of memory once written (R4).
            if let Some(d) = app.draft.as_mut() {
                d.token_field.clear();
            }
            let manual = app.snapshot.as_ref().is_some_and(|s| s.is_manual_dir());
            app.notice = Some((Status::Ok, "Saved.".into()));
            if manual {
                app.refresh(ctx);
            } else {
                app.dialog = Some(Dialog::Restart);
            }
        }
        Done::Applied(Err(e)) if e.step == guardian_winhost::apply::Step::Changed => {
            let planned = match (&app.snapshot, &app.draft) {
                (Some(s), Some(d)) => plan(s, &d.effective_env(), &d.whitelist).ok(),
                _ => None,
            };
            app.dialog = Some(match planned {
                Some(p) => Dialog::ChangedOnDisk { plan: p },
                None => Dialog::Error(e.to_string()),
            });
        }
        Done::Applied(Err(e)) => app.dialog = Some(Dialog::Error(e.to_string())),
        Done::Restarted(Ok(())) => {
            app.notice = Some((Status::Ok, "ProcSentinelAgent restarted.".into()));
            app.refresh(ctx);
        }
        Done::Restarted(Err(e)) => {
            app.dialog = Some(Dialog::Error(format!("The files are saved, but the service did not restart: {e}. See the Log tab.")));
            app.refresh(ctx);
        }
        Done::ReportSaved(Ok(p)) => app.notice = Some((Status::Ok, format!("Report saved to {}", p.display()))),
        Done::ReportSaved(Err(e)) => app.dialog = Some(Dialog::Error(format!("Could not save the report: {e}. Choose another folder."))),
    }
}
