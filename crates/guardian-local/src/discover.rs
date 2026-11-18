//! Signed UDP discovery: a console broadcasts a request signed with its key;
//! agents that trust that console answer with a reply signed with theirs.
//! Unknown consoles get no answer, and forged replies are ignored.

use guardian_core::keys::{KeyPair, PublicKey, SignContext, Signature};
use guardian_core::random::random_bytes;
use guardian_proto::v1::{DiscoverReply, DiscoverRequest};

/// Larger datagrams are ignored.
pub const MAX_DATAGRAM: usize = 1024;

fn request_msg(console_id: i64, nonce: &[u8]) -> Vec<u8> {
    let mut m = console_id.to_le_bytes().to_vec();
    m.extend_from_slice(nonce);
    m
}

fn reply_msg(nonce: &[u8], computer_id: u64, port: u32, online: bool) -> Vec<u8> {
    let mut m = nonce.to_vec();
    m.extend_from_slice(&computer_id.to_le_bytes());
    m.extend_from_slice(&port.to_le_bytes());
    m.push(u8::from(online));
    m
}

/// A signed request with a fresh 16-byte nonce.
pub fn request(me: &KeyPair, console_id: i64) -> (DiscoverRequest, [u8; 16]) {
    let nonce = random_bytes::<16>();
    let sig = me.sign(SignContext::Discover, &request_msg(console_id, &nonce));
    (
        DiscoverRequest {
            console_id,
            nonce: nonce.to_vec(),
            signature: sig.to_bytes().to_vec(),
        },
        nonce,
    )
}

/// The console id when the request is signed by that trusted console.
pub fn verify_request(r: &DiscoverRequest, trusted: &[(i64, PublicKey)]) -> Option<i64> {
    if r.nonce.len() != 16 {
        return None;
    }
    let sig = Signature::from_bytes(&r.signature).ok()?;
    let (_, key) = trusted.iter().find(|(id, _)| *id == r.console_id)?;
    key.verify(
        SignContext::Discover,
        &request_msg(r.console_id, &r.nonce),
        &sig,
    )
    .ok()?;
    Some(r.console_id)
}

/// The agent's signed answer.
pub fn reply(
    me: &KeyPair,
    computer_id: u64,
    nonce: &[u8],
    port: u16,
    online: bool,
) -> DiscoverReply {
    let port = u32::from(port);
    let sig = me.sign(
        SignContext::DiscoverReply,
        &reply_msg(nonce, computer_id, port, online),
    );
    DiscoverReply {
        computer_id,
        nonce: nonce.to_vec(),
        port,
        online,
        signature: sig.to_bytes().to_vec(),
    }
}

/// Whether `r` answers `nonce` and is signed by `agent`.
pub fn verify_reply(r: &DiscoverReply, nonce: &[u8; 16], agent: &PublicKey) -> bool {
    if r.nonce.as_slice() != nonce.as_slice() || u16::try_from(r.port).is_err() {
        return false;
    }
    let Ok(sig) = Signature::from_bytes(&r.signature) else {
        return false;
    };
    agent
        .verify(
            SignContext::DiscoverReply,
            &reply_msg(&r.nonce, r.computer_id, r.port, r.online),
            &sig,
        )
        .is_ok()
}
