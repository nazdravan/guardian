//! The connection: connect with retries, resume a saved session or ask for a
//! login, then follow `Watch` until the session ends.

use std::sync::Arc;
use std::time::Duration;

use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::watch_client::WatchClient;
use guardian_proto::v1::{Empty, InitRequest, LoginRequest, LoginResponse};
use tonic::Code;

use super::errors::{login_error_text, status_text};
use super::{ConnState, Core, Event};
use crate::settings::Secret;

const SESSION_ENDED: &str = "Your session has ended. Sign in again.";

/// 1 s, 2 s, 4 s … up to 30 s.
pub(crate) fn backoff(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.saturating_sub(1).min(5);
    Duration::from_secs(secs.min(30))
}

/// Connects (retrying), then resumes the saved session or asks for a login.
pub(crate) async fn run_session(core: Arc<Core>, server: String, pin: Option<String>) {
    core.emit(Event::Connection(ConnState::Connecting));
    let mut attempt = 0u32;
    let channel = loop {
        match guardian_client::connect(&server, pin.as_deref()).await {
            Ok(c) => break c,
            Err(e) => {
                attempt += 1;
                core.emit(Event::Connection(ConnState::Reconnecting {
                    attempt,
                    error: format!("Cannot reach the server: {e}"),
                }));
                tokio::time::sleep(backoff(attempt)).await;
            }
        }
    };
    core.set_channel(channel.clone());
    let Some(token) = core.token() else {
        core.emit_needs_login();
        return;
    };
    let authed = match guardian_client::with_token(channel, &token) {
        Ok(a) => a,
        Err(_) => {
            core.clear_token();
            core.emit_needs_login();
            return;
        }
    };
    match AuthClient::new(authed).who_am_i(Empty {}).await {
        Ok(u) => {
            core.emit(Event::LoggedIn(u.into_inner()));
            watch_loop(core).await;
        }
        Err(st) if st.code() == Code::Unauthenticated => {
            core.clear_token();
            core.emit_needs_login();
        }
        Err(st) => {
            core.emit(Event::Error(status_text(&st)));
            core.emit_needs_login();
        }
    }
}

fn after_login(core: &Arc<Core>, login: &str, r: LoginResponse) {
    core.save_login(login, r.token);
    if let Some(u) = r.user {
        core.emit(Event::LoggedIn(u));
    }
    let c = core.clone();
    core.replace_session(tokio::spawn(watch_loop(c)));
}

/// `Login`.
pub(crate) async fn login(core: Arc<Core>, login: String, password: Secret) {
    let Some(ch) = core.channel() else {
        core.emit(Event::Error("Not connected to a server yet.".into()));
        return;
    };
    match AuthClient::new(ch)
        .login(LoginRequest {
            login: login.clone(),
            password: password.expose().to_string(),
        })
        .await
    {
        Ok(r) => {
            let r = r.into_inner();
            if let Some(u) = &r.user {
                *super::lock(&core.login_secret) = Some((u.id, password.clone()));
                super::fallback::remember_login(&core, u, &password).await;
            }
            after_login(&core, &login, r);
        }
        Err(st) => {
            if st.code() == Code::Unavailable {
                super::fallback::server_unreachable(&core);
            }
            core.emit(Event::Error(login_error_text(&st)));
        }
    }
}

/// `Init`: the first administrator. An empty code means the embedded server's
/// own bootstrap code.
pub(crate) async fn init(core: Arc<Core>, code: String, login: String, password: Secret) {
    let Some(ch) = core.channel() else {
        core.emit(Event::Error("Not connected to a server yet.".into()));
        return;
    };
    let code = if code.trim().is_empty() {
        core.bootstrap().unwrap_or_default()
    } else {
        code
    };
    match AuthClient::new(ch)
        .init(InitRequest {
            bootstrap_code: code,
            login: login.clone(),
            password: password.expose().to_string(),
        })
        .await
    {
        Ok(r) => {
            core.set_bootstrap(None);
            after_login(&core, &login, r.into_inner());
        }
        Err(st) => core.emit(Event::Error(login_error_text(&st))),
    }
}

/// `Logout`: tell the server, forget the token either way.
pub(crate) async fn logout(core: Arc<Core>) {
    core.abort_session();
    if let Some(a) = core.authed() {
        let _ = AuthClient::new(a).logout(Empty {}).await;
    }
    core.clear_token();
    core.reset_fleet();
    core.emit(Event::LoggedOut { reason: None });
    core.emit_needs_login();
}

pub(crate) fn session_ended(core: &Arc<Core>) {
    core.clear_token();
    core.reset_fleet();
    core.emit(Event::LoggedOut {
        reason: Some(SESSION_ENDED.into()),
    });
    core.emit_needs_login();
}

/// Follows `Watch`; reconnects with backoff; ends when the session ends.
pub(crate) async fn watch_loop(core: Arc<Core>) {
    let mut attempt = 0u32;
    loop {
        let error = match core.authed() {
            None => {
                session_ended(&core);
                return;
            }
            Some(a) => match WatchClient::new(a).watch(Empty {}).await {
                Err(st) if st.code() == Code::Unauthenticated => {
                    session_ended(&core);
                    return;
                }
                Err(st) => status_text(&st),
                Ok(resp) => {
                    let mut stream = resp.into_inner();
                    let mut first = true;
                    loop {
                        match stream.message().await {
                            Ok(Some(ev)) => {
                                if first {
                                    first = false;
                                    attempt = 0;
                                    core.emit(Event::Connection(ConnState::Online));
                                }
                                let f = core.apply(ev);
                                core.emit(Event::Fleet(f));
                            }
                            Ok(None) => break "The server closed the connection.".to_string(),
                            Err(st) if st.code() == Code::Unauthenticated => {
                                session_ended(&core);
                                return;
                            }
                            Err(st) => break status_text(&st),
                        }
                    }
                }
            },
        };
        attempt += 1;
        core.emit(Event::Connection(ConnState::Reconnecting {
            attempt,
            error,
        }));
        tokio::time::sleep(backoff(attempt)).await;
    }
}
