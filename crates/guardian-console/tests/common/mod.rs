//! Shared helpers for the backend tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use guardian_console::backend::{Action, Backend, ConnState, Event, RealBackend};
use guardian_console::fleet::Fleet;
use guardian_console::settings::Secret;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{CreateEnrollCodeRequest, CreateRoomRequest, EnrollCodeKind, User};
use guardian_server::testing::{TestServer, req};

pub const WAIT: Duration = Duration::from_secs(10);

/// A backend on its own settings file, with every event it sent kept.
pub struct Console {
    pub b: RealBackend,
    pub path: PathBuf,
    dir: Option<tempfile::TempDir>,
    pub seen: Mutex<Vec<Event>>,
    pending: Mutex<VecDeque<Event>>,
    last_fleet: Mutex<Option<Fleet>>,
}

impl Console {
    pub fn new() -> Console {
        Console::at(tempfile::tempdir().unwrap())
    }

    pub fn at(dir: tempfile::TempDir) -> Console {
        let path = dir.path().join("console.toml");
        Console {
            b: RealBackend::start(path.clone(), Box::new(|| {})),
            path,
            dir: Some(dir),
            seen: Mutex::new(Vec::new()),
            pending: Mutex::new(VecDeque::new()),
            last_fleet: Mutex::new(None),
        }
    }

    /// Stops the backend and hands back its directory.
    pub fn into_dir(mut self) -> tempfile::TempDir {
        self.dir.take().unwrap()
    }

    /// Polls until an event matching `pred` arrives; returns the events since
    /// the call.
    pub async fn until(&self, pred: impl Fn(&Event) -> bool, what: &str) -> Vec<Event> {
        let end = Instant::now() + WAIT;
        let mut got = Vec::new();
        loop {
            {
                let mut q = self.pending.lock().unwrap();
                q.extend(self.b.poll());
                while let Some(e) = q.pop_front() {
                    if let Event::Fleet(f) = &e {
                        *self.last_fleet.lock().unwrap() = Some(f.clone());
                    }
                    self.seen.lock().unwrap().push(e.clone());
                    let hit = pred(&e);
                    got.push(e);
                    if hit {
                        return got;
                    }
                }
            }
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}: {got:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The latest fleet matching `pred`: the last one already seen, or the next.
    pub async fn fleet_where(&self, pred: impl Fn(&Fleet) -> bool) -> Fleet {
        if let Some(f) = self.last_fleet.lock().unwrap().clone().filter(|f| pred(f)) {
            return f;
        }
        let evs = self
            .until(
                |e| matches!(e, Event::Fleet(f) if pred(f)),
                "a matching fleet",
            )
            .await;
        match evs.into_iter().last() {
            Some(Event::Fleet(f)) => f,
            _ => unreachable!(),
        }
    }

    pub async fn connect(&self, s: &TestServer) {
        self.b.send(Action::Connect {
            server: s.url(),
            cert_sha256: s.pin(),
        });
        self.until(
            |e| matches!(e, Event::Connection(ConnState::NeedsLogin { .. })),
            "the sign-in prompt",
        )
        .await;
    }

    pub async fn login(&self, s: &TestServer, login: &str, password: &str) -> User {
        self.connect(s).await;
        self.b.send(Action::Login {
            login: login.into(),
            password: Secret::new(password),
        });
        let evs = self
            .until(|e| matches!(e, Event::LoggedIn(_)), "logged in")
            .await;
        match evs.into_iter().last() {
            Some(Event::LoggedIn(u)) => u,
            _ => unreachable!(),
        }
    }
}

/// A room and a room enrollment code for it.
pub async fn room_with_code(s: &TestServer, t: &str, name: &str) -> (i64, String) {
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(t, CreateRoomRequest { name: name.into() }))
        .await
        .unwrap()
        .into_inner();
    let code = a
        .create_enroll_code(req(
            t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: room.id,
                max_uses: 10,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    (room.id, code)
}
