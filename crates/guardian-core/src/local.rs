//! The rules of local fallback mode (§11): when an agent takes commands from a
//! console on the LAN, who may switch which profile, and which profile applies
//! under a local switch. Pure logic, shared by the agent and the console.

use std::time::Duration;

use guardian_proto::v1::{
    AgentState, AuthorizedUser, LocalActor, LocalOverride, LocalRefused, LocalSetProfile, Mode,
};

/// Without an authenticated server connection for this long, an agent is
/// offline and accepts local commands (§10.2).
pub const OFFLINE_AFTER: Duration = Duration::from_secs(60);

/// TCP and UDP port of the local channel (§11.3).
pub const DEFAULT_LOCAL_PORT: u16 = 11500;

/// Why an agent refuses a local request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Connected to the server (or offline for less than [`OFFLINE_AFTER`]).
    Online,
    /// The server removed this computer.
    Revoked,
    /// The user is not an admin or a teacher of this computer's room.
    Unauthorized,
    /// The computer is in no room or in another one.
    NotInRoom,
    /// The computer has its own exception profile.
    HasException,
    /// The profile is not in the synced state (or is invalid).
    UnknownProfile,
    /// `free` profiles cannot be set locally (fail-secure).
    FreeProfile,
    /// The request is malformed.
    BadRequest,
}

impl Refusal {
    /// The wire code.
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Online => "online",
            Refusal::Revoked => "revoked",
            Refusal::Unauthorized => "unauthorized",
            Refusal::NotInRoom => "not_in_room",
            Refusal::HasException => "has_exception",
            Refusal::UnknownProfile => "unknown_profile",
            Refusal::FreeProfile => "free_profile",
            Refusal::BadRequest => "bad_request",
        }
    }

    /// The text shown to the teacher.
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Online => "This computer is connected to the server; use the server.",
            Refusal::Revoked => "This computer was removed from the server.",
            Refusal::Unauthorized => "This user may not control this computer.",
            Refusal::NotInRoom => "This computer is not in that room.",
            Refusal::HasException => "This computer has its own profile.",
            Refusal::UnknownProfile => "This profile is not known on this computer.",
            Refusal::FreeProfile => "A free profile cannot be set in fallback mode.",
            Refusal::BadRequest => "Malformed request.",
        }
    }

    /// The protobuf form.
    pub fn to_proto(self) -> LocalRefused {
        LocalRefused {
            code: self.code().into(),
            message: self.message().into(),
        }
    }
}

/// The actor is one of the users the server authorized for this computer:
/// both the id and the login must match.
pub fn is_authorized(users: &[AuthorizedUser], actor: &LocalActor) -> bool {
    users
        .iter()
        .any(|u| u.user_id == actor.user_id && u.login == actor.login)
}

/// Room, exception and profile rules (everything but the user).
fn check_target(state: &AgentState, room_id: i64, profile_id: i64) -> Result<(), Refusal> {
    if state.room_id == 0 || state.room_id != room_id {
        return Err(Refusal::NotInRoom);
    }
    if state.exception_profile_id != 0 {
        return Err(Refusal::HasException);
    }
    let profile = state
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or(Refusal::UnknownProfile)?;
    match Mode::try_from(profile.mode) {
        Ok(Mode::Free) => Err(Refusal::FreeProfile),
        Ok(Mode::Blacklist | Mode::Whitelist) => Ok(()),
        Ok(Mode::Unspecified) | Err(_) => Err(Refusal::UnknownProfile),
    }
}

/// Whether `actor` may switch this computer's room to `req.profile_id`.
pub fn check_set_profile(
    state: &AgentState,
    users: &[AuthorizedUser],
    actor: &LocalActor,
    req: &LocalSetProfile,
) -> Result<(), Refusal> {
    if !is_authorized(users, actor) {
        return Err(Refusal::Unauthorized);
    }
    check_target(state, req.room_id, req.profile_id)
}

/// The state with a local switch applied: only the effective profile changes,
/// and only while the switch still passes the room, exception and profile rules.
pub fn apply_override(state: &AgentState, ov: Option<&LocalOverride>) -> AgentState {
    let mut out = state.clone();
    if let Some(ov) = ov
        && check_target(state, ov.room_id, ov.profile_id).is_ok()
    {
        out.effective_profile_id = ov.profile_id;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_proto::v1::Profile;

    fn profile(id: i64, name: &str, mode: Mode) -> Profile {
        Profile {
            id,
            name: name.into(),
            mode: mode as i32,
            apps: vec![],
        }
    }

    fn state() -> AgentState {
        AgentState {
            version: 4,
            effective_profile_id: 2,
            profiles: vec![
                profile(1, "Liber", Mode::Free),
                profile(2, "Lecție", Mode::Blacklist),
                profile(3, "Examen", Mode::Whitelist),
                profile(9, "Broken", Mode::Unspecified),
            ],
            room_id: 7,
            exception_profile_id: 0,
        }
    }

    fn users() -> Vec<AuthorizedUser> {
        vec![
            AuthorizedUser {
                user_id: 1,
                login: "admin".into(),
            },
            AuthorizedUser {
                user_id: 5,
                login: "prof".into(),
            },
        ]
    }

    fn actor(id: i64, login: &str) -> LocalActor {
        LocalActor {
            user_id: id,
            login: login.into(),
        }
    }

    fn req(room: i64, profile: i64) -> LocalSetProfile {
        LocalSetProfile {
            room_id: room,
            profile_id: profile,
        }
    }

    fn ov(room: i64, profile: i64) -> LocalOverride {
        LocalOverride {
            room_id: room,
            profile_id: profile,
            set_at_unix: 100,
            user_id: 5,
        }
    }

    #[test]
    fn set_profile_rules() {
        let prof = actor(5, "prof");
        let mut exc = state();
        exc.exception_profile_id = 1;
        let mut no_room = state();
        no_room.room_id = 0;
        let cases: Vec<(AgentState, LocalActor, LocalSetProfile, Result<(), Refusal>)> = vec![
            (state(), prof.clone(), req(7, 3), Ok(())),
            (
                state(),
                actor(6, "other"),
                req(7, 3),
                Err(Refusal::Unauthorized),
            ),
            (state(), prof.clone(), req(8, 3), Err(Refusal::NotInRoom)),
            (no_room, prof.clone(), req(0, 3), Err(Refusal::NotInRoom)),
            (exc, prof.clone(), req(7, 3), Err(Refusal::HasException)),
            (
                state(),
                prof.clone(),
                req(7, 42),
                Err(Refusal::UnknownProfile),
            ),
            (
                state(),
                prof.clone(),
                req(7, 9),
                Err(Refusal::UnknownProfile),
            ),
            (state(), prof.clone(), req(7, 1), Err(Refusal::FreeProfile)),
        ];
        for (i, (st, a, r, want)) in cases.into_iter().enumerate() {
            assert_eq!(check_set_profile(&st, &users(), &a, &r), want, "case {i}");
        }
    }

    #[test]
    fn login_must_match_user_id() {
        assert!(is_authorized(&users(), &actor(5, "prof")));
        assert!(!is_authorized(&users(), &actor(5, "admin")));
        assert!(!is_authorized(&users(), &actor(1, "prof")));
        assert_eq!(
            check_set_profile(&state(), &users(), &actor(5, "admin"), &req(7, 3)),
            Err(Refusal::Unauthorized)
        );
    }

    #[test]
    fn free_is_refused_even_for_admin() {
        assert_eq!(
            check_set_profile(&state(), &users(), &actor(1, "admin"), &req(7, 1)),
            Err(Refusal::FreeProfile)
        );
    }

    #[test]
    fn refusals_have_codes_and_texts() {
        assert_eq!(Refusal::Online.code(), "online");
        assert_eq!(
            Refusal::Online.message(),
            "This computer is connected to the server; use the server."
        );
        assert_eq!(Refusal::FreeProfile.code(), "free_profile");
        assert_eq!(
            Refusal::FreeProfile.to_proto(),
            LocalRefused {
                code: "free_profile".into(),
                message: "A free profile cannot be set in fallback mode.".into(),
            }
        );
        let all = [
            Refusal::Online,
            Refusal::Revoked,
            Refusal::Unauthorized,
            Refusal::NotInRoom,
            Refusal::HasException,
            Refusal::UnknownProfile,
            Refusal::FreeProfile,
            Refusal::BadRequest,
        ];
        let codes: std::collections::BTreeSet<_> = all.iter().map(|r| r.code()).collect();
        assert_eq!(codes.len(), all.len());
    }

    #[test]
    fn override_applies_only_in_its_room() {
        let st = state();
        assert_eq!(apply_override(&st, Some(&ov(7, 3))).effective_profile_id, 3);
        assert_eq!(apply_override(&st, Some(&ov(8, 3))), st);
        assert_eq!(apply_override(&st, None), st);
        // Only the effective profile changes.
        let applied = apply_override(&st, Some(&ov(7, 3)));
        assert_eq!(applied.version, st.version);
        assert_eq!(applied.profiles, st.profiles);
    }

    #[test]
    fn exception_beats_override() {
        let mut st = state();
        st.exception_profile_id = 1;
        st.effective_profile_id = 1;
        assert_eq!(apply_override(&st, Some(&ov(7, 3))), st);
    }

    #[test]
    fn stale_override_is_inert() {
        let mut st = state();
        st.profiles.retain(|p| p.id != 3);
        assert_eq!(apply_override(&st, Some(&ov(7, 3))), st);
        let mut moved = state();
        moved.room_id = 8;
        assert_eq!(apply_override(&moved, Some(&ov(7, 3))), moved);
        // A profile that became free since is not applied either.
        let mut freed = state();
        freed.profiles[2].mode = guardian_proto::v1::Mode::Free as i32;
        assert_eq!(apply_override(&freed, Some(&ov(7, 3))), freed);
    }
}
