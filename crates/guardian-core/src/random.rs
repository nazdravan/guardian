//! The one source of randomness: the OS CSPRNG.

/// Returns `N` bytes from the OS CSPRNG.
///
/// A machine without a working CSPRNG cannot run Guardian safely (keys, tokens and
/// nonces would be predictable), so this is the single place allowed to abort.
#[allow(clippy::expect_used)]
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("the OS random number generator is unavailable");
    buf
}
