#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;

use guardian_core::enforce::ProcessInfo;
use guardian_core::trusted::TrustedRoots;
use guardian_setup::model::{Arch, Secret, Whitelist, generated_whitelist, layout, os_arch};

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let m: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| m.get(k).cloned()
}

#[test]
fn x86_and_x64_from_env() {
    assert_eq!(
        os_arch(&env(&[("PROCESSOR_ARCHITECTURE", "AMD64")])),
        Arch::X64
    );
    assert_eq!(
        os_arch(&env(&[("PROCESSOR_ARCHITECTURE", "x86")])),
        Arch::X86
    );
    // A 32-bit process on 64-bit Windows.
    assert_eq!(
        os_arch(&env(&[
            ("PROCESSOR_ARCHITECTURE", "x86"),
            ("PROCESSOR_ARCHITEW6432", "AMD64")
        ])),
        Arch::X64
    );
}

#[test]
fn arm64_uses_x64() {
    assert_eq!(
        os_arch(&env(&[("PROCESSOR_ARCHITECTURE", "ARM64")])),
        Arch::X64
    );
    assert_eq!(
        os_arch(&env(&[
            ("PROCESSOR_ARCHITECTURE", "x86"),
            ("PROCESSOR_ARCHITEW6432", "ARM64")
        ])),
        Arch::X64
    );
}

#[test]
fn layout_on_64_bit_windows_from_a_32_bit_process() {
    let l = layout(&env(&[
        ("ProgramFiles", r"C:\Program Files (x86)"),
        ("ProgramW6432", r"C:\Program Files"),
        ("ProgramData", r"C:\ProgramData"),
        ("SystemRoot", r"C:\Windows"),
    ]));
    assert_eq!(l.program_dir.as_str(), r"C:\Program Files\Guardian");
    assert_eq!(
        l.agent_exe.as_str(),
        r"C:\Program Files\Guardian\guardian-agent.exe"
    );
    assert_eq!(
        l.worker_exe.as_str(),
        r"C:\Program Files\Guardian\guardian-worker.exe"
    );
    assert_eq!(
        l.config.as_str(),
        r"C:\ProgramData\Guardian Agent\agent.toml"
    );
    assert_eq!(l.data_dir.as_str(), r"C:\ProgramData\Guardian Agent");
    assert_eq!(
        l.legacy_dir.as_str(),
        r"C:\Windows\System32\ProcSentinel\agent"
    );
    assert_eq!(l.lock.as_str(), r"C:\ProgramData\Guardian\setup.lock");
}

#[test]
fn layout_on_32_bit_windows() {
    let l = layout(&env(&[("ProgramFiles", r"D:\Programs")]));
    assert_eq!(l.program_dir.as_str(), r"D:\Programs\Guardian");
    // Defaults when the variables are missing.
    assert_eq!(l.data_dir.as_str(), r"C:\ProgramData\Guardian Agent");
    assert_eq!(
        l.legacy_dir.as_str(),
        r"C:\Windows\System32\ProcSentinel\agent"
    );
    let empty = layout(&env(&[]));
    assert_eq!(empty.program_dir.as_str(), r"C:\Program Files\Guardian");
}

#[test]
fn layout_has_no_sysnative() {
    let l = layout(&env(&[
        ("PROCESSOR_ARCHITEW6432", "AMD64"),
        ("SystemRoot", r"C:\Windows"),
    ]));
    for p in [
        &l.program_dir,
        &l.agent_exe,
        &l.worker_exe,
        &l.config,
        &l.data_dir,
        &l.legacy_dir,
        &l.lock,
    ] {
        assert!(!p.as_str().to_lowercase().contains("sysnative"), "{p:?}");
    }
}

#[test]
fn bundled_binaries_follow_the_arch() {
    let (a, w) = guardian_setup::model::bundled_bins(&PathBuf::from("/x/agent-windows"), Arch::X64);
    assert_eq!(
        a,
        PathBuf::from("/x/agent-windows/bin/x64/guardian-agent.exe")
    );
    assert_eq!(
        w,
        PathBuf::from("/x/agent-windows/bin/x64/guardian-worker.exe")
    );
    let (a, _) = guardian_setup::model::bundled_bins(&PathBuf::from("/x"), Arch::X86);
    assert_eq!(a, PathBuf::from("/x/bin/x86/guardian-agent.exe"));
}

#[test]
fn whitelist_parses_bom_crlf_and_comments() {
    let (w, warnings) = Whitelist::parse(
        "\u{feff}# Guardian whitelist\r\nNotepad.exe\r\n\r\nbad/path\r\nvlc.exe\n",
    );
    assert_eq!(w.names(), vec!["notepad.exe", "vlc.exe"]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("bad/path"));
}

#[test]
fn whitelist_add_rejects_paths() {
    let mut w = Whitelist::default();
    assert!(w.add("C:\\x\\game.exe").is_err());
    assert_eq!(w.add("Game.exe"), Ok(true));
    assert_eq!(w.add("game.exe"), Ok(false));
    assert!(w.remove("GAME.EXE"));
    assert!(!w.remove("game.exe"));
}

#[test]
fn whitelist_render_round_trips() {
    let (w, _) = Whitelist::parse("# keep me\n# and me\nzeta.exe\nalpha.exe\n");
    let text = w.render();
    assert!(text.starts_with("# keep me\n# and me\n"), "{text}");
    let (back, warnings) = Whitelist::parse(&text);
    assert!(warnings.is_empty());
    assert_eq!(back.names(), vec!["alpha.exe", "zeta.exe"]);
    assert_eq!(back, w);
}

fn p(name: &str, exe: &str) -> ProcessInfo {
    ProcessInfo {
        pid: 1,
        name: name.into(),
        exe_path: Some(PathBuf::from(exe)),
        start_marker: 0,
    }
}

#[test]
fn generated_whitelist_skips_user_apps_and_protected() {
    let roots = TrustedRoots {
        recursive: vec![PathBuf::from(r"C:\Windows")],
        ..TrustedRoots::default()
    };
    let procs = vec![
        p(
            "RtkAudUService64.exe",
            r"C:\Program Files\Realtek\RtkAudUService64.exe",
        ),
        p(
            "chrome.exe",
            r"C:\Users\pupil\AppData\Local\Google\chrome.exe",
        ),
        p(
            "Teams.exe",
            r"c:\users\Teacher\AppData\Local\Microsoft\Teams.exe",
        ),
        p("svchost.exe", r"C:\Windows\System32\svchost.exe"),
        p("igfxEM.exe", r"C:\Windows\System32\DriverStore\igfxEM.exe"),
        ProcessInfo {
            pid: 2,
            name: "nopath.exe".into(),
            exe_path: None,
            start_marker: 0,
        },
    ];
    let w = generated_whitelist(&procs, &roots);
    assert_eq!(
        w.names(),
        vec!["igfxem.exe", "nopath.exe", "rtkauduservice64.exe"]
    );
}

#[test]
fn secret_never_prints() {
    let s = Secret::new("ABCD-EFGH");
    assert_eq!(format!("{s:?}"), "***");
    assert_eq!(serde_json::to_string(&s).unwrap(), "\"***\"");
    assert_eq!(s.expose(), "ABCD-EFGH");
}
