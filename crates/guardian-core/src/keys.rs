//! ed25519 identities and the challenge signature used by agents and consoles.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use zeroize::Zeroizing;

use crate::random::random_bytes;

/// Who is proving its identity; the label keeps signatures from one context
/// from being replayed in another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthContext {
    /// An agent answering the WebSocket challenge.
    Agent,
    /// A teacher console answering a local-mode challenge.
    Console,
}

impl AuthContext {
    fn label(self) -> &'static [u8] {
        match self {
            AuthContext::Agent => b"guardian/v1/agent-auth",
            AuthContext::Console => b"guardian/v1/console-auth",
        }
    }
}

/// Why a key or signature was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// Wrong length or not a valid curve point.
    #[error("malformed key or signature")]
    Malformed,
    /// The signature does not verify.
    #[error("bad signature")]
    BadSignature,
}

fn challenge_message(ctx: AuthContext, subject: u64, nonce: &[u8; 32]) -> Vec<u8> {
    let label = ctx.label();
    let mut m = Vec::with_capacity(label.len() + 32 + 8);
    m.extend_from_slice(label);
    m.extend_from_slice(nonce);
    m.extend_from_slice(&subject.to_le_bytes());
    m
}

/// A private ed25519 key. `Debug` shows only the public half.
pub struct KeyPair(SigningKey);

impl KeyPair {
    /// A fresh key from the OS CSPRNG.
    pub fn generate() -> KeyPair {
        let seed = Zeroizing::new(random_bytes::<32>());
        KeyPair::from_seed(&seed)
    }

    /// Rebuilds a key from its 32-byte seed.
    pub fn from_seed(seed: &[u8; 32]) -> KeyPair {
        KeyPair(SigningKey::from_bytes(seed))
    }

    /// The 32-byte seed (for storing the key on disk).
    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The public half.
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key().to_bytes())
    }

    /// Signs `label || nonce || subject_le`.
    pub fn sign_challenge(&self, ctx: AuthContext, subject: u64, nonce: &[u8; 32]) -> Signature {
        Signature(
            self.0
                .sign(&challenge_message(ctx, subject, nonce))
                .to_bytes(),
        )
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPair")
            .field("public", &self.public())
            .finish_non_exhaustive()
    }
}

/// A public ed25519 key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// Accepts exactly 32 bytes that decode to a valid point.
    pub fn from_bytes(b: &[u8]) -> Result<PublicKey, KeyError> {
        let arr: [u8; 32] = b.try_into().map_err(|_| KeyError::Malformed)?;
        VerifyingKey::from_bytes(&arr).map_err(|_| KeyError::Malformed)?;
        Ok(PublicKey(arr))
    }

    /// The raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Verifies (strictly) a signature made by [`KeyPair::sign_challenge`].
    pub fn verify_challenge(
        &self,
        ctx: AuthContext,
        subject: u64,
        nonce: &[u8; 32],
        sig: &Signature,
    ) -> Result<(), KeyError> {
        let vk = VerifyingKey::from_bytes(&self.0).map_err(|_| KeyError::Malformed)?;
        let s = ed25519_dalek::Signature::from_bytes(&sig.0);
        vk.verify_strict(&challenge_message(ctx, subject, nonce), &s)
            .map_err(|_| KeyError::BadSignature)
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey(")?;
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

/// An ed25519 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; 64]);

impl Signature {
    /// Accepts exactly 64 bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Signature, KeyError> {
        Ok(Signature(b.try_into().map_err(|_| KeyError::Malformed)?))
    }

    /// The raw 64 bytes.
    pub fn to_bytes(&self) -> [u8; 64] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_challenge_verifies() {
        let k = KeyPair::generate();
        let n = [3u8; 32];
        let s = k.sign_challenge(AuthContext::Agent, 7, &n);
        assert!(
            k.public()
                .verify_challenge(AuthContext::Agent, 7, &n, &s)
                .is_ok()
        );
    }

    #[test]
    fn signature_is_bound_to_context_subject_and_nonce() {
        let k = KeyPair::generate();
        let n = [3u8; 32];
        let s = k.sign_challenge(AuthContext::Agent, 7, &n);
        let pk = k.public();
        assert_eq!(
            pk.verify_challenge(AuthContext::Console, 7, &n, &s),
            Err(KeyError::BadSignature)
        );
        assert_eq!(
            pk.verify_challenge(AuthContext::Agent, 8, &n, &s),
            Err(KeyError::BadSignature)
        );
        assert_eq!(
            pk.verify_challenge(AuthContext::Agent, 7, &[4u8; 32], &s),
            Err(KeyError::BadSignature)
        );
        assert_eq!(
            KeyPair::generate()
                .public()
                .verify_challenge(AuthContext::Agent, 7, &n, &s),
            Err(KeyError::BadSignature)
        );
    }

    #[test]
    fn seed_roundtrip_keeps_identity() {
        let k = KeyPair::generate();
        let k2 = KeyPair::from_seed(&k.seed());
        assert_eq!(k.public(), k2.public());
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        assert_eq!(PublicKey::from_bytes(&[0u8; 31]), Err(KeyError::Malformed));
        assert_eq!(Signature::from_bytes(&[0u8; 63]), Err(KeyError::Malformed));
    }

    #[test]
    fn debug_does_not_print_the_seed() {
        let k = KeyPair::from_seed(&[0xAB; 32]);
        assert!(!format!("{k:?}").contains("abab"));
    }
}
