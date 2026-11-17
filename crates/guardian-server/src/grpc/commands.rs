//! `Control.SendCommand` / `Control.GetCommand`.

use std::collections::BTreeSet;

use guardian_core::feature::feature_of;
use guardian_proto::v1::{
    Command, CommandEnvelope, CommandStatus, GetCommandRequest, ResultStatus, Role,
    SendCommandRequest, SendCommandResponse, ServerFrame, command, send_command_request,
    server_frame,
};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use super::control::ControlSvc;
use crate::auth::{Caller, caller_from_request};
use crate::db::{self, now_unix};

/// Default command TTL (10 min).
pub const DEFAULT_TTL: u32 = 600;
/// Longest command TTL (24 h).
pub const MAX_TTL: u32 = 86_400;

fn bad(msg: &str) -> Status {
    Status::invalid_argument(msg.to_string())
}

fn len(s: &str) -> usize {
    s.chars().count()
}

/// Checks a command's arguments.
pub fn validate(kind: &command::Kind) -> Result<(), Status> {
    match kind {
        command::Kind::Message(m) => {
            if len(&m.title) > 100 {
                return Err(bad("message title must be at most 100 characters"));
            }
            if !(1..=2000).contains(&len(&m.text)) {
                return Err(bad("message text must be 1-2000 characters"));
            }
        }
        command::Kind::ProgramStart(p) => {
            if !(1..=1024).contains(&len(&p.path)) {
                return Err(bad("program path must be 1-1024 characters"));
            }
            if p.args.len() > 32 || p.args.iter().any(|a| len(a) > 1024) {
                return Err(bad("at most 32 program arguments of up to 1024 characters"));
            }
        }
        command::Kind::SiteOpen(s) => {
            let lower = s.url.to_ascii_lowercase();
            let scheme_ok = lower.starts_with("http://") || lower.starts_with("https://");
            if !scheme_ok || len(&s.url) > 2048 || s.url.chars().any(char::is_control) {
                return Err(bad(
                    "url must be http:// or https:// and at most 2048 characters",
                ));
            }
        }
        command::Kind::Shutdown(c) => {
            if c.countdown_seconds > 3600 {
                return Err(bad("countdown must be 0-3600 seconds"));
            }
        }
        command::Kind::Restart(c) => {
            if c.countdown_seconds > 3600 {
                return Err(bad("countdown must be 0-3600 seconds"));
            }
        }
    }
    Ok(())
}

impl ControlSvc {
    async fn targets(
        &self,
        caller: &Caller,
        target: Option<send_command_request::Target>,
    ) -> Result<(Vec<u64>, String), Status> {
        match target {
            Some(send_command_request::Target::RoomId(r)) => {
                if !caller.can_control_room(r) {
                    return Err(Status::permission_denied("not your room"));
                }
                if db::rooms::get(&self.state.db, r).await?.is_none() {
                    return Err(Status::not_found("room not found"));
                }
                let ids = db::computers::in_room(&self.state.db, r).await?;
                if ids.is_empty() {
                    return Err(bad("the room has no computers"));
                }
                Ok((ids, format!("room:{r}")))
            }
            Some(send_command_request::Target::Computers(list)) => {
                let ids: BTreeSet<u64> = list.ids.into_iter().collect();
                if ids.is_empty() {
                    return Err(bad("no computers to target"));
                }
                for id in &ids {
                    let row = db::computers::get(&self.state.db, *id).await?;
                    let allowed = match (&row, caller.role) {
                        (Some(_), Role::Admin) => true,
                        (Some(r), _) => r.room_id.is_some_and(|room| caller.can_control_room(room)),
                        (None, Role::Admin) => {
                            return Err(bad(&format!("computer {id} does not exist")));
                        }
                        (None, _) => false,
                    };
                    if !allowed {
                        return Err(Status::permission_denied(format!(
                            "computer {id} is not in your rooms"
                        )));
                    }
                }
                let ids: Vec<u64> = ids.into_iter().collect();
                let label = format!("computers:{ids:?}");
                Ok((ids, label))
            }
            None => Err(bad("target is required")),
        }
    }

    pub(crate) async fn send_command_impl(
        &self,
        req: Request<SendCommandRequest>,
    ) -> Result<Response<SendCommandResponse>, Status> {
        let caller = caller_from_request(&self.state, &req).await?;
        let r = req.into_inner();
        let kind = r
            .command
            .as_ref()
            .and_then(|c| c.kind.clone())
            .ok_or_else(|| bad("command is required"))?;
        validate(&kind)?;
        let ttl = match r.ttl_seconds {
            0 => DEFAULT_TTL,
            t if t > MAX_TTL => return Err(bad("ttl_seconds must be at most 86400")),
            t => t,
        };
        let (targets, label) = self.targets(&caller, r.target).await?;
        let feature = feature_of(&kind);
        let env = CommandEnvelope {
            id: Uuid::new_v4().to_string(),
            expires_at_unix: now_unix() + i64::from(ttl),
            command: Some(Command { kind: Some(kind) }),
        };
        let hub = &self.state.hub;
        let plan: Vec<(u64, ResultStatus)> = targets
            .iter()
            .map(|id| {
                let st = match hub.capabilities(*id) {
                    Some(caps) if caps.contains(feature.name) => ResultStatus::ResultDelivered,
                    Some(_) => ResultStatus::ResultUnsupported,
                    None => ResultStatus::ResultPending,
                };
                (*id, st)
            })
            .collect();
        db::commands::create(&self.state.db, &env, caller.user_id, &plan).await?;
        for (id, st) in &plan {
            if *st == ResultStatus::ResultDelivered {
                let sent = hub.send(
                    *id,
                    ServerFrame {
                        msg: Some(server_frame::Msg::Command(env.clone())),
                    },
                );
                if !sent {
                    // Went offline in between: it will get it on reconnect.
                    db::commands::transition(
                        &self.state.db,
                        &env.id,
                        *id,
                        ResultStatus::ResultDelivered,
                        ResultStatus::ResultPending,
                    )
                    .await?;
                    self.state.command_event(
                        &env.id,
                        *id,
                        ResultStatus::ResultPending,
                        String::new(),
                    );
                    continue;
                }
            }
            self.state.command_event(&env.id, *id, *st, String::new());
        }
        // A target that was offline when planned may have connected (and finished
        // its Welcome) before the rows were written: deliver it now. The
        // compare-and-set makes this and `take_pending` mutually exclusive.
        for (id, st) in &plan {
            if *st != ResultStatus::ResultPending {
                continue;
            }
            let Some(caps) = hub.capabilities(*id) else {
                continue;
            };
            let to = if caps.contains(feature.name) {
                ResultStatus::ResultDelivered
            } else {
                ResultStatus::ResultUnsupported
            };
            if db::commands::transition(
                &self.state.db,
                &env.id,
                *id,
                ResultStatus::ResultPending,
                to,
            )
            .await?
            {
                if to == ResultStatus::ResultDelivered {
                    hub.send(
                        *id,
                        ServerFrame {
                            msg: Some(server_frame::Msg::Command(env.clone())),
                        },
                    );
                }
                self.state.command_event(&env.id, *id, to, String::new());
            }
        }
        self.state.audit(
            Some(caller.user_id),
            "SendCommand",
            label,
            feature.name.to_string(),
        );
        Ok(Response::new(SendCommandResponse { command_id: env.id }))
    }

    pub(crate) async fn get_command_impl(
        &self,
        req: Request<GetCommandRequest>,
    ) -> Result<Response<CommandStatus>, Status> {
        let caller = caller_from_request(&self.state, &req).await?;
        let id = Uuid::parse_str(&req.get_ref().command_id)
            .map_err(|_| bad("command_id must be a UUID"))?;
        let (status, by) = db::commands::status(&self.state.db, id)
            .await?
            .ok_or_else(|| Status::not_found("command not found"))?;
        if caller.role != Role::Admin && by != Some(caller.user_id) {
            return Err(Status::not_found("command not found"));
        }
        Ok(Response::new(status))
    }
}
