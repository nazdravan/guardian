//! `guardian.v1.Watch` — live room and computer state for consoles.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use guardian_core::policy::Profile;
use guardian_proto::v1::watch_server::Watch;
use guardian_proto::v1::{
    ComputerResult, ComputerResultEvent, Empty, Role, Snapshot, WatchEvent, watch_event,
};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::auth::{Caller, bearer, caller_from_token};
use crate::db::{self, DbError};
use crate::hub::HubEvent;
use crate::state::AppState;

/// How often an open stream re-checks that its session is still valid.
const RECHECK: Duration = Duration::from_secs(60);

/// The Watch service.
#[derive(Debug, Clone)]
pub struct WatchSvc {
    state: Arc<AppState>,
}

impl WatchSvc {
    /// Wraps the shared state.
    pub fn new(state: Arc<AppState>) -> WatchSvc {
        WatchSvc { state }
    }
}

type Out = mpsc::Sender<Result<WatchEvent, Status>>;

fn ev(e: watch_event::Event) -> Result<WatchEvent, Status> {
    Ok(WatchEvent { event: Some(e) })
}

/// One stream's view: who is watching and what they have been shown.
struct View {
    caller: Caller,
    /// Computers this stream has been told about.
    known: BTreeSet<u64>,
}

impl View {
    fn admin(&self) -> bool {
        self.caller.role == Role::Admin
    }

    fn sees_room(&self, room: Option<i64>) -> bool {
        match room {
            Some(r) => self.caller.can_control_room(r),
            None => self.admin(),
        }
    }
}

async fn snapshot(state: &AppState, view: &mut View) -> Result<Snapshot, DbError> {
    let rooms = db::rooms::list(&state.db)
        .await?
        .into_iter()
        .filter(|r| view.sees_room(Some(r.id)))
        .collect();
    let default = db::settings::get(&state.db).await?.default_profile_id;
    let computers: Vec<_> = db::computers::list(&state.db)
        .await?
        .into_iter()
        .filter(|c| view.sees_room(c.room_id))
        .map(|c| c.to_proto(state.hub.is_online(c.id), default))
        .collect();
    view.known = computers.iter().map(|c| c.id).collect();
    let profiles = db::profiles::list(&state.db)
        .await?
        .iter()
        .map(Profile::to_proto)
        .collect();
    Ok(Snapshot {
        rooms,
        computers,
        profiles,
    })
}

/// Turns one hub event into what this stream should see (possibly nothing).
async fn translate(
    state: &AppState,
    view: &mut View,
    e: HubEvent,
) -> Result<Vec<watch_event::Event>, DbError> {
    let mut out = Vec::new();
    match e {
        HubEvent::ComputerChanged(id) => {
            let row = db::computers::get(&state.db, id).await?;
            match row {
                Some(c) if view.sees_room(c.room_id) => {
                    let default = db::settings::get(&state.db).await?.default_profile_id;
                    view.known.insert(id);
                    out.push(watch_event::Event::Computer(
                        c.to_proto(state.hub.is_online(id), default),
                    ));
                }
                // Moved out of sight, or deleted.
                _ => {
                    if view.known.remove(&id) {
                        out.push(watch_event::Event::ComputerRemoved(id));
                    }
                }
            }
        }
        HubEvent::ComputerRemoved(id) => {
            if view.known.remove(&id) {
                out.push(watch_event::Event::ComputerRemoved(id));
            }
        }
        HubEvent::RoomChanged(r) => {
            if view.sees_room(Some(r)) {
                if let Some(room) = db::rooms::get(&state.db, r).await? {
                    out.push(watch_event::Event::Room(room));
                }
            }
        }
        HubEvent::RoomRemoved(r) => {
            if view.sees_room(Some(r)) {
                view.caller.room_ids.remove(&r);
                out.push(watch_event::Event::RoomRemoved(r));
            }
        }
        HubEvent::CommandResult {
            command_id,
            computer_id,
            status,
            detail,
        } => {
            if view.known.contains(&computer_id) {
                out.push(watch_event::Event::CommandResult(ComputerResultEvent {
                    command_id: command_id.to_string(),
                    result: Some(ComputerResult {
                        computer_id,
                        status: status as i32,
                        detail,
                    }),
                }));
            }
        }
        HubEvent::UserRoomsChanged(_) => {}
    }
    Ok(out)
}

async fn send_snapshot(state: &AppState, view: &mut View, tx: &Out) -> bool {
    match snapshot(state, view).await {
        Ok(s) => tx.send(ev(watch_event::Event::Snapshot(s))).await.is_ok(),
        Err(e) => {
            let _ = tx.send(Err(e.into())).await;
            false
        }
    }
}

async fn run(
    state: Arc<AppState>,
    token: String,
    caller: Caller,
    mut rx: broadcast::Receiver<HubEvent>,
    tx: Out,
) {
    let mut view = View {
        caller,
        known: BTreeSet::new(),
    };
    if !send_snapshot(&state, &mut view, &tx).await {
        return;
    }
    let mut recheck = tokio::time::interval(RECHECK);
    recheck.tick().await;
    loop {
        let event = tokio::select! {
            e = rx.recv() => e,
            _ = tx.closed() => return,
            _ = recheck.tick() => {
                if let Err(s) = caller_from_token(&state, &token).await {
                    let _ = tx.send(Err(s)).await;
                    return;
                }
                continue;
            }
        };
        match event {
            Ok(HubEvent::UserRoomsChanged(uid)) if uid == view.caller.user_id => {
                match caller_from_token(&state, &token).await {
                    Ok(c) => view.caller = c,
                    Err(s) => {
                        let _ = tx.send(Err(s)).await;
                        return;
                    }
                }
                if !send_snapshot(&state, &mut view, &tx).await {
                    return;
                }
            }
            Ok(e) => match translate(&state, &mut view, e).await {
                Ok(events) => {
                    for e in events {
                        if tx.send(ev(e)).await.is_err() {
                            return;
                        }
                    }
                }
                Err(e) => tracing::error!("watch: {e}"),
            },
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("watch stream lagged by {n} events; resending snapshot");
                if !send_snapshot(&state, &mut view, &tx).await {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[tonic::async_trait]
impl Watch for WatchSvc {
    type WatchStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;

    async fn watch(&self, req: Request<Empty>) -> Result<Response<Self::WatchStream>, Status> {
        let token = bearer(&req)
            .ok_or_else(|| Status::unauthenticated("missing token"))?
            .to_string();
        let caller = caller_from_token(&self.state, &token).await?;
        // Subscribe before the snapshot so nothing between the two is lost.
        let rx = self.state.hub.events();
        let (tx, out) = mpsc::channel(256);
        tokio::spawn(run(self.state.clone(), token, caller, rx, tx));
        Ok(Response::new(Box::pin(ReceiverStream::new(out))))
    }
}
