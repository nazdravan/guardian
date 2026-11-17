//! `guardian.v1.Admin` — admin-only management.

use std::sync::Arc;

use guardian_core::policy::{Profile, ProfileId};
use guardian_proto::v1 as pb;
use guardian_proto::v1::admin_server::Admin;
use guardian_proto::v1::{
    ComputerIdRequest, ComputerList, ConsoleList, CreateEnrollCodeRequest, CreateRoomRequest,
    CreateUserRequest, Empty, EnrollCode, IdRequest, ProfileList, RenameRoomRequest, Room,
    RoomList, SetUserRoomsRequest, Settings, UpdateComputerRequest, UpdateUserRequest, User,
    UserList,
};
use tonic::{Request, Response, Status};

use crate::auth::{Caller, caller_from_request};
use crate::db::{self, profiles::DeleteProfile};
use crate::state::{AppState, Scope};

/// Trims and checks a room or profile name (1–64 characters).
pub fn valid_name(raw: &str, what: &str) -> Result<String, Status> {
    let t = raw.trim();
    if (1..=64).contains(&t.chars().count()) {
        Ok(t.to_string())
    } else {
        Err(Status::invalid_argument(format!(
            "{what} name must be 1-64 characters"
        )))
    }
}

fn unique(e: db::DbError, what: &str) -> Status {
    if e.is_unique_violation() {
        Status::already_exists(format!("{what} already exists"))
    } else {
        e.into()
    }
}

/// The Admin service.
#[derive(Debug, Clone)]
pub struct AdminSvc {
    pub(crate) state: Arc<AppState>,
}

impl AdminSvc {
    /// Wraps the shared state.
    pub fn new(state: Arc<AppState>) -> AdminSvc {
        AdminSvc { state }
    }

    pub(crate) async fn admin<T>(&self, req: &Request<T>) -> Result<Caller, Status> {
        let c = caller_from_request(&self.state, req).await?;
        c.require_admin()?;
        Ok(c)
    }

    fn profile_from(&self, p: &pb::Profile) -> Result<Profile, Status> {
        let name = valid_name(&p.name, "profile")?;
        let mut prof = Profile::try_from(p).map_err(|e| Status::invalid_argument(e.to_string()))?;
        prof.name = name;
        Ok(prof)
    }
}

#[tonic::async_trait]
impl Admin for AdminSvc {
    async fn list_users(&self, req: Request<Empty>) -> Result<Response<UserList>, Status> {
        self.list_users_impl(req).await
    }

    async fn create_user(&self, req: Request<CreateUserRequest>) -> Result<Response<User>, Status> {
        self.create_user_impl(req).await
    }

    async fn update_user(&self, req: Request<UpdateUserRequest>) -> Result<Response<User>, Status> {
        self.update_user_impl(req).await
    }

    async fn set_user_rooms(
        &self,
        req: Request<SetUserRoomsRequest>,
    ) -> Result<Response<User>, Status> {
        self.set_user_rooms_impl(req).await
    }

    async fn list_rooms(&self, req: Request<Empty>) -> Result<Response<RoomList>, Status> {
        self.admin(&req).await?;
        Ok(Response::new(RoomList {
            rooms: db::rooms::list(&self.state.db).await?,
        }))
    }

    async fn create_room(&self, req: Request<CreateRoomRequest>) -> Result<Response<Room>, Status> {
        let c = self.admin(&req).await?;
        let name = valid_name(&req.get_ref().name, "room")?;
        let room = db::rooms::create(&self.state.db, &name)
            .await
            .map_err(|e| unique(e, "a room with this name"))?;
        self.state.audit(
            Some(c.user_id),
            "CreateRoom",
            format!("room:{}", room.id),
            name,
        );
        Ok(Response::new(room))
    }

    async fn rename_room(&self, req: Request<RenameRoomRequest>) -> Result<Response<Room>, Status> {
        let c = self.admin(&req).await?;
        let r = req.into_inner();
        let name = valid_name(&r.name, "room")?;
        let room = db::rooms::rename(&self.state.db, r.id, &name)
            .await
            .map_err(|e| unique(e, "a room with this name"))?
            .ok_or_else(|| Status::not_found("room not found"))?;
        self.state.audit(
            Some(c.user_id),
            "RenameRoom",
            format!("room:{}", r.id),
            name,
        );
        self.state.room_changed(r.id);
        Ok(Response::new(room))
    }

    async fn delete_room(&self, req: Request<IdRequest>) -> Result<Response<Empty>, Status> {
        let c = self.admin(&req).await?;
        let id = req.get_ref().id;
        if !db::rooms::delete(&self.state.db, id).await? {
            return Err(Status::not_found("room not found"));
        }
        self.state.audit(
            Some(c.user_id),
            "DeleteRoom",
            format!("room:{id}"),
            String::new(),
        );
        self.state.room_removed(id);
        self.state.policy_changed(Scope::All).await;
        self.state.trust_changed().await;
        Ok(Response::new(Empty {}))
    }

    async fn list_profiles(&self, req: Request<Empty>) -> Result<Response<ProfileList>, Status> {
        self.admin(&req).await?;
        let profiles = db::profiles::list(&self.state.db).await?;
        Ok(Response::new(ProfileList {
            profiles: profiles.iter().map(Profile::to_proto).collect(),
        }))
    }

    async fn create_profile(
        &self,
        req: Request<pb::Profile>,
    ) -> Result<Response<pb::Profile>, Status> {
        let c = self.admin(&req).await?;
        let p = self.profile_from(req.get_ref())?;
        let created = db::profiles::create(&self.state.db, &p)
            .await
            .map_err(|e| unique(e, "a profile with this name"))?;
        self.state.audit(
            Some(c.user_id),
            "CreateProfile",
            format!("profile:{}", created.id.0),
            created.name.clone(),
        );
        Ok(Response::new(created.to_proto()))
    }

    async fn update_profile(
        &self,
        req: Request<pb::Profile>,
    ) -> Result<Response<pb::Profile>, Status> {
        let c = self.admin(&req).await?;
        let p = self.profile_from(req.get_ref())?;
        if !db::profiles::update(&self.state.db, &p)
            .await
            .map_err(|e| unique(e, "a profile with this name"))?
        {
            return Err(Status::not_found("profile not found"));
        }
        self.state.audit(
            Some(c.user_id),
            "UpdateProfile",
            format!("profile:{}", p.id.0),
            p.name.clone(),
        );
        self.state.policy_changed(Scope::All).await;
        Ok(Response::new(p.to_proto()))
    }

    async fn delete_profile(&self, req: Request<IdRequest>) -> Result<Response<Empty>, Status> {
        let c = self.admin(&req).await?;
        let id = req.get_ref().id;
        match db::profiles::delete(&self.state.db, ProfileId(id)).await? {
            DeleteProfile::Deleted => {}
            DeleteProfile::NotFound => return Err(Status::not_found("profile not found")),
            DeleteProfile::InUse(why) => {
                return Err(Status::failed_precondition(format!("profile {why}")));
            }
        }
        self.state.audit(
            Some(c.user_id),
            "DeleteProfile",
            format!("profile:{id}"),
            String::new(),
        );
        self.state.policy_changed(Scope::All).await;
        Ok(Response::new(Empty {}))
    }

    async fn list_computers(&self, req: Request<Empty>) -> Result<Response<ComputerList>, Status> {
        self.list_computers_impl(req).await
    }

    async fn update_computer(
        &self,
        req: Request<UpdateComputerRequest>,
    ) -> Result<Response<pb::Computer>, Status> {
        self.update_computer_impl(req).await
    }

    async fn revoke_computer(
        &self,
        req: Request<ComputerIdRequest>,
    ) -> Result<Response<Empty>, Status> {
        self.revoke_computer_impl(req).await
    }

    async fn create_enroll_code(
        &self,
        req: Request<CreateEnrollCodeRequest>,
    ) -> Result<Response<EnrollCode>, Status> {
        self.create_enroll_code_impl(req).await
    }

    async fn list_consoles(&self, req: Request<Empty>) -> Result<Response<ConsoleList>, Status> {
        self.list_consoles_impl(req).await
    }

    async fn revoke_console(&self, req: Request<IdRequest>) -> Result<Response<Empty>, Status> {
        self.revoke_console_impl(req).await
    }

    async fn get_settings(&self, req: Request<Empty>) -> Result<Response<Settings>, Status> {
        self.admin(&req).await?;
        Ok(Response::new(db::settings::get(&self.state.db).await?))
    }

    async fn update_settings(&self, req: Request<Settings>) -> Result<Response<Settings>, Status> {
        let c = self.admin(&req).await?;
        let s = req.into_inner();
        if !db::profiles::exists(&self.state.db, s.default_profile_id).await? {
            return Err(Status::invalid_argument("default profile does not exist"));
        }
        if !(1..=3650).contains(&s.retention_days) {
            return Err(Status::invalid_argument("retention_days must be 1-3650"));
        }
        db::settings::update(&self.state.db, &s).await?;
        self.state.audit(
            Some(c.user_id),
            "UpdateSettings",
            "settings".into(),
            format!(
                "default_profile_id={} retention_days={}",
                s.default_profile_id, s.retention_days
            ),
        );
        self.state.policy_changed(Scope::All).await;
        Ok(Response::new(db::settings::get(&self.state.db).await?))
    }
}
