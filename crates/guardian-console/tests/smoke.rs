#[test]
fn version_is_the_workspace_version() {
    assert_eq!(guardian_console::VERSION, env!("CARGO_PKG_VERSION"));
}
