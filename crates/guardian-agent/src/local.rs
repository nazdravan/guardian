//! Local fallback mode in the agent (§11): answering a console's request, the
//! local profile switch and the journal of local actions. No network here; the
//! listener calls [`handle`] through `AgentHandle::local_request`.

use std::time::Duration;

use guardian_core::local::{Refusal, apply_override, check_set_profile};
use guardian_proto::v1::{
    AgentState, CommandResult, LocalActor, LocalAuditEntry, LocalOverride, LocalReply,
    LocalRequest, LocalStatus, LocalStore, ResultStatus, StoredState, local_reply, local_request,
};

/// Most entries kept in the journal; older ones are dropped.
pub const MAX_JOURNAL: usize = 1000;

/// What the agent knows when a request arrives.
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    /// The server's state and trust.
    pub stored: &'a StoredState,
    /// Authenticated to the server now.
    pub online: bool,
    /// Time without a server connection (`None` = unknown or online).
    pub offline_for: Option<Duration>,
    /// How long that must be before local requests are accepted.
    pub offline_after: Duration,
    /// The server revoked this computer.
    pub revoked: bool,
    /// This computer.
    pub computer_id: u64,
    /// Feature names.
    pub capabilities: &'a [&'a str],
}

/// The server state with the local switch applied.
pub fn effective_state(stored: &StoredState, local: &LocalStore) -> AgentState {
    apply_override(
        &stored.state.clone().unwrap_or_default(),
        local.r#override.as_ref(),
    )
}

/// The profile set locally, if it currently applies.
pub fn active_override(stored: &StoredState, local: &LocalStore) -> Option<i64> {
    let ov = local.r#override.as_ref()?;
    (effective_state(stored, local).effective_profile_id == ov.profile_id).then_some(ov.profile_id)
}

fn status(ctx: &Ctx<'_>, local: &LocalStore) -> LocalStatus {
    let st = effective_state(ctx.stored, local);
    LocalStatus {
        computer_id: ctx.computer_id,
        room_id: st.room_id,
        effective_profile_id: st.effective_profile_id,
        local_override: active_override(ctx.stored, local).is_some(),
        state_version: st.version,
        capabilities: ctx.capabilities.iter().map(|c| c.to_string()).collect(),
    }
}

fn reply(id: &str, kind: local_reply::Kind) -> LocalReply {
    LocalReply {
        request_id: id.to_string(),
        kind: Some(kind),
    }
}

fn refused(id: &str, r: Refusal) -> LocalReply {
    reply(id, local_reply::Kind::Refused(r.to_proto()))
}

/// Appends to the journal, dropping the oldest entry beyond [`MAX_JOURNAL`].
pub fn journal(local: &mut LocalStore, mut e: LocalAuditEntry) {
    local.next_seq = local.next_seq.max(1);
    e.seq = local.next_seq;
    local.next_seq += 1;
    local.journal.push(e);
    if local.journal.len() > MAX_JOURNAL {
        let drop = local.journal.len() - MAX_JOURNAL;
        tracing::warn!("local journal full; dropping the {drop} oldest entries");
        local.journal.drain(..drop);
    }
}

/// The server stored everything up to `up_to`.
pub fn ack(local: &mut LocalStore, up_to: u64) {
    local.journal.retain(|e| e.seq > up_to);
}

/// Answers one request. Returns the reply and whether `local` changed (the
/// caller then saves it and re-applies the policy).
pub fn handle(
    ctx: &Ctx<'_>,
    local: &mut LocalStore,
    console_id: i64,
    req: &LocalRequest,
    now_unix: i64,
) -> (LocalReply, bool) {
    let id = req.id.as_str();
    if ctx.revoked {
        return (refused(id, Refusal::Revoked), false);
    }
    let offline_long_enough =
        !ctx.online && ctx.offline_for.is_some_and(|d| d >= ctx.offline_after);
    if !offline_long_enough {
        return (refused(id, Refusal::Online), false);
    }
    let (Some(actor), Some(kind)) = (req.actor.as_ref(), req.kind.as_ref()) else {
        return (refused(id, Refusal::BadRequest), false);
    };
    match kind {
        local_request::Kind::Status(_) => (
            reply(id, local_reply::Kind::Status(status(ctx, local))),
            false,
        ),
        local_request::Kind::Command(cmd) => (
            reply(
                id,
                local_reply::Kind::CommandResult(CommandResult {
                    command_id: cmd.id.clone(),
                    status: ResultStatus::ResultUnsupported as i32,
                    detail: "not supported by this agent version".into(),
                }),
            ),
            false,
        ),
        local_request::Kind::SetProfile(sp) => {
            let server = ctx.stored.state.clone().unwrap_or_default();
            if let Err(r) = check_set_profile(&server, &ctx.stored.users, actor, sp) {
                return (refused(id, r), false);
            }
            local.r#override = Some(LocalOverride {
                room_id: sp.room_id,
                profile_id: sp.profile_id,
                set_at_unix: now_unix,
                user_id: actor.user_id,
            });
            let name = server
                .profiles
                .iter()
                .find(|p| p.id == sp.profile_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            journal(
                local,
                entry(
                    console_id,
                    actor,
                    now_unix,
                    sp.room_id,
                    sp.profile_id,
                    &name,
                ),
            );
            (
                reply(id, local_reply::Kind::Status(status(ctx, local))),
                true,
            )
        }
    }
}

fn entry(
    console_id: i64,
    actor: &LocalActor,
    at: i64,
    room: i64,
    profile: i64,
    name: &str,
) -> LocalAuditEntry {
    LocalAuditEntry {
        seq: 0,
        at_unix: at,
        console_id,
        user_id: actor.user_id,
        login: actor.login.clone(),
        action: "local.set_profile".into(),
        detail: format!("room {room} → profile {profile} ({name})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> LocalAuditEntry {
        LocalAuditEntry::default()
    }

    #[test]
    fn journal_is_capped_at_1000() {
        let mut l = LocalStore::default();
        for _ in 0..1005 {
            journal(&mut l, e());
        }
        assert_eq!(l.journal.len(), MAX_JOURNAL);
        assert_eq!(l.journal[0].seq, 6);
        assert_eq!(l.journal[MAX_JOURNAL - 1].seq, 1005);
        assert_eq!(l.next_seq, 1006);
    }

    #[test]
    fn ack_drops_up_to_seq() {
        let mut l = LocalStore::default();
        for _ in 0..5 {
            journal(&mut l, e());
        }
        ack(&mut l, 3);
        let seqs: Vec<u64> = l.journal.iter().map(|x| x.seq).collect();
        assert_eq!(seqs, vec![4, 5]);
        journal(&mut l, e());
        assert_eq!(l.journal.last().map(|x| x.seq), Some(6));
    }
}
