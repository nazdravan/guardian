use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};
use guardian_ui::GuardianApp;
use guardian_ui::state::Tab;
use guardian_ui::tabs::configuration::Dialog;

fn snap(elevated: bool) -> guardian_core::snapshot::Snapshot {
    let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
    s.elevated = elevated;
    s
}

fn harness(elevated: bool) -> Harness<'static, GuardianApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_eframe(move |cc| {
        guardian_ui::theme::install(&cc.egui_ctx);
        GuardianApp::with_snapshot(snap(elevated), Tab::Configuration)
    });
    h.run_steps(2);
    h
}

#[test]
fn read_only_without_admin() {
    let h = harness(false);
    h.get_by_label("Run as administrator to edit");
    assert!(h.query_by_label("Save").is_none());
    h.get_by_label("notepad.exe");
}

#[test]
fn add_entry_and_save_opens_confirm() {
    let mut h = harness(true);
    assert_eq!(guardian_ui::tabs::configuration::add_entry(h.state_mut(), "\"C:\\Office\\WINWORD.EXE\"").unwrap(), vec![]);
    h.run_steps(2);
    h.get_by_label("1 unsaved change");
    h.get_by_label("Save").click();
    h.run_steps(2);
    match &h.state().dialog {
        Some(Dialog::Confirm { plan, warnings }) => {
            assert_eq!(plan.files.len(), 1);
            assert_eq!(plan.files[0].summary, vec!["+ winword.exe".to_string()]);
            assert!(warnings.is_empty());
        }
        _ => panic!("expected the confirm dialog"),
    }
}

#[test]
fn token_field_never_echoes_and_warns_when_enrolled() {
    let mut h = harness(true);
    h.state_mut().draft.as_mut().unwrap().token_field = "bt_typed_secret".into();
    h.run_steps(2);
    assert!(h.query_by_label_contains("bt_typed_secret").is_none());
    guardian_ui::tabs::configuration::begin_save(h.state_mut());
    match &h.state().dialog {
        Some(Dialog::Confirm { warnings, plan }) => {
            assert!(warnings.iter().any(|w| w.message.contains("Unenrol it in the cabinet first")));
            assert_eq!(plan.files[0].summary, vec!["BINDING_TOKEN: set".to_string()]);
        }
        _ => panic!("expected the confirm dialog"),
    }
}

#[test]
fn invalid_server_address_disables_save() {
    let mut h = harness(true);
    h.state_mut().draft.as_mut().unwrap().env.server_address = "guardian.example.com".into();
    h.run_steps(2);
    h.get_by_label_contains("must start with http:// or https://");
    guardian_ui::tabs::configuration::begin_save(h.state_mut());
    assert!(h.state().dialog.is_none());
}

#[test]
fn reorder_only_says_nothing_to_save() {
    let mut h = harness(true);
    h.state_mut().draft.as_mut().unwrap().whitelist.entries.reverse();
    guardian_ui::tabs::configuration::begin_save(h.state_mut());
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().notice.as_ref().map(|n| n.1.as_str()), Some("Nothing to save."));
}
