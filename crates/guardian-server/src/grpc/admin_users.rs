//! Admin: users and their rooms.

use std::collections::BTreeSet;

use guardian_proto::v1::{
    CreateUserRequest, Empty, Role, SetUserRoomsRequest, UpdateUserRequest, User, UserList,
};
use tonic::{Request, Response, Status};

use super::admin::AdminSvc;
use crate::auth::{hash_password_async, validate_login, validate_password};
use crate::db::{self, users::UpdateOutcome};

fn role_arg(v: i32) -> Result<Role, Status> {
    match Role::try_from(v) {
        Ok(Role::Admin) => Ok(Role::Admin),
        Ok(Role::Teacher) => Ok(Role::Teacher),
        _ => Err(Status::invalid_argument("role must be admin or teacher")),
    }
}

impl AdminSvc {
    async fn user_proto(&self, id: i64) -> Result<User, Status> {
        let u = db::users::get(&self.state.db, id)
            .await?
            .ok_or_else(|| Status::not_found("user not found"))?;
        let rooms = db::users::rooms_of(&self.state.db, id).await?;
        Ok(db::users::to_proto(&u, &rooms))
    }

    pub(crate) async fn list_users_impl(
        &self,
        req: Request<Empty>,
    ) -> Result<Response<UserList>, Status> {
        self.admin(&req).await?;
        let users = db::users::list(&self.state.db).await?;
        let links = db::users::all_rooms(&self.state.db).await?;
        Ok(Response::new(UserList {
            users: users
                .iter()
                .map(|u| {
                    let rooms: BTreeSet<i64> = links
                        .iter()
                        .filter(|(uid, _)| *uid == u.id)
                        .map(|(_, r)| *r)
                        .collect();
                    db::users::to_proto(u, &rooms)
                })
                .collect(),
        }))
    }

    pub(crate) async fn create_user_impl(
        &self,
        req: Request<CreateUserRequest>,
    ) -> Result<Response<User>, Status> {
        let c = self.admin(&req).await?;
        let r = req.into_inner();
        validate_login(&r.login)?;
        validate_password(&r.password)?;
        let role = role_arg(r.role)?;
        let hash = hash_password_async(r.password).await?;
        let mut conn = self
            .state
            .db
            .write
            .acquire()
            .await
            .map_err(db::DbError::from)?;
        let id = db::users::insert(&mut conn, &r.login, &hash, role)
            .await
            .map_err(|e| {
                if e.is_unique_violation() {
                    Status::already_exists("a user with this login already exists")
                } else {
                    e.into()
                }
            })?;
        drop(conn);
        self.state
            .audit(Some(c.user_id), "CreateUser", format!("user:{id}"), r.login);
        if role == Role::Admin {
            self.state.trust_changed().await;
        }
        Ok(Response::new(self.user_proto(id).await?))
    }

    pub(crate) async fn update_user_impl(
        &self,
        req: Request<UpdateUserRequest>,
    ) -> Result<Response<User>, Status> {
        let c = self.admin(&req).await?;
        let r = req.into_inner();
        let role = r.role.map(role_arg).transpose()?;
        let hash = match r.new_password {
            Some(p) => {
                validate_password(&p)?;
                Some(hash_password_async(p).await?)
            }
            None => None,
        };
        match db::users::update(&self.state.db, r.id, role, r.active, hash.as_deref()).await? {
            UpdateOutcome::Updated => {}
            UpdateOutcome::NotFound => return Err(Status::not_found("user not found")),
            UpdateOutcome::LastAdmin => {
                return Err(Status::failed_precondition(
                    "this is the last active admin; create another admin first",
                ));
            }
        }
        let detail = format!(
            "role={:?} active={:?} password_reset={}",
            role,
            r.active,
            hash.is_some()
        );
        self.state.audit(
            Some(c.user_id),
            "UpdateUser",
            format!("user:{}", r.id),
            detail,
        );
        if role.is_some() || r.active.is_some() {
            self.state.trust_changed().await;
            self.state.user_rooms_changed(r.id).await;
        }
        Ok(Response::new(self.user_proto(r.id).await?))
    }

    pub(crate) async fn set_user_rooms_impl(
        &self,
        req: Request<SetUserRoomsRequest>,
    ) -> Result<Response<User>, Status> {
        let c = self.admin(&req).await?;
        let r = req.into_inner();
        if db::users::get(&self.state.db, r.user_id).await?.is_none() {
            return Err(Status::not_found("user not found"));
        }
        if !db::users::set_rooms(&self.state.db, r.user_id, &r.room_ids).await? {
            return Err(Status::invalid_argument(
                "a room in the list does not exist",
            ));
        }
        self.state.audit(
            Some(c.user_id),
            "SetUserRooms",
            format!("user:{}", r.user_id),
            format!("{:?}", r.room_ids),
        );
        self.state.trust_changed().await;
        self.state.user_rooms_changed(r.user_id).await;
        Ok(Response::new(self.user_proto(r.user_id).await?))
    }
}
