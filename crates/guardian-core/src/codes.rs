//! Enrollment and bootstrap codes.

use crate::random::random_bytes;

/// Characters a code may contain (no 0/O, 1/I/L, U).
pub const CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRSTVWXYZ23456789";
/// Length of every code.
pub const CODE_LEN: usize = 8;

/// A random code of [`CODE_LEN`] characters from [`CODE_ALPHABET`], without modulo bias.
pub fn generate_code() -> String {
    let alphabet = CODE_ALPHABET.as_bytes();
    let n = alphabet.len();
    // Largest multiple of n that fits in a byte; bytes above it are discarded.
    let limit = (256 / n) * n;
    let mut out = String::with_capacity(CODE_LEN);
    while out.len() < CODE_LEN {
        for b in random_bytes::<16>() {
            let b = usize::from(b);
            if b < limit && out.len() < CODE_LEN {
                out.push(char::from(alphabet[b % n]));
            }
        }
    }
    out
}

/// Uppercases and drops spaces and dashes, so `" abcd-2345 "` equals `"ABCD2345"`.
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_use_the_alphabet() {
        let c = generate_code();
        assert_eq!(c.len(), 8);
        assert!(c.chars().all(|ch| CODE_ALPHABET.contains(ch)));
    }

    #[test]
    fn normalize_ignores_case_spaces_and_dashes() {
        assert_eq!(normalize_code(" abcd-2345 "), "ABCD2345");
    }
}
