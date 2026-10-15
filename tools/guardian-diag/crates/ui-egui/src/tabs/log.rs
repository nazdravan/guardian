//! Log tab (spec R20–R23): the agent's Event Log records, newest first.

use chrono::{DateTime, Utc};
use guardian_core::parse::{AgentEvent, EventKind, KillReason, Level};
use guardian_core::report::kill_summary;

use crate::state::edit_lock;
use crate::{GuardianApp, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Kills,
    Problems,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Hour,
    Day,
    Week,
    All,
}

pub fn visible<'a>(events: &'a [AgentEvent], f: Filter, p: Period, search: &str, now: DateTime<Utc>) -> Vec<&'a AgentEvent> {
    let since = match p {
        Period::Hour => Some(now - chrono::Duration::hours(1)),
        Period::Day => Some(now - chrono::Duration::hours(24)),
        Period::Week => Some(now - chrono::Duration::days(7)),
        Period::All => None,
    };
    let q = search.to_lowercase();
    events
        .iter()
        .filter(|e| since.is_none_or(|s| e.time >= s))
        .filter(|e| match f {
            Filter::All => true,
            Filter::Kills => matches!(e.kind, EventKind::Kill { .. }),
            Filter::Problems => e.level != Level::Info,
            Filter::Sync => matches!(e.kind, EventKind::Sync) || e.message.starts_with("Sync failed"),
        })
        .filter(|e| q.is_empty() || e.message.to_lowercase().contains(&q))
        .collect()
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn show(app: &mut GuardianApp, ui: &mut egui::Ui) {
    let Some(s) = app.snapshot.clone() else { return };
    let Some(events) = s.events.as_ok() else {
        ui.colored_label(theme::palette().muted, format!("Event Log: {}", s.events.problem().unwrap_or_default()));
        return;
    };
    if events.is_empty() {
        ui.label("No ProcSentinelAgent records. If the agent was only ever run from a console, it never wrote to the Event Log.");
        return;
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.kill_summary, "Kill summary");
        if !app.kill_summary {
            for (f, l) in [(Filter::All, "All"), (Filter::Kills, "Kills"), (Filter::Problems, "Errors and warnings"), (Filter::Sync, "Sync")] {
                ui.selectable_value(&mut app.log_filter, f, l);
            }
            ui.separator();
            for (p, l) in [(Period::Hour, "1 h"), (Period::Day, "24 h"), (Period::Week, "7 days"), (Period::All, "all")] {
                ui.selectable_value(&mut app.log_period, p, l);
            }
            ui.add(egui::TextEdit::singleline(&mut app.log_search).hint_text("search").desired_width(180.0));
        }
    });
    let oldest = events.iter().map(|e| e.time).min();
    ui.label(
        egui::RichText::new(format!("{} records, the log holds entries since {}", events.len(), oldest.map(local).unwrap_or_default()))
            .color(theme::palette().muted),
    );

    let editable = edit_lock(&s).is_none();
    let mut add: Option<String> = None;
    if app.kill_summary {
        egui::Grid::new("kills").num_columns(5).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
            for k in kill_summary(events) {
                ui.label(theme::mono(&k.process));
                ui.label(format!("×{}", k.count));
                ui.label(theme::mono(format!("{} … {}", local(k.first), local(k.last))));
                ui.label(match k.reason {
                    KillReason::Blacklist => "blacklist",
                    KillReason::Whitelist => "whitelist",
                });
                if editable && k.reason == KillReason::Whitelist && ui.small_button("Add to local whitelist").clicked() {
                    add = Some(k.process.clone());
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    } else {
        egui::Grid::new("log").num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
            for e in visible(events, app.log_filter, app.log_period, &app.log_search, s.taken_at) {
                ui.label(theme::mono(local(e.time)));
                let c = match e.level {
                    Level::Error => theme::palette().error,
                    Level::Warning => theme::palette().warn,
                    Level::Info => theme::palette().muted,
                };
                ui.colored_label(c, format!("{:?}", e.level));
                ui.label(&e.message);
                ui.end_row();
            }
        });
    }
    if let Some(n) = add {
        crate::tabs::processes::add_and_open_configuration(app, &n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};

    #[test]
    fn visible_filters() {
        let now: DateTime<Utc> = "2026-10-10T11:15:00Z".parse().unwrap();
        let s = snapshot_from_dir(&fixtures_dir().join("healthy"), now);
        let ev = s.events.as_ok().unwrap();
        assert_eq!(visible(ev, Filter::All, Period::All, "", now).len(), 6);
        assert_eq!(visible(ev, Filter::Kills, Period::All, "", now).len(), 2);
        assert_eq!(visible(ev, Filter::Problems, Period::All, "", now).len(), 2);
        assert_eq!(visible(ev, Filter::Sync, Period::All, "", now).len(), 2);
        assert_eq!(visible(ev, Filter::All, Period::All, "STEAM", now).len(), 1);
        assert_eq!(visible(ev, Filter::All, Period::Hour, "", now + chrono::Duration::hours(2)).len(), 0);
    }
}
