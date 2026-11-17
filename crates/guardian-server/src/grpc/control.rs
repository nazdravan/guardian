//! `guardian.v1.Control` — what teachers do in their rooms.

use std::sync::Arc;

use guardian_proto::v1::control_server::Control;
use guardian_proto::v1::{
    CommandStatus, GetCommandRequest, Room, SendCommandRequest, SendCommandResponse,
    SetRoomProfileRequest,
};
use tonic::{Request, Response, Status};

use crate::auth::caller_from_request;
use crate::db;
use crate::state::{AppState, Scope};

/// The Control service.
#[derive(Debug, Clone)]
pub struct ControlSvc {
    pub(crate) state: Arc<AppState>,
}

impl ControlSvc {
    /// Wraps the shared state.
    pub fn new(state: Arc<AppState>) -> ControlSvc {
        ControlSvc { state }
    }
}

#[tonic::async_trait]
impl Control for ControlSvc {
    async fn set_room_profile(
        &self,
        req: Request<SetRoomProfileRequest>,
    ) -> Result<Response<Room>, Status> {
        let c = caller_from_request(&self.state, &req).await?;
        let r = req.into_inner();
        if db::rooms::get(&self.state.db, r.room_id).await?.is_none() {
            // Teachers learn nothing about rooms that are not theirs.
            return Err(if c.can_control_room(r.room_id) {
                Status::not_found("room not found")
            } else {
                Status::permission_denied("not your room")
            });
        }
        if !c.can_control_room(r.room_id) {
            return Err(Status::permission_denied("not your room"));
        }
        if !db::profiles::exists(&self.state.db, r.profile_id).await? {
            return Err(Status::not_found("profile not found"));
        }
        let room = db::rooms::set_active_profile(&self.state.db, r.room_id, r.profile_id)
            .await?
            .ok_or_else(|| Status::not_found("room not found"))?;
        self.state.audit(
            Some(c.user_id),
            "SetRoomProfile",
            format!("room:{}", r.room_id),
            format!("profile:{}", r.profile_id),
        );
        self.state.policy_changed(Scope::Room(r.room_id)).await;
        self.state.room_changed(r.room_id);
        Ok(Response::new(room))
    }

    async fn send_command(
        &self,
        req: Request<SendCommandRequest>,
    ) -> Result<Response<SendCommandResponse>, Status> {
        self.send_command_impl(req).await
    }

    async fn get_command(
        &self,
        req: Request<GetCommandRequest>,
    ) -> Result<Response<CommandStatus>, Status> {
        self.get_command_impl(req).await
    }
}
