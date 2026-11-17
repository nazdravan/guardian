#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use guardian_core::keys::KeyPair;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::{
    Command, CommandEnvelope, CommandResult, ComputerIds, CreateEnrollCodeRequest,
    CreateRoomRequest, EnrollCodeKind, GetCommandRequest, MessageCmd, ResultStatus, Room,
    SendCommandRequest, ServerFrame, ShutdownCmd, SiteOpenCmd, command, send_command_request,
    server_frame,
};
use guardian_server::testing::{SimAgent, TestServer, req};
use tonic::Code;
use tonic::transport::Channel;

const WAIT: Duration = Duration::from_secs(2);

struct Ctx {
    s: TestServer,
    a: AdminClient<Channel>,
    ctl: ControlClient<Channel>,
    t: String,
}

impl Ctx {
    async fn new() -> Ctx {
        let s = TestServer::start().await;
        let ch = s.grpc().await;
        let t = s.admin_token().await;
        Ctx {
            a: AdminClient::new(ch.clone()),
            ctl: ControlClient::new(ch),
            s,
            t,
        }
    }
    async fn room(&mut self, name: &str) -> Room {
        self.a
            .create_room(req(&self.t, CreateRoomRequest { name: name.into() }))
            .await
            .unwrap()
            .into_inner()
    }
    async fn agent(&mut self, room_id: i64, caps: &[&str]) -> SimAgent {
        let code = self
            .a
            .create_enroll_code(req(
                &self.t,
                CreateEnrollCodeRequest {
                    kind: EnrollCodeKind::EnrollAgent as i32,
                    room_id,
                    max_uses: 1,
                    ttl_seconds: 0,
                },
            ))
            .await
            .unwrap()
            .into_inner()
            .code;
        SimAgent::enroll(&self.s, &code, caps).await.unwrap()
    }
    async fn send(
        &mut self,
        token: &str,
        target: send_command_request::Target,
        kind: command::Kind,
        ttl: u32,
    ) -> Result<String, tonic::Status> {
        self.ctl
            .send_command(req(
                token,
                SendCommandRequest {
                    target: Some(target),
                    command: Some(Command { kind: Some(kind) }),
                    ttl_seconds: ttl,
                },
            ))
            .await
            .map(|r| r.into_inner().command_id)
    }
    async fn status_of(&mut self, id: &str, computer: u64) -> ResultStatus {
        self.s.handle.state().batcher.flush().await;
        let st = self
            .ctl
            .get_command(req(
                &self.t,
                GetCommandRequest {
                    command_id: id.into(),
                },
            ))
            .await
            .unwrap()
            .into_inner();
        let r = st
            .results
            .iter()
            .find(|r| r.computer_id == computer)
            .unwrap();
        ResultStatus::try_from(r.status).unwrap()
    }
}

fn msg(text: &str) -> command::Kind {
    command::Kind::Message(MessageCmd {
        title: "Atenție".into(),
        text: text.into(),
    })
}

fn ids(v: &[u64]) -> send_command_request::Target {
    send_command_request::Target::Computers(ComputerIds { ids: v.to_vec() })
}

async fn next_command(agent: &mut SimAgent) -> CommandEnvelope {
    loop {
        match agent.next_frame(WAIT).await {
            Some(ServerFrame {
                msg: Some(server_frame::Msg::Command(c)),
            }) => return c,
            Some(_) => {}
            None => panic!("expected a command"),
        }
    }
}

#[tokio::test]
async fn command_reaches_online_agent_and_result_is_recorded() {
    let mut c = Ctx::new().await;
    let r = c.room("R").await;
    let mut agent = c.agent(r.id, &["message"]).await;
    let t = c.t.clone();
    let id = c
        .send(
            &t,
            send_command_request::Target::RoomId(r.id),
            msg("Salut"),
            0,
        )
        .await
        .unwrap();
    let env = next_command(&mut agent).await;
    assert_eq!(env.id, id);
    assert!(
        matches!(env.command.unwrap().kind, Some(command::Kind::Message(m)) if m.text == "Salut")
    );
    assert_eq!(
        c.status_of(&id, agent.id()).await,
        ResultStatus::ResultDelivered
    );
    agent
        .send_result(CommandResult {
            command_id: id.clone(),
            status: ResultStatus::ResultOk as i32,
            detail: "shown".into(),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(c.status_of(&id, agent.id()).await, ResultStatus::ResultOk);
    let st = c
        .ctl
        .get_command(req(
            &t,
            GetCommandRequest {
                command_id: id.clone(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(st.expires_at_unix - st.created_at_unix == 600);
}

#[tokio::test]
async fn agent_without_capability_gets_unsupported_without_delivery() {
    let mut c = Ctx::new().await;
    let mut agent = c.agent(0, &["message"]).await;
    let t = c.t.clone();
    let id = c
        .send(
            &t,
            ids(&[agent.id()]),
            command::Kind::Shutdown(ShutdownCmd {
                countdown_seconds: 60,
            }),
            0,
        )
        .await
        .unwrap();
    assert!(agent.next_frame(Duration::from_millis(500)).await.is_none());
    assert_eq!(
        c.status_of(&id, agent.id()).await,
        ResultStatus::ResultUnsupported
    );
}

#[tokio::test]
async fn offline_agent_gets_command_on_reconnect() {
    let mut c = Ctx::new().await;
    let agent = c.agent(0, &["message"]).await;
    let (cid, key) = (agent.id(), KeyPair::from_seed(&agent.key().seed()));
    agent.close().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let t = c.t.clone();
    let id = c.send(&t, ids(&[cid]), msg("Mai târziu"), 0).await.unwrap();
    assert_eq!(c.status_of(&id, cid).await, ResultStatus::ResultPending);
    let agent = SimAgent::connect(&c.s, cid, &key, &["message"])
        .await
        .unwrap();
    assert_eq!(agent.welcome().pending.len(), 1);
    assert_eq!(agent.welcome().pending[0].id, id);
    assert_eq!(c.status_of(&id, cid).await, ResultStatus::ResultDelivered);
}

#[tokio::test]
async fn pending_command_without_capability_becomes_unsupported_on_reconnect() {
    let mut c = Ctx::new().await;
    let agent = c.agent(0, &[]).await;
    let (cid, key) = (agent.id(), KeyPair::from_seed(&agent.key().seed()));
    agent.close().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let t = c.t.clone();
    let id = c.send(&t, ids(&[cid]), msg("x"), 0).await.unwrap();
    let agent = SimAgent::connect(&c.s, cid, &key, &[]).await.unwrap();
    assert!(agent.welcome().pending.is_empty());
    assert_eq!(c.status_of(&id, cid).await, ResultStatus::ResultUnsupported);
}

#[tokio::test]
async fn expired_command_is_not_delivered_later() {
    let mut c = Ctx::new().await; // expiry sweep every 100 ms
    let agent = c.agent(0, &["message"]).await;
    let (cid, key) = (agent.id(), KeyPair::from_seed(&agent.key().seed()));
    agent.close().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let t = c.t.clone();
    let id = c.send(&t, ids(&[cid]), msg("expiră"), 1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert_eq!(c.status_of(&id, cid).await, ResultStatus::ResultExpired);
    let agent = SimAgent::connect(&c.s, cid, &key, &["message"])
        .await
        .unwrap();
    assert!(agent.welcome().pending.is_empty());
}

#[tokio::test]
async fn teacher_cannot_target_computer_outside_rooms() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let r2 = c.room("R2").await;
    let mine = c.agent(r1.id, &["message"]).await;
    let other = c.agent(r2.id, &["message"]).await;
    let roomless = c.agent(0, &["message"]).await;
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    for target in [
        ids(&[mine.id(), other.id()]),
        ids(&[roomless.id()]),
        send_command_request::Target::RoomId(r2.id),
    ] {
        let e = c.send(&tt, target, msg("x"), 0).await.unwrap_err();
        assert_eq!(e.code(), Code::PermissionDenied);
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
        .fetch_one(&c.s.handle.state().db.read)
        .await
        .unwrap();
    assert_eq!(n, 0);
    c.send(&tt, ids(&[mine.id()]), msg("ok"), 0).await.unwrap();
}

#[tokio::test]
async fn invalid_arguments_are_rejected() {
    let mut c = Ctx::new().await;
    let agent = c.agent(0, &["message", "site.open"]).await;
    let t = c.t.clone();
    let target = || ids(&[agent.id()]);
    let cases = [
        (
            command::Kind::SiteOpen(SiteOpenCmd {
                url: "file:///etc/passwd".into(),
            }),
            0,
        ),
        (msg(""), 0),
        (msg(&"x".repeat(2001)), 0),
        (
            command::Kind::Shutdown(ShutdownCmd {
                countdown_seconds: 3601,
            }),
            0,
        ),
        (msg("ok"), 86_401),
    ];
    for (kind, ttl) in cases {
        let e = c.send(&t, target(), kind, ttl).await.unwrap_err();
        assert_eq!(e.code(), Code::InvalidArgument, "{}", e.message());
    }
    let e = c.send(&t, ids(&[]), msg("x"), 0).await.unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let e = c
        .ctl
        .send_command(req(
            &t,
            SendCommandRequest {
                target: Some(target()),
                command: None,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn result_for_foreign_command_is_ignored() {
    let mut c = Ctx::new().await;
    let mut a1 = c.agent(0, &["message"]).await;
    let mut a2 = c.agent(0, &["message"]).await;
    let t = c.t.clone();
    let id = c
        .send(&t, ids(&[a1.id()]), msg("doar a1"), 0)
        .await
        .unwrap();
    next_command(&mut a1).await;
    a2.send_result(CommandResult {
        command_id: id.clone(),
        status: ResultStatus::ResultOk as i32,
        detail: String::new(),
    })
    .await;
    a2.send_result(CommandResult {
        command_id: "not-a-uuid".into(),
        status: ResultStatus::ResultOk as i32,
        detail: String::new(),
    })
    .await;
    // Agents may not claim server-side statuses.
    a1.send_result(CommandResult {
        command_id: id.clone(),
        status: ResultStatus::ResultExpired as i32,
        detail: String::new(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let st = c
        .ctl
        .get_command(req(
            &t,
            GetCommandRequest {
                command_id: id.clone(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(st.results.len(), 1);
    assert_eq!(
        c.status_of(&id, a1.id()).await,
        ResultStatus::ResultDelivered
    );
}
