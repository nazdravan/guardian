/// Windows tools print UTF-16LE when their output is redirected in some locales; detect it.
pub fn decode_output(b: &[u8]) -> String {
    let utf16 = b.starts_with(&[0xFF, 0xFE]) || (b.len() >= 4 && b.len().is_multiple_of(2) && b[1] == 0 && b[3] == 0);
    if utf16 {
        let body = b.strip_prefix(&[0xFF, 0xFE][..]).unwrap_or(b);
        let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(b).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_output_cases() {
        assert_eq!(decode_output(b"abc"), "abc");
        let utf16: Vec<u8> = "\u{feff}<Event/>".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_output(&utf16), "<Event/>");
        let no_bom: Vec<u8> = "<Event/>".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_output(&no_bom), "<Event/>");
    }
}
