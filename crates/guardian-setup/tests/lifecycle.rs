#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use guardian_agent::config::AgentConfig;
use guardian_core::enforce::ProcessInfo;
use guardian_setup::fake::{FakeHost, Method, NEW_AGENT};
use guardian_setup::host::{Host, HostError, ServiceState};
use guardian_setup::lifecycle::{
    InstallArgs, OpError, install, reenroll, status, uninstall, update,
};
use guardian_setup::model::{FIREWALL_RULE, LEGACY_SERVICE, SERVICE, Secret};

fn args() -> InstallArgs {
    InstallArgs {
        server: "wss://srv.school:8443".into(),
        code: Secret::new("ABCD-EFGH"),
        cert_sha256: None,
        local_port: None,
        remove_legacy: false,
        no_start: false,
    }
}

fn quiet(_: &str) {}

fn config(h: &FakeHost) -> AgentConfig {
    AgentConfig::load(&h.io_path(&h.layout().config)).unwrap()
}

fn installed_agent(h: &FakeHost) -> String {
    std::fs::read_to_string(h.io_path(&h.layout().agent_exe)).unwrap()
}

#[test]
fn fresh_install_runs_every_step_in_order() {
    let h = FakeHost::new();
    let r = install(&h, &args(), &quiet).unwrap();
    assert!(r.enrolled_now);
    assert_eq!(r.computer_id, config(&h).computer_id.unwrap());
    assert!(installed_agent(&h).contains("x64"));
    assert!(h.io_path(&h.layout().worker_exe).is_file());
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
    assert_eq!(h.firewall_port(FIREWALL_RULE), Some(11500));
    assert!(
        h.io_path(&h.layout().data_dir.join("whitelist.txt"))
            .is_file()
    );
    let calls = h.calls();
    let pos = |needle: &str| {
        calls
            .iter()
            .position(|c| c.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not in {calls:?}"))
    };
    assert!(pos("run guardian-agent.exe enroll") < pos("set_firewall_rule"));
    assert!(pos("set_firewall_rule") < pos("run guardian-agent.exe service install"));
}

#[test]
fn install_twice_keeps_enrollment() {
    let h = FakeHost::new();
    let first = install(&h, &args(), &quiet).unwrap();
    let second = install(&h, &args(), &quiet).unwrap();
    assert!(!second.enrolled_now);
    assert_eq!(first.computer_id, second.computer_id);
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn legacy_agent_blocks_install_until_removed() {
    let h = FakeHost::new();
    h.with_legacy();
    assert!(matches!(
        install(&h, &args(), &quiet),
        Err(OpError::LegacyAgent)
    ));
    assert!(h.service_state(SERVICE).is_none());
    assert_eq!(h.service_state(LEGACY_SERVICE), Some(ServiceState::Running));
    let mut a = args();
    a.remove_legacy = true;
    install(&h, &a, &quiet).unwrap();
    assert!(h.service_state(LEGACY_SERVICE).is_none());
    assert!(!h.io_path(&h.layout().legacy_dir).exists());
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn failed_enroll_leaves_no_service_and_retry_works() {
    let h = FakeHost::new();
    h.fail_enroll(Some("invalid enrollment code"));
    match install(&h, &args(), &quiet) {
        Err(OpError::Enroll(m)) => assert!(m.contains("invalid enrollment code"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(h.service_state(SERVICE).is_none());
    h.fail_enroll(None);
    install(&h, &args(), &quiet).unwrap();
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn missing_bundled_binaries_is_a_clear_error() {
    let h = FakeHost::new();
    h.without_bundle();
    match install(&h, &args(), &quiet) {
        Err(e @ OpError::MissingBinaries(_)) => {
            assert!(
                e.to_string()
                    .contains("run guardian-setup.exe from the unpacked archive")
            )
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn existing_whitelist_is_not_overwritten() {
    let h = FakeHost::new();
    let wl = h.io_path(&h.layout().data_dir.join("whitelist.txt"));
    std::fs::create_dir_all(wl.parent().unwrap()).unwrap();
    std::fs::write(&wl, "mine.exe\n").unwrap();
    h.set_processes(vec![ProcessInfo {
        pid: 1,
        name: "driver.exe".into(),
        exe_path: Some(PathBuf::from(r"C:\Program Files\X\driver.exe")),
        start_marker: 0,
    }]);
    install(&h, &args(), &quiet).unwrap();
    assert_eq!(std::fs::read_to_string(&wl).unwrap(), "mine.exe\n");
}

#[test]
fn generated_whitelist_has_the_running_programs() {
    let h = FakeHost::new();
    h.set_processes(vec![ProcessInfo {
        pid: 1,
        name: "driver.exe".into(),
        exe_path: Some(PathBuf::from(r"C:\Program Files\X\driver.exe")),
        start_marker: 0,
    }]);
    install(&h, &args(), &quiet).unwrap();
    let text =
        std::fs::read_to_string(h.io_path(&h.layout().data_dir.join("whitelist.txt"))).unwrap();
    assert!(text.contains("driver.exe"), "{text}");
}

#[test]
fn local_port_goes_into_agent_toml_and_the_firewall_rule() {
    let h = FakeHost::new();
    let mut a = args();
    a.local_port = Some(11600);
    install(&h, &a, &quiet).unwrap();
    assert_eq!(config(&h).local_port, Some(11600));
    assert_eq!(h.firewall_port(FIREWALL_RULE), Some(11600));
}

#[test]
fn update_replaces_binaries_and_restarts() {
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    std::fs::write(h.io_path(&h.layout().agent_exe), "old agent").unwrap();
    update(&h, &quiet).unwrap();
    assert!(installed_agent(&h).starts_with(NEW_AGENT));
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
    let calls = h.calls();
    let last_stop = calls
        .iter()
        .rposition(|c| c == "stop_service GuardianAgent")
        .unwrap();
    let last_start = calls
        .iter()
        .rposition(|c| c == "start_service GuardianAgent")
        .unwrap();
    assert!(last_stop < last_start);
}

#[test]
fn update_rolls_back_when_the_new_agent_does_not_start() {
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    std::fs::write(h.io_path(&h.layout().agent_exe), "old agent").unwrap();
    h.agent_wont_start_if(NEW_AGENT);
    assert!(update(&h, &quiet).is_err());
    assert_eq!(installed_agent(&h), "old agent");
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn update_needs_an_installed_agent() {
    let h = FakeHost::new();
    assert!(matches!(update(&h, &quiet), Err(OpError::NotInstalled)));
}

#[test]
fn uninstall_keeps_data_purge_removes_it() {
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    uninstall(&h, false, &quiet).unwrap();
    assert!(h.service_state(SERVICE).is_none());
    assert!(!h.io_path(&h.layout().program_dir).exists());
    assert!(h.io_path(&h.layout().config).is_file());
    install(&h, &args(), &quiet).unwrap();
    uninstall(&h, true, &quiet).unwrap();
    assert!(!h.io_path(&h.layout().data_dir).exists());
}

#[test]
fn uninstall_removes_firewall_rule() {
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    uninstall(&h, false, &quiet).unwrap();
    assert_eq!(h.firewall_port(FIREWALL_RULE), None);
}

#[test]
fn reenroll_clears_identity_and_enrolls() {
    let h = FakeHost::new();
    let first = install(&h, &args(), &quiet).unwrap();
    let data = h.io_path(&h.layout().data_dir);
    for f in ["agent.key", "state.pb", "seen.txt", "local.pb"] {
        std::fs::write(data.join(f), "x").unwrap();
    }
    let id = reenroll(&h, None, Secret::new("NEW-CODE"), None, &quiet).unwrap();
    assert_ne!(id, first.computer_id);
    for f in ["agent.key", "state.pb", "seen.txt", "local.pb"] {
        assert!(!data.join(f).exists(), "{f} survived");
    }
    assert_eq!(config(&h).server, "wss://srv.school:8443");
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn status_lists_problems() {
    let h = FakeHost::new();
    let s = status(&h);
    assert!(!s.installed);
    assert!(
        s.problems.iter().any(|p| p.contains("not installed")),
        "{:?}",
        s.problems
    );
    install(&h, &args(), &quiet).unwrap();
    let s = status(&h);
    assert!(s.installed);
    assert!(s.problems.is_empty(), "{:?}", s.problems);
    assert_eq!(s.local_port, 11500);
    assert!(s.firewall_rule);
    h.stop_service(SERVICE, std::time::Duration::from_secs(1))
        .unwrap();
    h.delete_firewall_rule(FIREWALL_RULE).unwrap();
    h.with_legacy();
    let s = status(&h);
    assert_eq!(s.service, Some(ServiceState::Stopped));
    assert_eq!(s.problems.len(), 3, "{:?}", s.problems);
    assert!(s.legacy_agent);
}

#[test]
fn every_mutating_op_needs_admin() {
    let h = FakeHost::new();
    h.not_elevated();
    assert!(matches!(
        install(&h, &args(), &quiet),
        Err(OpError::NeedsAdmin)
    ));
    assert!(matches!(update(&h, &quiet), Err(OpError::NeedsAdmin)));
    assert!(matches!(
        uninstall(&h, false, &quiet),
        Err(OpError::NeedsAdmin)
    ));
    assert!(matches!(
        reenroll(&h, None, Secret::new("X"), None, &quiet),
        Err(OpError::NeedsAdmin)
    ));
    assert!(h.calls().is_empty(), "{:?}", h.calls());
    assert_eq!(
        OpError::NeedsAdmin.to_string(),
        "Run this from an elevated prompt (Run as administrator)."
    );
}

#[test]
fn code_never_reaches_progress_or_errors() {
    let h = FakeHost::new();
    let seen = std::sync::Mutex::new(Vec::new());
    let progress = |m: &str| seen.lock().unwrap().push(m.to_string());
    install(&h, &args(), &progress).unwrap();
    h.fail_enroll(Some("server said no"));
    let _ = reenroll(&h, None, Secret::new("ABCD-EFGH"), None, &progress).map_err(|e| {
        seen.lock().unwrap().push(e.to_string());
    });
    let all = seen.lock().unwrap().join("\n");
    assert!(!all.contains("ABCD-EFGH"), "{all}");
    assert!(!h.calls().join("\n").contains("ABCD-EFGH"));
}

#[test]
fn host_failure_is_reported() {
    let h = FakeHost::new();
    h.fail_on(
        Method::SetFirewall,
        HostError::Failed("netsh failed".into()),
    );
    match install(&h, &args(), &quiet) {
        Err(OpError::Host(HostError::Failed(m))) => assert!(m.contains("netsh")),
        other => panic!("{other:?}"),
    }
}

// ---- final review (stage 3c) ----

#[test]
fn enroll_failure_without_message_shows_the_exit_code() {
    let h = FakeHost::new();
    h.fail_enroll(Some(""));
    match install(&h, &args(), &quiet) {
        Err(OpError::Enroll(m)) => assert!(m.contains("exit code 1"), "{m:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn enrollment_code_goes_to_the_agent_by_environment() {
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    let enroll = h
        .calls()
        .into_iter()
        .find(|c| c.contains("enroll"))
        .unwrap();
    assert!(!enroll.contains("--code"), "{enroll}");
    assert!(enroll.contains("env GUARDIAN_AGENT_CODE"), "{enroll}");
    assert_eq!(h.last_enroll_code().as_deref(), Some("ABCD-EFGH"));
}

#[test]
fn rollback_stops_a_restarted_agent_before_restoring() {
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    std::fs::write(h.io_path(&h.layout().agent_exe), "old agent").unwrap();
    h.agent_wont_start_if(NEW_AGENT);
    // The service manager restarts a failing agent by itself.
    h.restarts_on_failure();
    assert!(update(&h, &quiet).is_err());
    assert_eq!(installed_agent(&h), "old agent");
    let calls = h.calls();
    let failed_start = calls
        .iter()
        .rposition(|c| c == "start_service GuardianAgent")
        .unwrap();
    let stop_before = calls[..failed_start]
        .iter()
        .rposition(|c| c == "stop_service GuardianAgent")
        .unwrap();
    let start_fail = calls
        .iter()
        .position(|c| c == "start_service GuardianAgent (failed)")
        .unwrap();
    assert!(start_fail < stop_before, "{calls:?}");
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[cfg(unix)]
fn running_as_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[cfg(unix)]
#[test]
fn uninstall_retries_while_files_are_locked() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return; // root ignores the permission that stands in for a lock
    }
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    let dir = h.io_path(&h.layout().program_dir);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let unlock = dir.clone();
    h.on_wait(Box::new(move || {
        let _ = std::fs::set_permissions(&unlock, std::fs::Permissions::from_mode(0o755));
    }));
    uninstall(&h, false, &quiet).unwrap();
    assert!(!dir.exists());
}

#[test]
fn no_start_waits_for_the_service_before_stopping() {
    let h = FakeHost::new();
    h.starts_slowly();
    let mut a = args();
    a.no_start = true;
    install(&h, &a, &quiet).unwrap();
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Stopped));
}

#[test]
fn install_waits_until_the_new_service_runs() {
    let h = FakeHost::new();
    h.starts_slowly();
    install(&h, &args(), &quiet).unwrap();
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[test]
fn failed_reenroll_restores_the_old_identity() {
    let h = FakeHost::new();
    let first = install(&h, &args(), &quiet).unwrap();
    let data = h.io_path(&h.layout().data_dir);
    std::fs::write(data.join("agent.key"), "old key").unwrap();
    std::fs::write(data.join("state.pb"), "old state").unwrap();
    h.fail_enroll(Some("invalid enrollment code"));
    assert!(reenroll(&h, None, Secret::new("BAD"), None, &quiet).is_err());
    assert_eq!(config(&h).computer_id, Some(first.computer_id));
    assert_eq!(
        std::fs::read_to_string(data.join("agent.key")).unwrap(),
        "old key"
    );
    assert_eq!(
        std::fs::read_to_string(data.join("state.pb")).unwrap(),
        "old state"
    );
    assert_eq!(h.service_state(SERVICE), Some(ServiceState::Running));
}

#[cfg(unix)]
#[test]
fn status_without_admin_does_not_claim_unenrolled() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let h = FakeHost::new();
    install(&h, &args(), &quiet).unwrap();
    let cfg = h.io_path(&h.layout().config);
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o000)).unwrap();
    h.not_elevated();
    let s = status(&h);
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        !s.problems.iter().any(|p| p.contains("not enrolled")),
        "{:?}",
        s.problems
    );
    assert!(s.needs_admin);
}

#[test]
fn missing_binaries_do_not_remove_legacy() {
    let h = FakeHost::new();
    h.with_legacy();
    h.without_bundle();
    let mut a = args();
    a.remove_legacy = true;
    assert!(matches!(
        install(&h, &a, &quiet),
        Err(OpError::MissingBinaries(_))
    ));
    assert_eq!(h.service_state(LEGACY_SERVICE), Some(ServiceState::Running));
    assert!(h.io_path(&h.layout().legacy_dir).exists());
}
