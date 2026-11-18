#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_core::keys::KeyPair;
use guardian_local::discover::{reply, request, verify_reply, verify_request};

#[test]
fn request_round_trip() {
    let c = KeyPair::generate();
    let other = KeyPair::generate();
    let (r, nonce) = request(&c, 4);
    assert_eq!(r.nonce, nonce.to_vec());
    assert_eq!(
        verify_request(&r, &[(1, other.public()), (4, c.public())]),
        Some(4)
    );
    assert_eq!(verify_request(&r, &[(4, other.public())]), None);
    assert_eq!(verify_request(&r, &[]), None);
}

#[test]
fn tampered_request_is_rejected() {
    let c = KeyPair::generate();
    let (mut r, _) = request(&c, 4);
    r.nonce[0] ^= 1;
    assert_eq!(verify_request(&r, &[(4, c.public())]), None);
    let (mut r2, _) = request(&c, 4);
    r2.console_id = 5;
    assert_eq!(verify_request(&r2, &[(5, c.public())]), None);
    let (mut r3, _) = request(&c, 4);
    r3.signature.truncate(10);
    assert_eq!(verify_request(&r3, &[(4, c.public())]), None);
}

#[test]
fn reply_round_trip() {
    let a = KeyPair::generate();
    let nonce = [7u8; 16];
    let r = reply(&a, 12, &nonce, 11500, false);
    assert!(verify_reply(&r, &nonce, &a.public()));
    let mut changed = r.clone();
    changed.port = 11501;
    assert!(!verify_reply(&changed, &nonce, &a.public()));
    let mut flipped = r.clone();
    flipped.online = true;
    assert!(!verify_reply(&flipped, &nonce, &a.public()));
}

#[test]
fn reply_signed_by_other_key_is_rejected() {
    let a = KeyPair::generate();
    let nonce = [7u8; 16];
    let r = reply(&KeyPair::generate(), 12, &nonce, 11500, false);
    assert!(!verify_reply(&r, &nonce, &a.public()));
}

#[test]
fn reply_for_other_nonce_is_rejected() {
    let a = KeyPair::generate();
    let r = reply(&a, 12, &[7u8; 16], 11500, false);
    assert!(!verify_reply(&r, &[8u8; 16], &a.public()));
}
