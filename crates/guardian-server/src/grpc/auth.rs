//! `guardian.v1.Auth`.

use std::sync::Arc;

use guardian_core::codes::{codes_equal, normalize_code};
use guardian_core::keys::PublicKey;
use guardian_proto::v1::auth_server::Auth;
use guardian_proto::v1::{
    ChangePasswordRequest, Empty, EnrollConsoleRequest, EnrollConsoleResponse, InitRequest,
    LoginRequest, LoginResponse, Role, User,
};
use tonic::{Request, Response, Status};

use crate::auth::{
    SESSION_TTL_SECS, caller_from_request, hash_password_async, new_session_token, validate_login,
    validate_password, verify_password_async,
};
use crate::batch::{AuditEntry, AuditSource};
use crate::db::codes::{CodeError, CodeKind, INVALID_CODE};
use crate::db::{self, now_unix, users::UserRow};
use crate::state::AppState;

const BAD_LOGIN: &str = "invalid login or password";

/// The Auth service.
#[derive(Debug, Clone)]
pub struct AuthSvc {
    state: Arc<AppState>,
}

impl AuthSvc {
    /// Wraps the shared state.
    pub fn new(state: Arc<AppState>) -> AuthSvc {
        AuthSvc { state }
    }

    async fn session_for(&self, user: &UserRow) -> Result<LoginResponse, Status> {
        let (token, hash) = new_session_token();
        let expires_at = now_unix() + SESSION_TTL_SECS;
        db::sessions::create(&self.state.db, &hash, user.id, expires_at).await?;
        let rooms = db::users::rooms_of(&self.state.db, user.id).await?;
        Ok(LoginResponse {
            token,
            expires_at_unix: expires_at,
            user: Some(db::users::to_proto(user, &rooms)),
        })
    }
}

#[tonic::async_trait]
impl Auth for AuthSvc {
    async fn init(&self, req: Request<InitRequest>) -> Result<Response<LoginResponse>, Status> {
        self.state.limiter.check_request(&req)?;
        let r = req.into_inner();
        let Some(expected) = self.state.bootstrap_code() else {
            return Err(Status::failed_precondition("server is already initialized"));
        };
        if !codes_equal(&normalize_code(&r.bootstrap_code), &expected) {
            return Err(Status::permission_denied("wrong bootstrap code"));
        }
        validate_login(&r.login)?;
        validate_password(&r.password)?;
        let hash = hash_password_async(r.password).await?;
        let mut tx = self
            .state
            .db
            .write
            .begin()
            .await
            .map_err(db::DbError::from)?;
        if db::users::count(&mut tx).await? > 0 {
            return Err(Status::failed_precondition("server is already initialized"));
        }
        let id = db::users::insert(&mut tx, &r.login, &hash, Role::Admin).await?;
        tx.commit().await.map_err(db::DbError::from)?;
        if let Ok(mut b) = self.state.bootstrap.lock() {
            *b = None;
        }
        self.state.batcher.audit(AuditEntry {
            actor_user_id: Some(id),
            action: "Init".into(),
            target: format!("user:{id}"),
            detail: r.login.clone(),
            source: AuditSource::Server,
        });
        let user = db::users::get(&self.state.db, id)
            .await?
            .ok_or_else(|| Status::internal("user vanished"))?;
        Ok(Response::new(self.session_for(&user).await?))
    }

    async fn login(&self, req: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        self.state.limiter.check_request(&req)?;
        let r = req.into_inner();
        let user = db::users::by_login(&self.state.db, &r.login).await?;
        let ok =
            verify_password_async(r.password, user.as_ref().map(|u| u.password_hash.clone())).await;
        match user {
            Some(u) if ok && u.active => Ok(Response::new(self.session_for(&u).await?)),
            _ => Err(Status::unauthenticated(BAD_LOGIN)),
        }
    }

    async fn logout(&self, req: Request<Empty>) -> Result<Response<Empty>, Status> {
        let caller = caller_from_request(&self.state, &req).await?;
        db::sessions::delete(&self.state.db, &caller.session_hash).await?;
        Ok(Response::new(Empty {}))
    }

    async fn change_password(
        &self,
        req: Request<ChangePasswordRequest>,
    ) -> Result<Response<Empty>, Status> {
        let caller = caller_from_request(&self.state, &req).await?;
        let r = req.into_inner();
        validate_password(&r.new_password)?;
        let user = db::users::get(&self.state.db, caller.user_id)
            .await?
            .ok_or_else(|| Status::unauthenticated("user not found"))?;
        if !verify_password_async(r.old_password, Some(user.password_hash)).await {
            return Err(Status::permission_denied("old password is wrong"));
        }
        let hash = hash_password_async(r.new_password).await?;
        db::users::set_password(&self.state.db, caller.user_id, &hash).await?;
        db::sessions::delete_for_user(&self.state.db, caller.user_id, Some(&caller.session_hash))
            .await?;
        self.state.batcher.audit(AuditEntry {
            actor_user_id: Some(caller.user_id),
            action: "ChangePassword".into(),
            target: format!("user:{}", caller.user_id),
            detail: String::new(),
            source: AuditSource::Server,
        });
        Ok(Response::new(Empty {}))
    }

    async fn who_am_i(&self, req: Request<Empty>) -> Result<Response<User>, Status> {
        let caller = caller_from_request(&self.state, &req).await?;
        Ok(Response::new(User {
            id: caller.user_id,
            login: caller.login,
            role: caller.role as i32,
            active: true,
            room_ids: caller.room_ids.into_iter().collect(),
        }))
    }

    async fn enroll_console(
        &self,
        req: Request<EnrollConsoleRequest>,
    ) -> Result<Response<EnrollConsoleResponse>, Status> {
        self.state.limiter.check_request(&req)?;
        let r = req.into_inner();
        let key = PublicKey::from_bytes(&r.public_key)
            .map_err(|_| Status::invalid_argument("public_key must be a 32-byte ed25519 key"))?;
        let name = crate::grpc::admin::valid_name(&r.name, "console")?;
        match db::codes::consume(&self.state.db, &r.code, CodeKind::Console).await {
            Ok(_) => {}
            Err(CodeError::Invalid) => return Err(Status::permission_denied(INVALID_CODE)),
            Err(CodeError::Db(e)) => return Err(e.into()),
        }
        let id = db::consoles::insert(&self.state.db, &key, &name)
            .await
            .map_err(|e| {
                if e.is_unique_violation() {
                    Status::already_exists("this console is already enrolled")
                } else {
                    e.into()
                }
            })?;
        self.state
            .audit(None, "EnrollConsole", format!("console:{id}"), name);
        self.state.trust_changed().await;
        Ok(Response::new(EnrollConsoleResponse { console_id: id }))
    }
}
