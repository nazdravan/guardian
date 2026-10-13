use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};
use guardian_ui::GuardianApp;
use guardian_ui::state::Tab;

fn snap() -> guardian_core::snapshot::Snapshot {
    snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
}

#[test]
fn diagnostics_tab_shows_verdict_and_checks() {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 720.0)).build_eframe(|cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        GuardianApp::with_snapshot(snap(), Tab::Diagnostics)
    });
    h.run_steps(2);
    h.get_by_label_contains("Protection is on");
    h.get_by_label("Last sync");
    h.get_by_label("Server refusal");
}
