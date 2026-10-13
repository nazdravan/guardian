use std::path::PathBuf;

pub const TABS: [&str; 4] = ["diagnostics", "configuration", "processes", "log"];
pub const USAGE: &str = "Usage:\n  Guardian.exe [--tab diagnostics|configuration|processes|log] [--dir <agent folder>]\n  Guardian.exe report [--json] [--dir <agent folder>] [--offline]\n\nExit codes of report: 0 protection works, 10 warnings, 11 errors, 2 usage error.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Window { tab: Option<String>, dir: Option<PathBuf> },
    Report { json: bool, dir: Option<PathBuf>, offline: bool },
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter().peekable();
    let report = it.peek().is_some_and(|a| *a == "report");
    if report {
        it.next();
    }
    let (mut tab, mut dir, mut json, mut offline) = (None, None, false, false);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(it.next().ok_or("--dir needs a folder")?)),
            "--tab" if !report => {
                let t = it.next().ok_or("--tab needs a name")?.to_lowercase();
                if !TABS.contains(&t.as_str()) {
                    return Err(format!("unknown tab {t}"));
                }
                tab = Some(t);
            }
            "--json" if report => json = true,
            "--offline" if report => offline = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(if report { Command::Report { json, dir, offline } } else { Command::Window { tab, dir } })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_cases() {
        assert_eq!(parse(&a(&[])), Ok(Command::Window { tab: None, dir: None }));
        assert_eq!(parse(&a(&["--tab", "configuration"])), Ok(Command::Window { tab: Some("configuration".into()), dir: None }));
        assert_eq!(
            parse(&a(&["report", "--json", "--dir", "X", "--offline"])),
            Ok(Command::Report { json: true, dir: Some("X".into()), offline: true })
        );
        assert!(parse(&a(&["report", "--bogus"])).is_err());
        assert!(parse(&a(&["--tab"])).is_err());
        assert!(parse(&a(&["--tab", "nope"])).is_err());
    }
}
