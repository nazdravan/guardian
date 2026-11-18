#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_console::settings::{ConsoleSettings, Mode, normalize_pin, validate_server_url};

#[test]
fn missing_file_is_default() {
    let d = tempfile::tempdir().unwrap();
    let s = ConsoleSettings::load(&d.path().join("none.toml")).unwrap();
    assert_eq!(s, ConsoleSettings::default());
    assert!(s.mode.is_none() && s.token().is_none());
}

#[test]
fn remote_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("g/console.toml");
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Remote {
        server: "https://srv.school:8443".into(),
        cert_sha256: Some("ab".repeat(32)),
    });
    s.login = Some("prof".into());
    s.set_token(Some("tok-123".into()));
    s.save(&p).unwrap();
    let back = ConsoleSettings::load(&p).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.token(), Some("tok-123"));
}

#[test]
fn embedded_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("console.toml");
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Embedded { port: 9443 });
    s.save(&p).unwrap();
    assert_eq!(ConsoleSettings::load(&p).unwrap(), s);
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("mode = \"embedded\""), "{text}");
}

#[test]
fn token_is_hidden_in_debug() {
    let mut s = ConsoleSettings::default();
    s.set_token(Some("very-secret-token".into()));
    assert!(!format!("{s:?}").contains("very-secret-token"));
}

#[cfg(unix)]
#[test]
fn file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("console.toml");
    ConsoleSettings::default().save(&p).unwrap();
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn url_validation() {
    assert_eq!(
        validate_server_url("https://srv.school").unwrap(),
        "https://srv.school:8443"
    );
    assert_eq!(
        validate_server_url(" https://10.0.0.5:443 ").unwrap(),
        "https://10.0.0.5:443"
    );
    assert!(
        validate_server_url("http://x")
            .unwrap_err()
            .contains("https://")
    );
    assert!(validate_server_url("").is_err());
    assert!(validate_server_url("https://a b").is_err());
    assert!(validate_server_url("srv.school").is_err());
}

#[test]
fn pin_normalization() {
    let colon = (0..32).map(|_| "AB").collect::<Vec<_>>().join(":");
    assert_eq!(normalize_pin(&colon).unwrap(), Some("ab".repeat(32)));
    assert_eq!(
        normalize_pin(&format!(" {} ", "Cd ".repeat(32))).unwrap(),
        Some("cd".repeat(32))
    );
    assert!(normalize_pin(&"a".repeat(63)).is_err());
    assert!(normalize_pin(&"g".repeat(64)).is_err());
    assert_eq!(normalize_pin("").unwrap(), None);
    assert_eq!(normalize_pin("   ").unwrap(), None);
}
