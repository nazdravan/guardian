//! Admin: enrollment codes, computers, consoles.

use guardian_proto::v1 as pb;
use guardian_proto::v1::{
    ComputerIdRequest, ComputerList, ConsoleList, CreateEnrollCodeRequest, Empty, EnrollCode,
    EnrollCodeKind, IdRequest, UpdateComputerRequest,
};
use tonic::{Request, Response, Status};

use super::admin::AdminSvc;
use crate::db::{self, codes::CodeKind, computers::ComputerUpdate};
use crate::state::Scope;

/// TTL of single-use agent codes and console codes (15 min).
pub const SINGLE_USE_TTL: u32 = 900;
/// Default TTL of room codes (24 h).
pub const ROOM_CODE_TTL: u32 = 86_400;

impl AdminSvc {
    pub(crate) async fn computer_proto(&self, id: u64) -> Result<pb::Computer, Status> {
        let row = db::computers::get(&self.state.db, id)
            .await?
            .ok_or_else(|| Status::not_found("computer not found"))?;
        let default = db::settings::get(&self.state.db).await?.default_profile_id;
        Ok(row.to_proto(self.state.is_online(id), default))
    }

    pub(crate) async fn create_enroll_code_impl(
        &self,
        req: Request<CreateEnrollCodeRequest>,
    ) -> Result<Response<EnrollCode>, Status> {
        let c = self.admin(&req).await?;
        let r = req.into_inner();
        let (kind, room, uses, ttl) = match EnrollCodeKind::try_from(r.kind) {
            Ok(EnrollCodeKind::EnrollAgent) if r.room_id == 0 => {
                (CodeKind::Agent, None, 1, SINGLE_USE_TTL)
            }
            Ok(EnrollCodeKind::EnrollAgent) => {
                if db::rooms::get(&self.state.db, r.room_id).await?.is_none() {
                    return Err(Status::invalid_argument("room does not exist"));
                }
                if !(1..=500).contains(&r.max_uses) {
                    return Err(Status::invalid_argument("room codes need max_uses 1-500"));
                }
                let ttl = if r.ttl_seconds == 0 {
                    ROOM_CODE_TTL
                } else {
                    r.ttl_seconds
                };
                if !(60..=604_800).contains(&ttl) {
                    return Err(Status::invalid_argument("ttl_seconds must be 60-604800"));
                }
                (CodeKind::Agent, Some(r.room_id), r.max_uses, ttl)
            }
            Ok(EnrollCodeKind::EnrollConsole) => (CodeKind::Console, None, 1, SINGLE_USE_TTL),
            _ => return Err(Status::invalid_argument("kind must be agent or console")),
        };
        let code = db::codes::create(&self.state.db, kind, room, uses, ttl, c.user_id).await?;
        self.state.audit(
            Some(c.user_id),
            "CreateEnrollCode",
            room.map_or_else(|| "school".into(), |r| format!("room:{r}")),
            format!("{kind:?} max_uses={uses} ttl={ttl}"),
        );
        Ok(Response::new(code))
    }

    pub(crate) async fn list_computers_impl(
        &self,
        req: Request<Empty>,
    ) -> Result<Response<ComputerList>, Status> {
        self.admin(&req).await?;
        let default = db::settings::get(&self.state.db).await?.default_profile_id;
        let rows = db::computers::list(&self.state.db).await?;
        Ok(Response::new(ComputerList {
            computers: rows
                .iter()
                .map(|r| r.to_proto(self.state.is_online(r.id), default))
                .collect(),
        }))
    }

    pub(crate) async fn update_computer_impl(
        &self,
        req: Request<UpdateComputerRequest>,
    ) -> Result<Response<pb::Computer>, Status> {
        let c = self.admin(&req).await?;
        let r = req.into_inner();
        let mut u = ComputerUpdate::default();
        if let Some(name) = &r.name {
            let t = name.trim();
            if t.chars().count() > 64 {
                return Err(Status::invalid_argument(
                    "computer name must be at most 64 characters",
                ));
            }
            u.name = Some(t.to_string());
        }
        if let Some(room) = r.room_id {
            if room != 0 && db::rooms::get(&self.state.db, room).await?.is_none() {
                return Err(Status::invalid_argument("room does not exist"));
            }
            u.room_id = Some((room != 0).then_some(room));
        }
        if let Some(p) = r.exception_profile_id {
            if p != 0 && !db::profiles::exists(&self.state.db, p).await? {
                return Err(Status::invalid_argument("profile does not exist"));
            }
            u.exception_profile_id = Some((p != 0).then_some(p));
        }
        if !db::computers::update(&self.state.db, r.id, &u).await? {
            return Err(Status::not_found("computer not found"));
        }
        self.state.audit(
            Some(c.user_id),
            "UpdateComputer",
            format!("computer:{}", r.id),
            format!("{u:?}"),
        );
        if u.room_id.is_some() || u.exception_profile_id.is_some() {
            self.state.policy_changed(Scope::Computer(r.id)).await;
        }
        if u.room_id.is_some() {
            self.state.trust_changed().await;
        }
        self.state.computer_changed(r.id);
        Ok(Response::new(self.computer_proto(r.id).await?))
    }

    pub(crate) async fn revoke_computer_impl(
        &self,
        req: Request<ComputerIdRequest>,
    ) -> Result<Response<Empty>, Status> {
        let c = self.admin(&req).await?;
        let id = req.get_ref().id;
        if !db::computers::delete(&self.state.db, id).await? {
            return Err(Status::not_found("computer not found"));
        }
        self.state.audit(
            Some(c.user_id),
            "RevokeComputer",
            format!("computer:{id}"),
            String::new(),
        );
        self.state.computer_revoked(id).await;
        Ok(Response::new(Empty {}))
    }

    pub(crate) async fn list_consoles_impl(
        &self,
        req: Request<Empty>,
    ) -> Result<Response<ConsoleList>, Status> {
        self.admin(&req).await?;
        Ok(Response::new(ConsoleList {
            consoles: db::consoles::list(&self.state.db).await?,
        }))
    }

    pub(crate) async fn revoke_console_impl(
        &self,
        req: Request<IdRequest>,
    ) -> Result<Response<Empty>, Status> {
        let c = self.admin(&req).await?;
        let id = req.get_ref().id;
        if !db::consoles::revoke(&self.state.db, id).await? {
            return Err(Status::not_found("console not found"));
        }
        self.state.audit(
            Some(c.user_id),
            "RevokeConsole",
            format!("console:{id}"),
            String::new(),
        );
        self.state.trust_changed().await;
        Ok(Response::new(Empty {}))
    }
}
