#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_setup::app::SetupApp;
use guardian_setup::fake::FakeHost;
use guardian_setup::host::{Host, ServiceState};
use guardian_setup::lifecycle::{InstallArgs, install};
use guardian_setup::model::{SERVICE, Secret};

fn harness(h: Arc<FakeHost>) -> Harness<'static, SetupApp> {
    let app = SetupApp::new(h);
    let mut hs = Harness::builder()
        .with_size((900.0, 640.0))
        .build_ui_state(|ui, app: &mut SetupApp| app.show(ui), app);
    settle(&mut hs);
    hs
}

/// Runs frames until no operation is running.
fn settle(h: &mut Harness<'static, SetupApp>) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        h.step();
        if !h.state().busy() {
            h.run();
            return;
        }
        assert!(Instant::now() < end, "an operation never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn type_into(h: &mut Harness<'static, SetupApp>, label: &str, text: &str) {
    h.get_by_label(label).focus();
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
}

fn installed() -> Arc<FakeHost> {
    let f = Arc::new(FakeHost::new());
    install(
        &*f,
        &InstallArgs {
            server: "wss://srv.school:8443".into(),
            code: Secret::new("ABCD-EFGH"),
            cert_sha256: None,
            local_port: None,
            remove_legacy: false,
            no_start: false,
        },
        &|_| {},
    )
    .unwrap();
    f
}

#[test]
fn not_installed_shows_the_install_form() {
    let mut h = harness(Arc::new(FakeHost::new()));
    h.get_by_label("Server address");
    h.get_by_label("Certificate SHA-256 (optional)");
    h.get_by_label("Enrollment code");
    h.get_by_label("Install");
    h.run();
}

#[test]
fn install_sends_the_values() {
    let f = Arc::new(FakeHost::new());
    let mut h = harness(f.clone());
    type_into(&mut h, "Server address", "wss://srv.school:8443");
    type_into(&mut h, "Enrollment code", "ABCD-EFGH");
    h.get_by_label("Install").click();
    settle(&mut h);
    assert!(
        f.calls()
            .iter()
            .any(|c| c.contains("enroll --server wss://srv.school:8443 --code ***")),
        "{:?}",
        f.calls()
    );
    assert_eq!(f.service_state(SERVICE), Some(ServiceState::Running));
    h.get_by_label_contains("Computer #");
}

#[test]
fn legacy_install_asks_first() {
    let f = Arc::new(FakeHost::new());
    f.with_legacy();
    let mut h = harness(f.clone());
    h.get_by_label_contains("Guardian 2.x is installed on this computer.");
    type_into(&mut h, "Server address", "wss://srv.school:8443");
    type_into(&mut h, "Enrollment code", "ABCD-EFGH");
    h.get_by_label("Install").click();
    h.run();
    h.get_by_label_contains("Remove Guardian 2.x and install 3.0?");
    assert!(f.service_state(SERVICE).is_none());
    h.get_by_label("Yes").click();
    settle(&mut h);
    assert_eq!(f.service_state(SERVICE), Some(ServiceState::Running));
    assert!(f.service_state("ProcSentinelAgent").is_none());
}

#[test]
fn installed_shows_service_and_buttons() {
    let h = harness(installed());
    h.get_by_label_contains("Computer #");
    h.get_by_label_contains("wss://srv.school:8443");
    h.get_by_label("Service: running");
    h.get_by_label("Local fallback port 11500: open");
    for b in [
        "Start",
        "Stop",
        "Restart",
        "Update",
        "Re-enroll…",
        "Uninstall…",
    ] {
        h.get_by_label(b);
    }
}

#[test]
fn stop_and_start_call_the_service() {
    let f = installed();
    let mut h = harness(f.clone());
    h.get_by_label("Stop").click();
    settle(&mut h);
    assert_eq!(f.service_state(SERVICE), Some(ServiceState::Stopped));
    h.get_by_label("Service: stopped");
    h.get_by_label("Start").click();
    settle(&mut h);
    assert_eq!(f.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn whitelist_tab_adds_and_removes() {
    let f = installed();
    let mut h = harness(f.clone());
    h.get_by_label("Whitelist").click();
    settle(&mut h);
    type_into(&mut h, "Program name", "Notes.exe");
    h.get_by_label("Add").click();
    settle(&mut h);
    h.get_by_label("notes.exe");
    let text = std::fs::read_to_string(guardian_setup::lifecycle::whitelist_path(&*f)).unwrap();
    assert!(text.contains("notes.exe"));
    h.get_by_label("Remove notes.exe").click();
    settle(&mut h);
    assert!(h.query_by_label("notes.exe").is_none());
}

#[test]
fn op_error_is_shown_and_window_stays() {
    let f = Arc::new(FakeHost::new());
    f.fail_enroll(Some("invalid enrollment code"));
    let mut h = harness(f.clone());
    type_into(&mut h, "Server address", "wss://srv.school:8443");
    type_into(&mut h, "Enrollment code", "WRONG");
    h.get_by_label("Install").click();
    settle(&mut h);
    h.get_by_label_contains("Enrollment failed: invalid enrollment code");
    h.get_by_label("Install");
    assert!(f.service_state(SERVICE).is_none());
}

#[test]
fn not_elevated_is_explained() {
    let f = Arc::new(FakeHost::new());
    f.not_elevated();
    let h = harness(f.clone());
    h.get_by_label_contains("without administrator rights");
    assert!(!f.is_elevated());
}
