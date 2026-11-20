#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_platform::protected_acl_args;

#[test]
fn acl_args_set_inheritable_grants_on_the_folder_only() {
    let a = protected_acl_args(&["S-1-5-21-1-2-3-1001"]);
    // Not recursive: files inherit from the folder. With /T the folder's
    // inheritance flags were applied to files too, which left existing files
    // (agent.key, cert.pem) without any access on a second run.
    assert!(!a.iter().any(|s| s.eq_ignore_ascii_case("/T")), "{a:?}");
    assert_eq!(
        a,
        vec![
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-21-1-2-3-1001:(OI)(CI)F",
            "/C",
            "/Q",
        ]
    );
    assert_eq!(protected_acl_args(&[]).len(), 7);
}
