use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};
use guardian_ui::GuardianApp;
use guardian_ui::state::Tab;

fn harness(tab: Tab) -> Harness<'static, GuardianApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_eframe(move |cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        GuardianApp::with_snapshot(snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap()), tab)
    });
    h.run_steps(2);
    h
}

#[test]
fn processes_verdicts_and_add_to_whitelist() {
    let mut h = harness(Tab::Processes);
    h.get_by_label("game.exe");
    h.get_by_label_contains("Will be closed");
    h.get_by_label("Add to local whitelist").click();
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Configuration);
    assert!(h.state().draft.as_ref().unwrap().whitelist.entries.contains(&"game.exe".to_string()));
}

#[test]
fn why_is_it_closed() {
    let mut h = harness(Tab::Processes);
    // chrome.exe is in the policy but not running. WINWORD.EXE (running, also in the policy)
    // would make the label ambiguous, so list only what will be closed.
    h.state_mut().only_closing = true;
    h.state_mut().why_query = "chrome.exe".into();
    h.run_steps(2);
    h.get_by_label_contains("Allowed (policy)");
}

#[test]
fn log_kill_summary() {
    let mut h = harness(Tab::Log);
    h.state_mut().kill_summary = true;
    h.run_steps(2);
    h.get_by_label("game.exe");
    h.get_by_label("steam.exe");
    h.get_by_label_contains("6 records");
}

#[test]
fn no_add_button_without_elevation() {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_eframe(move |cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        s.elevated = false;
        GuardianApp::with_snapshot(s, Tab::Processes)
    });
    h.run_steps(2);
    h.get_by_label("game.exe");
    assert!(h.query_by_label("Add to local whitelist").is_none());
}
