//! Pure state shared by the tabs.

use guardian_core::edit::{EnvEdit, WhitelistEdit};
use guardian_core::snapshot::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Diagnostics,
    Configuration,
    Processes,
    Log,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Diagnostics, Tab::Configuration, Tab::Processes, Tab::Log];
    pub fn from_arg(s: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.arg() == s)
    }
    pub fn arg(self) -> &'static str {
        match self {
            Tab::Diagnostics => "diagnostics",
            Tab::Configuration => "configuration",
            Tab::Processes => "processes",
            Tab::Log => "log",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Tab::Diagnostics => "Diagnostics",
            Tab::Configuration => "Configuration",
            Tab::Processes => "Processes",
            Tab::Log => "Log",
        }
    }
}

/// The edit draft (spec R14): built from a snapshot, compared against it for the dirty count.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub env: EnvEdit,
    pub whitelist: WhitelistEdit,
    /// The text in the BINDING_TOKEN password field; folded into env.binding_token on Save.
    pub token_field: String,
}

impl Draft {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        Draft { env: EnvEdit::from_snapshot(s), whitelist: WhitelistEdit::from_snapshot(s), token_field: String::new() }
    }
    /// How many things differ from what is on disk (for "N unsaved changes").
    pub fn dirty_count(&self, s: &Snapshot) -> usize {
        let base = Draft::from_snapshot(s);
        let env = guardian_core::edit::plan::env_changes(s, &self.effective_env()).len();
        let added = self.whitelist.entries.iter().filter(|e| !base.whitelist.entries.contains(e)).count();
        let removed = base.whitelist.entries.iter().filter(|e| !self.whitelist.entries.contains(e)).count();
        env + added + removed
    }
    /// env with the password field applied: non-empty field = Set, otherwise whatever Remove/Keep says.
    pub fn effective_env(&self) -> EnvEdit {
        let mut e = self.env.clone();
        if !self.token_field.is_empty() {
            e.binding_token = guardian_core::edit::TokenEdit::Set(self.token_field.clone());
        }
        e
    }
}

/// Spec R3/R5: who may edit, and why not.
pub fn edit_lock(s: &Snapshot) -> Option<&'static str> {
    if !s.elevated && !s.is_manual_dir() { Some("Run as administrator to edit") } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }

    #[test]
    fn tabs_roundtrip() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_arg(t.arg()), Some(t));
        }
    }

    #[test]
    fn dirty_count_counts_env_and_whitelist() {
        let s = snap();
        let mut d = Draft::from_snapshot(&s);
        assert_eq!(d.dirty_count(&s), 0);
        d.env.check_interval = "15".into();
        d.whitelist.entries.push("winword.exe".into());
        d.whitelist.entries.retain(|e| e != "calc.exe");
        d.token_field = "tok".into();
        assert_eq!(d.dirty_count(&s), 4);
    }

    #[test]
    fn edit_locked_when_not_elevated() {
        let mut s = snap();
        s.elevated = false;
        assert_eq!(edit_lock(&s), Some("Run as administrator to edit"));
        s.elevated = true;
        assert_eq!(edit_lock(&s), None);
    }
}
