//! Profiles, application names and the effective-profile rule.

use std::collections::BTreeSet;

use guardian_proto::v1 as pb;

/// What a profile does with running processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Nothing is killed.
    Free,
    /// Listed applications are killed.
    Blacklist,
    /// Everything not listed (and not protected) is killed.
    Whitelist,
}

/// A protobuf `Mode` that has no meaning (`MODE_UNSPECIFIED` or an unknown value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unspecified or unknown mode")]
pub struct UnknownMode;

impl TryFrom<pb::Mode> for Mode {
    type Error = UnknownMode;
    fn try_from(m: pb::Mode) -> Result<Self, Self::Error> {
        match m {
            pb::Mode::Free => Ok(Mode::Free),
            pb::Mode::Blacklist => Ok(Mode::Blacklist),
            pb::Mode::Whitelist => Ok(Mode::Whitelist),
            pb::Mode::Unspecified => Err(UnknownMode),
        }
    }
}

impl TryFrom<i32> for Mode {
    type Error = UnknownMode;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        pb::Mode::try_from(v)
            .map_err(|_| UnknownMode)
            .and_then(Mode::try_from)
    }
}

impl From<Mode> for pb::Mode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Free => pb::Mode::Free,
            Mode::Blacklist => pb::Mode::Blacklist,
            Mode::Whitelist => pb::Mode::Whitelist,
        }
    }
}

/// Database id of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileId(pub i64);

/// Why an application name was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AppNameError {
    /// Not 3–64 characters after trimming.
    #[error("application name must be 3 to 64 characters")]
    Length,
    /// Contains a character outside `[A-Za-z0-9 ._-]`.
    #[error("application name may contain only letters, digits, space, '.', '_' and '-'")]
    Charset,
}

/// A validated, lowercased application (process) name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppName(String);

impl AppName {
    /// Trims, validates (`[A-Za-z0-9 ._-]`, 3–64 chars) and lowercases `raw`.
    pub fn parse(raw: &str) -> Result<AppName, AppNameError> {
        let t = raw.trim();
        let len = t.chars().count();
        if !(3..=64).contains(&len) {
            return Err(AppNameError::Length);
        }
        if !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        {
            return Err(AppNameError::Charset);
        }
        Ok(AppName(t.to_ascii_lowercase()))
    }

    /// The normalized name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a protobuf profile could not be converted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The mode is unspecified.
    #[error(transparent)]
    Mode(#[from] UnknownMode),
    /// One of the application names is invalid.
    #[error("invalid application name {name:?}: {err}")]
    App {
        /// The offending raw name.
        name: String,
        /// The reason.
        err: AppNameError,
    },
}

/// A named rule set: a mode plus a list of applications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Database id.
    pub id: ProfileId,
    /// Display name.
    pub name: String,
    /// What the profile does.
    pub mode: Mode,
    /// Applications the mode applies to.
    pub apps: BTreeSet<AppName>,
}

impl TryFrom<&pb::Profile> for Profile {
    type Error = ProfileError;
    fn try_from(p: &pb::Profile) -> Result<Self, Self::Error> {
        let mode = Mode::try_from(p.mode)?;
        let apps = p
            .apps
            .iter()
            .map(|a| {
                AppName::parse(a).map_err(|err| ProfileError::App {
                    name: a.clone(),
                    err,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Profile {
            id: ProfileId(p.id),
            name: p.name.clone(),
            mode,
            apps,
        })
    }
}

impl Profile {
    /// The protobuf form (apps in sorted order).
    pub fn to_proto(&self) -> pb::Profile {
        pb::Profile {
            id: self.id.0,
            name: self.name.clone(),
            mode: pb::Mode::from(self.mode) as i32,
            apps: self.apps.iter().map(|a| a.0.clone()).collect(),
        }
    }
}

/// A computer's exception wins, then its room's active profile, then the school default.
pub fn effective_profile_id(
    exception: Option<ProfileId>,
    room_active: Option<ProfileId>,
    school_default: ProfileId,
) -> ProfileId {
    exception.or(room_active).unwrap_or(school_default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_trimmed_and_lowercased() {
        assert_eq!(
            AppName::parse("  Chrome.EXE ").unwrap().as_str(),
            "chrome.exe"
        );
    }

    #[test]
    fn app_name_rejects_paths_and_wildcards() {
        assert_eq!(
            AppName::parse(r"C:\x\chrome.exe"),
            Err(AppNameError::Charset)
        );
        assert_eq!(AppName::parse("chr*me"), Err(AppNameError::Charset));
        assert_eq!(AppName::parse("/usr/bin/vlc"), Err(AppNameError::Charset));
    }

    #[test]
    fn app_name_length_is_3_to_64() {
        assert_eq!(AppName::parse("qq"), Err(AppNameError::Length));
        assert!(AppName::parse("vlc").is_ok());
        assert!(AppName::parse(&"a".repeat(64)).is_ok());
        assert_eq!(AppName::parse(&"a".repeat(65)), Err(AppNameError::Length));
    }

    #[test]
    fn exception_wins_then_room_then_default() {
        let (e, r, d) = (ProfileId(9), ProfileId(5), ProfileId(2));
        assert_eq!(effective_profile_id(Some(e), Some(r), d), e);
        assert_eq!(effective_profile_id(None, Some(r), d), r);
        assert_eq!(effective_profile_id(None, None, d), d);
    }

    #[test]
    fn unspecified_mode_is_rejected() {
        assert!(Mode::try_from(guardian_proto::v1::Mode::Unspecified).is_err());
    }
}
