//! State shared by every request handler.

use std::sync::Mutex;

use crate::auth::RateLimiter;
use crate::batch::Batcher;
use crate::config::ServerConfig;
use crate::db::Db;
use crate::hub::{Hub, HubEvent};

/// Shared server state (behind an `Arc`).
#[derive(Debug)]
pub struct AppState {
    /// The database.
    pub db: Db,
    /// Batched writer for results and audit entries.
    pub batcher: Batcher,
    /// Per-IP limiter for authentication attempts.
    pub limiter: RateLimiter,
    /// The one-time bootstrap code, while no user exists.
    pub bootstrap: Mutex<Option<String>>,
    /// The configuration the server was started with.
    pub config: ServerConfig,
    /// Connected agents and the event bus.
    pub hub: Hub,
}

impl AppState {
    /// The current bootstrap code, if any.
    pub fn bootstrap_code(&self) -> Option<String> {
        self.bootstrap.lock().ok().and_then(|g| g.clone())
    }
}

/// Which agents a policy change affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every connected agent.
    All,
    /// Agents in one room.
    Room(i64),
    /// One agent.
    Computer(u64),
}

impl AppState {
    /// Computes and pushes the new state to every connected agent in `scope`.
    pub async fn policy_changed(&self, scope: Scope) {
        let online = self.hub.online_ids();
        let targets: Vec<u64> = match scope {
            Scope::All => online,
            Scope::Computer(c) => online.into_iter().filter(|o| *o == c).collect(),
            Scope::Room(r) => match crate::db::computers::in_room(&self.db, r).await {
                Ok(ids) => ids.into_iter().filter(|i| online.contains(i)).collect(),
                Err(e) => {
                    tracing::error!("policy push for room {r}: {e}");
                    return;
                }
            },
        };
        for id in targets {
            match crate::hub::build_agent_state(&self.db, id).await {
                Ok(st) => {
                    self.hub.send(
                        id,
                        guardian_proto::v1::ServerFrame {
                            msg: Some(guardian_proto::v1::server_frame::Msg::State(st)),
                        },
                    );
                }
                Err(e) => tracing::error!("policy push to computer {id}: {e}"),
            }
            self.hub.emit(HubEvent::ComputerChanged(id));
        }
    }

    /// Pushes trusted consoles and authorized users to every connected agent.
    pub async fn trust_changed(&self) {
        for id in self.hub.online_ids() {
            match crate::hub::build_trust(&self.db, id).await {
                Ok((consoles, users)) => {
                    self.hub.send(
                        id,
                        guardian_proto::v1::ServerFrame {
                            msg: Some(guardian_proto::v1::server_frame::Msg::TrustUpdate(
                                guardian_proto::v1::Welcome {
                                    state: None,
                                    pending: Vec::new(),
                                    consoles,
                                    users,
                                },
                            )),
                        },
                    );
                }
                Err(e) => tracing::error!("trust push to computer {id}: {e}"),
            }
        }
    }

    /// A user's rooms, role or active flag changed.
    pub async fn user_rooms_changed(&self, user_id: i64) {
        self.hub.emit(HubEvent::UserRoomsChanged(user_id));
    }

    /// Whether an agent is connected (the hub, from Task 11).
    pub fn is_online(&self, id: u64) -> bool {
        self.hub.is_online(id)
    }

    /// A computer's row changed.
    pub fn computer_changed(&self, id: u64) {
        self.hub.emit(HubEvent::ComputerChanged(id));
    }

    /// A room was renamed or switched profile.
    pub fn room_changed(&self, room_id: i64) {
        self.hub.emit(HubEvent::RoomChanged(room_id));
    }

    /// A room was deleted.
    pub fn room_removed(&self, room_id: i64) {
        self.hub.emit(HubEvent::RoomRemoved(room_id));
    }

    /// A computer was revoked: tell it, close it, and drop it from `Watch`.
    pub async fn computer_revoked(&self, id: u64) {
        self.hub.send(
            id,
            guardian_proto::v1::ServerFrame {
                msg: Some(guardian_proto::v1::server_frame::Msg::Error(
                    guardian_proto::v1::ServerError {
                        code: "revoked".into(),
                        message: "this computer was removed from the school".into(),
                    },
                )),
            },
        );
        self.hub.disconnect(id);
        self.hub.emit(HubEvent::ComputerRemoved(id));
    }

    /// Commands waiting for `id` that its capabilities allow; `pending` ones become
    /// `delivered`, the rest `unsupported`. Already-`delivered`, unfinished ones are
    /// included again (the previous connection may have died before reading them).
    pub async fn take_pending(
        &self,
        id: u64,
        caps: &std::collections::BTreeSet<String>,
    ) -> Vec<guardian_proto::v1::CommandEnvelope> {
        use guardian_proto::v1::ResultStatus;
        let pending =
            match crate::db::commands::pending_for(&self.db, id, crate::db::now_unix()).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("pending commands for {id}: {e}");
                    return Vec::new();
                }
            };
        let mut out = Vec::new();
        for (env, current) in pending {
            let supported = env
                .command
                .as_ref()
                .and_then(|c| c.kind.as_ref())
                .is_some_and(|k| caps.contains(guardian_core::feature::feature_of(k).name));
            let to = if supported {
                ResultStatus::ResultDelivered
            } else {
                ResultStatus::ResultUnsupported
            };
            if current == ResultStatus::ResultDelivered {
                // Already sent once, maybe to a connection that died: send again.
                if supported {
                    out.push(env);
                }
                continue;
            }
            if let Ok(true) = crate::db::commands::transition(
                &self.db,
                &env.id,
                id,
                ResultStatus::ResultPending,
                to,
            )
            .await
            {
                self.command_event(&env.id, id, to, String::new());
                if supported {
                    out.push(env);
                }
            }
        }
        out
    }

    /// Emits a `CommandResult` hub event.
    pub fn command_event(
        &self,
        command_id: &str,
        computer_id: u64,
        status: guardian_proto::v1::ResultStatus,
        detail: String,
    ) {
        if let Ok(command_id) = uuid::Uuid::parse_str(command_id) {
            self.hub.emit(HubEvent::CommandResult {
                command_id,
                computer_id,
                status,
                detail,
            });
        }
    }

    /// A result reported by an agent: accepted only for a command targeting it and
    /// only with an agent-side status.
    pub async fn command_result_from_agent(&self, id: u64, r: guardian_proto::v1::CommandResult) {
        use guardian_proto::v1::ResultStatus;
        let Ok(command_id) = uuid::Uuid::parse_str(&r.command_id) else {
            tracing::warn!("computer {id}: result for malformed command id");
            return;
        };
        let status = ResultStatus::try_from(r.status).unwrap_or(ResultStatus::Unspecified);
        if !matches!(
            status,
            ResultStatus::ResultOk
                | ResultStatus::ResultFailed
                | ResultStatus::ResultNoUser
                | ResultStatus::ResultUnsupported
        ) {
            tracing::warn!("computer {id}: ignoring result with status {status:?}");
            return;
        }
        match crate::db::commands::is_target(&self.db, &r.command_id, id).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!("computer {id}: ignoring result for a command not sent to it");
                return;
            }
            Err(e) => {
                tracing::error!("{e}");
                return;
            }
        }
        let detail: String = r.detail.chars().take(1000).collect();
        self.batcher
            .command_result(command_id, id, status, detail.clone());
        self.command_event(&r.command_id, id, status, detail);
    }

    /// Marks expired targets and tells `Watch`.
    pub async fn expire_commands(&self) {
        match crate::db::commands::expire_due(&self.db, crate::db::now_unix()).await {
            Ok(rows) => {
                for (c, id) in rows {
                    self.hub.emit(HubEvent::CommandResult {
                        command_id: c,
                        computer_id: id,
                        status: guardian_proto::v1::ResultStatus::ResultExpired,
                        detail: String::new(),
                    });
                }
            }
            Err(e) => tracing::error!("expiring commands: {e}"),
        }
    }

    /// Stores the journal an agent kept in local fallback mode (at most
    /// [`MAX_LOCAL_AUDIT`] entries; texts cut to 200 characters). Returns the
    /// highest `seq` stored.
    pub fn audit_local(
        &self,
        computer_id: u64,
        entries: &[guardian_proto::v1::LocalAuditEntry],
    ) -> u64 {
        if entries.len() > MAX_LOCAL_AUDIT {
            tracing::warn!(
                "computer {computer_id}: local journal of {} entries, keeping {MAX_LOCAL_AUDIT}",
                entries.len()
            );
        }
        let mut up_to = 0;
        for e in entries.iter().take(MAX_LOCAL_AUDIT) {
            let detail = format!("{} (console {}, {})", e.detail, e.console_id, e.login);
            self.batcher.audit_at(
                crate::batch::AuditEntry {
                    actor_user_id: Some(e.user_id),
                    action: cut(&e.action),
                    target: format!("computer:{computer_id}"),
                    detail: cut(&detail),
                    source: crate::batch::AuditSource::Local,
                },
                e.at_unix,
            );
            up_to = up_to.max(e.seq);
        }
        up_to
    }

    /// Queues an audit entry for an action through the server.
    pub fn audit(&self, actor: Option<i64>, action: &str, target: String, detail: String) {
        self.batcher.audit(crate::batch::AuditEntry {
            actor_user_id: actor,
            action: action.into(),
            target,
            detail,
            source: crate::batch::AuditSource::Server,
        });
    }
}

/// Most entries taken from one `LocalAudit` frame.
pub const MAX_LOCAL_AUDIT: usize = 1000;

fn cut(s: &str) -> String {
    s.chars().take(200).collect()
}
