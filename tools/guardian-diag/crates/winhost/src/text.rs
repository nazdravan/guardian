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

/// One argument as the Windows command line (CommandLineToArgvW rules) must see it.
pub fn win_quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.contains(|c: char| c.is_whitespace() || c == '"') {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// A PowerShell single-quoted literal; every character PowerShell treats as a single quote is doubled.
pub fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// The PowerShell command that starts `exe` with `args` under UAC. Arguments are first quoted for the
/// Windows command line (Start-Process joins them with spaces), then for PowerShell.
pub fn runas_script(exe: &str, args: &[String]) -> String {
    let list = if args.is_empty() {
        String::new()
    } else {
        format!(" -ArgumentList {}", args.iter().map(|a| ps_quote(&win_quote_arg(a))).collect::<Vec<_>>().join(","))
    };
    format!("Start-Process -FilePath {}{} -Verb RunAs", ps_quote(exe), list)
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

    #[test]
    fn runas_script_quotes_arguments() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(runas_script(r"C:\g.exe", &[]), r"Start-Process -FilePath 'C:\g.exe' -Verb RunAs");
        assert_eq!(runas_script(r"C:\g.exe", &a(&["report", "--x"])), r"Start-Process -FilePath 'C:\g.exe' -ArgumentList 'report','--x' -Verb RunAs");
        assert_eq!(
            runas_script("g", &a(&["--dir", r"E:\My Flash"])),
            r#"Start-Process -FilePath 'g' -ArgumentList '--dir','"E:\My Flash"' -Verb RunAs"#
        );
        assert_eq!(win_quote_arg(r"E:\My Flash\"), r#""E:\My Flash\\""#);
        assert_eq!(ps_quote("Nicolae\u{2019}s USB"), "'Nicolae\u{2019}\u{2019}s USB'");
        assert_eq!(
            runas_script("g", &a(&["Nicolae\u{2019}s USB"])),
            "Start-Process -FilePath 'g' -ArgumentList '\"Nicolae\u{2019}\u{2019}s USB\"' -Verb RunAs"
        );
        assert_eq!(win_quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(win_quote_arg(r#"a\"b c"#), r#""a\\\"b c""#);
        assert_eq!(ps_quote("it's"), "'it''s'");
        assert_eq!(win_quote_arg(""), "\"\"");
    }
}
