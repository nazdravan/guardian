//! Control and administration calls. Each reports `Done(<short text>)` or
//! `Error(<text>)`; list-returning ones also send the fresh list.

use std::sync::Arc;

use guardian_client::Authed;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::{
    ChangePasswordRequest, ComputerIdRequest, CreateEnrollCodeRequest, CreateRoomRequest,
    CreateUserRequest, Empty, EnrollCodeKind, IdRequest, RenameRoomRequest, SetRoomProfileRequest,
    SetUserRoomsRequest, UpdateComputerRequest, UpdateUserRequest,
};

use super::errors::status_text;
use super::{Action, Core, Event};

fn authed(core: &Core) -> Option<Authed> {
    let a = core.authed();
    if a.is_none() {
        core.emit(Event::Error("Not signed in.".into()));
    }
    a
}

async fn load_profiles(core: &Core, a: Authed) {
    match AdminClient::new(a).list_profiles(Empty {}).await {
        Ok(r) => {
            let ps = r.into_inner().profiles;
            let f = core.set_profiles(ps.clone());
            core.emit(Event::Profiles(ps));
            core.emit(Event::Fleet(f));
        }
        Err(st) => core.emit(Event::Error(status_text(&st))),
    }
}

async fn load_users(core: &Core, a: Authed) {
    match AdminClient::new(a).list_users(Empty {}).await {
        Ok(r) => core.emit(Event::Users(r.into_inner().users)),
        Err(st) => core.emit(Event::Error(status_text(&st))),
    }
}

/// Runs one admin/control action.
pub(crate) async fn run(core: Arc<Core>, action: Action) {
    let Some(a) = authed(&core) else { return };
    let mut admin = AdminClient::new(a.clone());
    let done = |m: String| core.emit(Event::Done(m));
    let fail = |st: tonic::Status| core.emit(Event::Error(status_text(&st)));
    match action {
        Action::SetRoomProfile {
            room_id,
            profile_id,
        } => match ControlClient::new(a)
            .set_room_profile(SetRoomProfileRequest {
                room_id,
                profile_id,
            })
            .await
        {
            Ok(r) => {
                let r = r.into_inner();
                let p = core
                    .profile_name(r.active_profile_id)
                    .unwrap_or_else(|| format!("profile {}", r.active_profile_id));
                done(format!("{}: {p}", r.name));
            }
            Err(st) => fail(st),
        },
        Action::CreateRoom { name } => match admin.create_room(CreateRoomRequest { name }).await {
            Ok(r) => done(format!("Room {:?} created", r.into_inner().name)),
            Err(st) => fail(st),
        },
        Action::RenameRoom { id, name } => {
            match admin.rename_room(RenameRoomRequest { id, name }).await {
                Ok(r) => done(format!("Room renamed to {:?}", r.into_inner().name)),
                Err(st) => fail(st),
            }
        }
        Action::DeleteRoom { id } => match admin.delete_room(IdRequest { id }).await {
            Ok(_) => done("Room deleted".into()),
            Err(st) => fail(st),
        },
        Action::LoadProfiles => load_profiles(&core, a).await,
        Action::SaveProfile(p) => {
            let r = if p.id == 0 {
                admin.create_profile(p).await
            } else {
                admin.update_profile(p).await
            };
            match r {
                Ok(r) => {
                    done(format!("Profile {:?} saved", r.into_inner().name));
                    load_profiles(&core, a).await;
                }
                Err(st) => fail(st),
            }
        }
        Action::DeleteProfile { id } => match admin.delete_profile(IdRequest { id }).await {
            Ok(_) => {
                done("Profile deleted".into());
                load_profiles(&core, a).await;
            }
            Err(st) => fail(st),
        },
        Action::UpdateComputer {
            id,
            name,
            room_id,
            exception_profile_id,
        } => match admin
            .update_computer(UpdateComputerRequest {
                id,
                name,
                room_id,
                exception_profile_id,
            })
            .await
        {
            Ok(r) => done(format!("Computer {:?} saved", r.into_inner().name)),
            Err(st) => fail(st),
        },
        Action::RevokeComputer { id } => {
            match admin.revoke_computer(ComputerIdRequest { id }).await {
                Ok(_) => done("Computer revoked".into()),
                Err(st) => fail(st),
            }
        }
        Action::LoadUsers => load_users(&core, a).await,
        Action::CreateUser {
            login,
            password,
            role,
        } => match admin
            .create_user(CreateUserRequest {
                login,
                password: password.expose().to_string(),
                role: role as i32,
            })
            .await
        {
            Ok(r) => {
                done(format!("User {:?} created", r.into_inner().login));
                load_users(&core, a).await;
            }
            Err(st) => fail(st),
        },
        Action::UpdateUser {
            id,
            role,
            active,
            new_password,
        } => match admin
            .update_user(UpdateUserRequest {
                id,
                role: role.map(|r| r as i32),
                active,
                new_password: new_password.map(|p| p.expose().to_string()),
            })
            .await
        {
            Ok(r) => {
                done(format!("User {:?} saved", r.into_inner().login));
                load_users(&core, a).await;
            }
            Err(st) => fail(st),
        },
        Action::SetUserRooms { user_id, room_ids } => {
            match admin
                .set_user_rooms(SetUserRoomsRequest { user_id, room_ids })
                .await
            {
                Ok(r) => {
                    done(format!("Rooms of {:?} saved", r.into_inner().login));
                    load_users(&core, a).await;
                }
                Err(st) => fail(st),
            }
        }
        Action::CreateAgentCode {
            room_id,
            max_uses,
            ttl_seconds,
        } => match admin
            .create_enroll_code(CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id,
                max_uses,
                ttl_seconds,
            })
            .await
        {
            Ok(r) => core.emit(Event::Code(r.into_inner())),
            Err(st) => fail(st),
        },
        Action::LoadSettings => match admin.get_settings(Empty {}).await {
            Ok(r) => core.emit(Event::Settings(r.into_inner())),
            Err(st) => fail(st),
        },
        Action::SaveSettings(s) => match admin.update_settings(s).await {
            Ok(r) => {
                done("Settings saved".into());
                core.emit(Event::Settings(r.into_inner()));
            }
            Err(st) => fail(st),
        },
        Action::ChangePassword { old, new } => match AuthClient::new(a)
            .change_password(ChangePasswordRequest {
                old_password: old.expose().to_string(),
                new_password: new.expose().to_string(),
            })
            .await
        {
            Ok(_) => done("Password changed".into()),
            Err(st) => fail(st),
        },
        Action::Connect { .. }
        | Action::Login { .. }
        | Action::Init { .. }
        | Action::Logout
        | Action::StartEmbedded { .. }
        | Action::StopEmbedded
        | Action::OpenFirewall
        | Action::Shutdown => {}
    }
}
