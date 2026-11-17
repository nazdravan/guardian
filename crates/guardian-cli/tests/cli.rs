#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use assert_cmd::Command;
use guardian_proto::v1::{ServerFrame, server_frame};
use guardian_server::testing::{SimAgent, TestServer};
use predicates::prelude::*;

struct Cli {
    server: String,
    config: PathBuf,
    _dir: tempfile::TempDir,
}

impl Cli {
    fn new(s: &TestServer) -> Cli {
        let dir = tempfile::tempdir().unwrap();
        Cli {
            server: s.url(),
            config: dir.path().join("cli.toml"),
            _dir: dir,
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::cargo_bin("guardian-cli").unwrap();
        c.args([
            "--server",
            &self.server,
            "--config",
            self.config.to_str().unwrap(),
        ]);
        c.args(args);
        c.timeout(Duration::from_secs(20));
        c
    }

    /// Runs on a blocking thread so the in-process server keeps serving.
    async fn run(&self, args: &[&str], stdin: Option<&str>) -> assert_cmd::assert::Assert {
        let mut c = self.cmd(args);
        if let Some(i) = stdin {
            c.write_stdin(i.to_string());
        }
        tokio::task::spawn_blocking(move || c.assert())
            .await
            .unwrap()
    }

    async fn ok(&self, args: &[&str]) -> String {
        let a = self.run(args, None).await.success();
        String::from_utf8(a.get_output().stdout.clone()).unwrap()
    }

    async fn init(&self, s: &TestServer) {
        let code = s.handle.bootstrap_code().unwrap();
        self.run(
            &["init", "--code", &code, "--login", "admin"],
            Some("admin-password\n"),
        )
        .await
        .success();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn init_login_and_whoami() {
    let s = TestServer::start().await;
    let cli = Cli::new(&s);
    cli.init(&s).await;
    assert!(cli.ok(&["whoami"]).await.contains("admin"));
    cli.ok(&["logout"]).await;
    cli.run(&["whoami"], None).await.failure();
    cli.run(&["login", "--login", "admin"], Some("admin-password\n"))
        .await
        .success();
    assert!(cli.ok(&["whoami"]).await.contains("admin"));
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_and_room_flow() {
    let s = TestServer::start().await;
    let cli = Cli::new(&s);
    cli.init(&s).await;
    cli.ok(&["profile", "add", "Desen", "--mode", "whitelist"])
        .await;
    cli.ok(&["profile", "apps", "Desen", "paint.exe", "MSPaint.exe"])
        .await;
    cli.ok(&["room", "add", "Sala 12"]).await;
    cli.ok(&["room", "profile", "Sala 12", "Desen"]).await;
    let rooms: serde_json::Value =
        serde_json::from_str(&cli.ok(&["--json", "room", "list"]).await).unwrap();
    let room = rooms
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "Sala 12")
        .unwrap();
    assert_eq!(room["profile"], "Desen");
    let profiles: serde_json::Value =
        serde_json::from_str(&cli.ok(&["--json", "profile", "list"]).await).unwrap();
    let p = profiles
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Desen")
        .unwrap();
    assert_eq!(p["mode"], "whitelist");
    assert_eq!(p["apps"], serde_json::json!(["mspaint.exe", "paint.exe"]));
    cli.ok(&["profile", "rm-app", "Desen", "paint.exe"]).await;
    cli.run(
        &["user", "add", "vasile", "--role", "teacher"],
        Some("vasile-password\n"),
    )
    .await
    .success();
    cli.ok(&["user", "rooms", "vasile", "Sala 12"]).await;
    let users = cli.ok(&["user", "list"]).await;
    assert!(
        users.contains("vasile") && users.contains("Sala 12"),
        "{users}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_to_room_reports_command_id() {
    let s = TestServer::start().await;
    let cli = Cli::new(&s);
    cli.init(&s).await;
    cli.ok(&["room", "add", "Sala 12"]).await;
    let out = cli
        .ok(&["code", "agent", "--room", "Sala 12", "--uses", "5"])
        .await;
    let code = out.lines().next().unwrap().trim().to_string();
    assert_eq!(code.len(), 8, "{out}");
    let mut agent = SimAgent::enroll(&s, &code, &["message"]).await.unwrap();
    let out = cli
        .ok(&["send", "--room", "Sala 12", "message", "--text", "Salut"])
        .await;
    let id = out.trim();
    assert_eq!(id.len(), 36, "{out}");
    match agent.next_frame(Duration::from_secs(3)).await {
        Some(ServerFrame {
            msg: Some(server_frame::Msg::Command(c)),
        }) => assert_eq!(c.id, id),
        other => panic!("{other:?}"),
    }
    let show = cli.ok(&["command", "show", id]).await;
    assert!(show.contains("delivered"), "{show}");
    let list = cli.ok(&["computer", "list"]).await;
    assert!(list.contains("online"), "{list}");
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_exit_nonzero() {
    let s = TestServer::start().await;
    let cli = Cli::new(&s);
    cli.init(&s).await;
    cli.run(&["room", "profile", "Nu există", "Lecție"], None)
        .await
        .failure()
        .stderr(predicate::str::contains("Nu există"));
    cli.run(&["profile", "add", "Lecție", "--mode", "free"], None)
        .await
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn session_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let s = TestServer::start().await;
    let cli = Cli::new(&s);
    cli.init(&s).await;
    let mode = std::fs::metadata(&cli.config).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let text = std::fs::read_to_string(&cli.config).unwrap();
    assert!(text.contains("token"));
}

#[tokio::test(flavor = "multi_thread")]
async fn works_over_tls_with_pin() {
    let s = TestServer::start_tls().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("cli.toml");
    let pin = s.pin().unwrap();
    let code = s.handle.bootstrap_code().unwrap();
    let url = s.url();
    let (c1, c2) = (config.clone(), config.clone());
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("guardian-cli")
            .unwrap()
            .args([
                "--server",
                &url,
                "--cert-sha256",
                &pin,
                "--config",
                c1.to_str().unwrap(),
                "init",
                "--code",
                &code,
                "--login",
                "admin",
            ])
            .write_stdin("admin-password\n")
            .assert()
            .success();
        // Server and pin are remembered in the session file.
        Command::cargo_bin("guardian-cli")
            .unwrap()
            .args(["--config", c2.to_str().unwrap(), "whoami"])
            .assert()
            .success()
            .stdout(predicate::str::contains("admin"));
    })
    .await
    .unwrap();
}
