//! gRPC services.

use std::sync::Arc;

use guardian_proto::v1::admin_server::AdminServer;
use guardian_proto::v1::auth_server::AuthServer;
use guardian_proto::v1::control_server::ControlServer;
use guardian_proto::v1::watch_server::WatchServer;

use crate::state::AppState;

pub mod admin;
mod admin_computers;
mod admin_users;
pub mod auth;
pub mod commands;
pub mod control;
pub mod watch;

/// Largest decoded gRPC message (4 MiB).
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// Every gRPC service (plus reflection) as one axum router.
pub fn router(state: Arc<AppState>) -> axum::Router {
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(guardian_proto::FILE_DESCRIPTOR_SET)
        .build_v1();
    let mut routes = tonic::service::Routes::new(
        AuthServer::new(auth::AuthSvc::new(state.clone())).max_decoding_message_size(MAX_MESSAGE),
    )
    .add_service(
        AdminServer::new(admin::AdminSvc::new(state.clone()))
            .max_decoding_message_size(MAX_MESSAGE),
    )
    .add_service(
        ControlServer::new(control::ControlSvc::new(state.clone()))
            .max_decoding_message_size(MAX_MESSAGE),
    )
    .add_service(
        WatchServer::new(watch::WatchSvc::new(state)).max_decoding_message_size(MAX_MESSAGE),
    );
    if let Ok(r) = reflection {
        routes = routes.add_service(r);
    }
    routes.into_axum_router()
}
