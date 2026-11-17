//! Executes CLI commands against the server.

use std::io::{BufRead, IsTerminal};

use anyhow::{Context, anyhow, bail};
use guardian_client::Authed;
use guardian_proto::v1 as pb;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::watch_client::WatchClient;
use guardian_proto::v1::{Empty, command, send_command_request, watch_event};
use serde_json::json;
use tonic::transport::Channel;

use crate::session::{self, Session};
use crate::{
    Cli, Cmd, CodeCmd, CommandCmd, ComputerCmd, ConsoleCmd, ModeArg, ProfileCmd, RoleArg, RoomCmd,
    SendArgs, SendWhat, SettingsCmd, UserCmd,
};

type R<T = ()> = anyhow::Result<T>;

fn status(s: tonic::Status) -> anyhow::Error {
    anyhow::Error::new(s)
}

/// Reads a secret: prompt on a terminal, otherwise one line of stdin.
fn read_secret(prompt: &str) -> R<String> {
    if std::io::stdin().is_terminal() {
        return rpassword::prompt_password(prompt).context("reading password");
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim_end_matches(['\r', '\n']).to_string();
    if line.is_empty() {
        bail!("no password given on stdin");
    }
    Ok(line)
}

/// `YYYY-MM-DD HH:MM:SS UTC`.
fn fmt_time(unix: i64) -> String {
    if unix <= 0 {
        return "-".into();
    }
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

fn mode_name(m: i32) -> &'static str {
    match pb::Mode::try_from(m) {
        Ok(pb::Mode::Free) => "free",
        Ok(pb::Mode::Blacklist) => "blacklist",
        Ok(pb::Mode::Whitelist) => "whitelist",
        _ => "?",
    }
}

fn mode_arg(m: ModeArg) -> i32 {
    match m {
        ModeArg::Free => pb::Mode::Free as i32,
        ModeArg::Blacklist => pb::Mode::Blacklist as i32,
        ModeArg::Whitelist => pb::Mode::Whitelist as i32,
    }
}

fn role_arg(r: RoleArg) -> i32 {
    match r {
        RoleArg::Admin => pb::Role::Admin as i32,
        RoleArg::Teacher => pb::Role::Teacher as i32,
    }
}

fn role_name(r: i32) -> &'static str {
    if r == pb::Role::Admin as i32 {
        "admin"
    } else {
        "teacher"
    }
}

fn status_name(s: i32) -> &'static str {
    use pb::ResultStatus as S;
    match S::try_from(s) {
        Ok(S::ResultPending) => "pending",
        Ok(S::ResultDelivered) => "delivered",
        Ok(S::ResultOk) => "ok",
        Ok(S::ResultFailed) => "failed",
        Ok(S::ResultNoUser) => "no_user",
        Ok(S::ResultUnsupported) => "unsupported",
        Ok(S::ResultExpired) => "expired",
        _ => "?",
    }
}

fn feature_name(c: Option<&pb::Command>) -> &'static str {
    match c.and_then(|c| c.kind.as_ref()) {
        Some(command::Kind::Message(_)) => "message",
        Some(command::Kind::ProgramStart(_)) => "program.start",
        Some(command::Kind::SiteOpen(_)) => "site.open",
        Some(command::Kind::Shutdown(_)) => "power.shutdown",
        Some(command::Kind::Restart(_)) => "power.restart",
        None => "?",
    }
}

/// Everything a command needs.
struct Ctx {
    ch: Authed,
    json: bool,
}

impl Ctx {
    fn admin(&self) -> AdminClient<Authed> {
        AdminClient::new(self.ch.clone())
    }
    fn control(&self) -> ControlClient<Authed> {
        ControlClient::new(self.ch.clone())
    }

    /// The first `Watch` event: rooms and profiles visible to the caller.
    async fn snapshot(&self) -> R<pb::Snapshot> {
        let mut st = WatchClient::new(self.ch.clone())
            .watch(Empty {})
            .await
            .map_err(status)?
            .into_inner();
        match st.message().await.map_err(status)? {
            Some(pb::WatchEvent {
                event: Some(watch_event::Event::Snapshot(s)),
            }) => Ok(s),
            _ => bail!("server sent no snapshot"),
        }
    }

    /// Rooms: through Admin, or (for teachers) the Watch snapshot.
    async fn rooms(&self) -> R<Vec<pb::Room>> {
        match self.admin().list_rooms(Empty {}).await {
            Ok(r) => Ok(r.into_inner().rooms),
            Err(s) if s.code() == tonic::Code::PermissionDenied => Ok(self.snapshot().await?.rooms),
            Err(s) => Err(status(s)),
        }
    }

    async fn profiles(&self) -> R<Vec<pb::Profile>> {
        match self.admin().list_profiles(Empty {}).await {
            Ok(r) => Ok(r.into_inner().profiles),
            Err(s) if s.code() == tonic::Code::PermissionDenied => {
                Ok(self.snapshot().await?.profiles)
            }
            Err(s) => Err(status(s)),
        }
    }

    async fn room(&self, key: &str) -> R<pb::Room> {
        let rooms = self.rooms().await?;
        pick(rooms, key, |r| r.id, |r| &r.name).ok_or_else(|| anyhow!("room \"{key}\" not found"))
    }

    async fn profile(&self, key: &str) -> R<pb::Profile> {
        let profiles = self.profiles().await?;
        pick(profiles, key, |p| p.id, |p| &p.name)
            .ok_or_else(|| anyhow!("profile \"{key}\" not found"))
    }

    async fn user(&self, login: &str) -> R<pb::User> {
        self.admin()
            .list_users(Empty {})
            .await
            .map_err(status)?
            .into_inner()
            .users
            .into_iter()
            .find(|u| u.login == login)
            .ok_or_else(|| anyhow!("user \"{login}\" not found"))
    }
}

/// By exact name, else by numeric id.
fn pick<T>(
    items: Vec<T>,
    key: &str,
    id: impl Fn(&T) -> i64,
    name: impl Fn(&T) -> &String,
) -> Option<T> {
    let by_id = key.parse::<i64>().ok();
    let mut by_name = None;
    let mut by_num = None;
    for it in items {
        if name(&it) == key {
            by_name = Some(it);
        } else if by_id == Some(id(&it)) {
            by_num = Some(it);
        }
    }
    by_name.or(by_num)
}

fn print_json(v: &serde_json::Value) -> R {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Entry point.
pub async fn run(cli: Cli) -> R {
    let path = cli.config.clone().unwrap_or_else(session::default_path);
    let mut sess = Session::load(&path)?;
    let server = cli
        .server
        .clone()
        .or_else(|| sess.server.clone())
        .ok_or_else(|| anyhow!("no server: pass --server or set GUARDIAN_SERVER"))?;
    let same_server = sess.server.as_deref() == Some(server.as_str());
    let pin = cli.cert_sha256.clone().or_else(|| {
        if same_server {
            sess.cert_sha256.clone()
        } else {
            None
        }
    });
    let channel: Channel = guardian_client::connect(&server, pin.as_deref())
        .await
        .with_context(|| format!("connecting to {server}"))?;

    let login_response = match &cli.cmd {
        Cmd::Init { code, login } => {
            let password = read_secret("Password for the new admin: ")?;
            Some(
                AuthClient::new(channel.clone())
                    .init(pb::InitRequest {
                        bootstrap_code: code.clone(),
                        login: login.clone(),
                        password,
                    })
                    .await
                    .map_err(status)?
                    .into_inner(),
            )
        }
        Cmd::Login { login } => {
            let password = read_secret("Password: ")?;
            Some(
                AuthClient::new(channel.clone())
                    .login(pb::LoginRequest {
                        login: login.clone(),
                        password,
                    })
                    .await
                    .map_err(status)?
                    .into_inner(),
            )
        }
        _ => None,
    };
    if let Some(r) = login_response {
        sess = Session {
            server: Some(server),
            cert_sha256: pin,
            token: Some(r.token),
        };
        sess.save(&path)?;
        let login = r.user.map(|u| u.login).unwrap_or_default();
        println!(
            "logged in as {login} (session until {})",
            fmt_time(r.expires_at_unix)
        );
        return Ok(());
    }

    let token = match (&sess.token, same_server) {
        (Some(t), true) => t.clone(),
        _ => bail!("not logged in to {server}: run `guardian-cli login --login <name>`"),
    };
    let ch = guardian_client::with_token(channel, &token)?;
    let ctx = Ctx { ch, json: cli.json };
    match cli.cmd {
        Cmd::Init { .. } | Cmd::Login { .. } => Ok(()),
        Cmd::Logout => {
            AuthClient::new(ctx.ch.clone())
                .logout(Empty {})
                .await
                .map_err(status)?;
            sess.token = None;
            sess.save(&path)?;
            println!("logged out");
            Ok(())
        }
        Cmd::Whoami => {
            let u = AuthClient::new(ctx.ch.clone())
                .who_am_i(Empty {})
                .await
                .map_err(status)?
                .into_inner();
            println!("{} ({})", u.login, role_name(u.role));
            Ok(())
        }
        Cmd::Passwd => {
            let old_password = read_secret("Current password: ")?;
            let new_password = read_secret("New password: ")?;
            AuthClient::new(ctx.ch.clone())
                .change_password(pb::ChangePasswordRequest {
                    old_password,
                    new_password,
                })
                .await
                .map_err(status)?;
            println!("password changed; other sessions were ended");
            Ok(())
        }
        Cmd::User(c) => user(&ctx, c).await,
        Cmd::Room(c) => room(&ctx, c).await,
        Cmd::Profile(c) => profile(&ctx, c).await,
        Cmd::Computer(c) => computer(&ctx, c).await,
        Cmd::Code(c) => code(&ctx, c).await,
        Cmd::Console(c) => console(&ctx, c).await,
        Cmd::Send(a) => send(&ctx, a).await,
        Cmd::Command(CommandCmd::Show { id }) => command_show(&ctx, &id).await,
        Cmd::Watch => watch(&ctx).await,
        Cmd::Settings(c) => settings(&ctx, c).await,
    }
}

async fn user(ctx: &Ctx, c: UserCmd) -> R {
    let mut a = ctx.admin();
    match c {
        UserCmd::List => {
            let users = a
                .list_users(Empty {})
                .await
                .map_err(status)?
                .into_inner()
                .users;
            let rooms = ctx.rooms().await?;
            let room_names = |ids: &[i64]| -> Vec<String> {
                ids.iter()
                    .map(|id| {
                        rooms
                            .iter()
                            .find(|r| r.id == *id)
                            .map_or_else(|| id.to_string(), |r| r.name.clone())
                    })
                    .collect()
            };
            if ctx.json {
                return print_json(&json!(
                    users
                        .iter()
                        .map(|u| json!({
                            "id": u.id, "login": u.login, "role": role_name(u.role),
                            "active": u.active, "rooms": room_names(&u.room_ids),
                        }))
                        .collect::<Vec<_>>()
                ));
            }
            for u in &users {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    u.id,
                    u.login,
                    role_name(u.role),
                    if u.active { "active" } else { "disabled" },
                    room_names(&u.room_ids).join(", ")
                );
            }
        }
        UserCmd::Add { login, role } => {
            let password = read_secret(&format!("Password for {login}: "))?;
            let u = a
                .create_user(pb::CreateUserRequest {
                    login,
                    password,
                    role: role_arg(role),
                })
                .await
                .map_err(status)?
                .into_inner();
            println!("user {} created (id {})", u.login, u.id);
        }
        UserCmd::SetRole { login, role } => {
            let u = ctx.user(&login).await?;
            a.update_user(pb::UpdateUserRequest {
                id: u.id,
                role: Some(role_arg(role)),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("{login} is now {}", role_name(role_arg(role)));
        }
        UserCmd::Disable { login } => {
            let u = ctx.user(&login).await?;
            a.update_user(pb::UpdateUserRequest {
                id: u.id,
                active: Some(false),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("{login} disabled");
        }
        UserCmd::Enable { login } => {
            let u = ctx.user(&login).await?;
            a.update_user(pb::UpdateUserRequest {
                id: u.id,
                active: Some(true),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("{login} enabled");
        }
        UserCmd::ResetPassword { login } => {
            let u = ctx.user(&login).await?;
            let p = read_secret(&format!("New password for {login}: "))?;
            a.update_user(pb::UpdateUserRequest {
                id: u.id,
                new_password: Some(p),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("password of {login} reset; their sessions were ended");
        }
        UserCmd::Rooms { login, rooms } => {
            let u = ctx.user(&login).await?;
            let mut ids = Vec::new();
            for r in &rooms {
                ids.push(ctx.room(r).await?.id);
            }
            a.set_user_rooms(pb::SetUserRoomsRequest {
                user_id: u.id,
                room_ids: ids,
            })
            .await
            .map_err(status)?;
            println!(
                "{login}: {}",
                if rooms.is_empty() {
                    "no rooms".into()
                } else {
                    rooms.join(", ")
                }
            );
        }
    }
    Ok(())
}

async fn room(ctx: &Ctx, c: RoomCmd) -> R {
    let mut a = ctx.admin();
    match c {
        RoomCmd::List => {
            let rooms = ctx.rooms().await?;
            let profiles = ctx.profiles().await?;
            let pname = |id: i64| {
                profiles
                    .iter()
                    .find(|p| p.id == id)
                    .map_or_else(|| id.to_string(), |p| p.name.clone())
            };
            if ctx.json {
                return print_json(&json!(rooms.iter().map(|r| json!({
                    "id": r.id, "name": r.name, "profile_id": r.active_profile_id, "profile": pname(r.active_profile_id),
                })).collect::<Vec<_>>()));
            }
            for r in &rooms {
                println!("{}\t{}\t{}", r.id, r.name, pname(r.active_profile_id));
            }
        }
        RoomCmd::Add { name } => {
            let r = a
                .create_room(pb::CreateRoomRequest { name })
                .await
                .map_err(status)?
                .into_inner();
            println!("room {} created (id {})", r.name, r.id);
        }
        RoomCmd::Rename { room, name } => {
            let r = ctx.room(&room).await?;
            a.rename_room(pb::RenameRoomRequest {
                id: r.id,
                name: name.clone(),
            })
            .await
            .map_err(status)?;
            println!("room {} renamed to {name}", r.name);
        }
        RoomCmd::Rm { room } => {
            let r = ctx.room(&room).await?;
            a.delete_room(pb::IdRequest { id: r.id })
                .await
                .map_err(status)?;
            println!("room {} deleted", r.name);
        }
        RoomCmd::Profile { room, profile } => {
            let r = ctx.room(&room).await?;
            let p = ctx.profile(&profile).await?;
            ctx.control()
                .set_room_profile(pb::SetRoomProfileRequest {
                    room_id: r.id,
                    profile_id: p.id,
                })
                .await
                .map_err(status)?;
            println!("{}: {}", r.name, p.name);
        }
    }
    Ok(())
}

async fn profile(ctx: &Ctx, c: ProfileCmd) -> R {
    let mut a = ctx.admin();
    match c {
        ProfileCmd::List => {
            let profiles = ctx.profiles().await?;
            if ctx.json {
                return print_json(&json!(
                    profiles
                        .iter()
                        .map(|p| json!({
                            "id": p.id, "name": p.name, "mode": mode_name(p.mode), "apps": p.apps,
                        }))
                        .collect::<Vec<_>>()
                ));
            }
            for p in &profiles {
                println!(
                    "{}\t{}\t{}\t{}",
                    p.id,
                    p.name,
                    mode_name(p.mode),
                    p.apps.join(", ")
                );
            }
        }
        ProfileCmd::Add { name, mode } => {
            let p = a
                .create_profile(pb::Profile {
                    id: 0,
                    name,
                    mode: mode_arg(mode),
                    apps: vec![],
                })
                .await
                .map_err(status)?
                .into_inner();
            println!("profile {} created (id {})", p.name, p.id);
        }
        ProfileCmd::Mode { profile, mode } => {
            let mut p = ctx.profile(&profile).await?;
            p.mode = mode_arg(mode);
            a.update_profile(p.clone()).await.map_err(status)?;
            println!("{}: {}", p.name, mode_name(p.mode));
        }
        ProfileCmd::Apps { profile, apps } => {
            let mut p = ctx.profile(&profile).await?;
            p.apps = apps;
            let p = a.update_profile(p).await.map_err(status)?.into_inner();
            println!("{}: {}", p.name, p.apps.join(", "));
        }
        ProfileCmd::AddApp { profile, apps } => {
            let mut p = ctx.profile(&profile).await?;
            p.apps.extend(apps);
            let p = a.update_profile(p).await.map_err(status)?.into_inner();
            println!("{}: {}", p.name, p.apps.join(", "));
        }
        ProfileCmd::RmApp { profile, apps } => {
            let mut p = ctx.profile(&profile).await?;
            let drop: Vec<String> = apps.iter().map(|a| a.trim().to_lowercase()).collect();
            p.apps.retain(|a| !drop.contains(a));
            let p = a.update_profile(p).await.map_err(status)?.into_inner();
            println!("{}: {}", p.name, p.apps.join(", "));
        }
        ProfileCmd::Rm { profile } => {
            let p = ctx.profile(&profile).await?;
            a.delete_profile(pb::IdRequest { id: p.id })
                .await
                .map_err(status)?;
            println!("profile {} deleted", p.name);
        }
    }
    Ok(())
}

async fn computer(ctx: &Ctx, c: ComputerCmd) -> R {
    let mut a = ctx.admin();
    match c {
        ComputerCmd::List => {
            let list = a
                .list_computers(Empty {})
                .await
                .map_err(status)?
                .into_inner()
                .computers;
            let rooms = ctx.rooms().await?;
            let profiles = ctx.profiles().await?;
            let rname = |id: i64| {
                rooms
                    .iter()
                    .find(|r| r.id == id)
                    .map_or_else(|| "-".into(), |r| r.name.clone())
            };
            let pname = |id: i64| {
                profiles
                    .iter()
                    .find(|p| p.id == id)
                    .map_or_else(|| "-".into(), |p| p.name.clone())
            };
            if ctx.json {
                return print_json(&json!(list.iter().map(|c| json!({
                    "id": c.id, "name": c.name, "hostname": c.hostname, "os": c.os,
                    "agent_version": c.agent_version, "room": rname(c.room_id),
                    "exception": pname(c.exception_profile_id), "profile": pname(c.effective_profile_id),
                    "online": c.online, "last_seen": c.last_seen_unix, "lan_addresses": c.lan_addresses,
                })).collect::<Vec<_>>()));
            }
            for c in &list {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    c.id,
                    c.name,
                    c.os,
                    rname(c.room_id),
                    pname(c.effective_profile_id),
                    if c.online { "online" } else { "offline" },
                    fmt_time(c.last_seen_unix)
                );
            }
        }
        ComputerCmd::Rename { id, name } => {
            a.update_computer(pb::UpdateComputerRequest {
                id,
                name: Some(name.clone()),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("computer {id}: {name}");
        }
        ComputerCmd::Move { id, room } => {
            let room_id = if room == "-" {
                0
            } else {
                ctx.room(&room).await?.id
            };
            a.update_computer(pb::UpdateComputerRequest {
                id,
                room_id: Some(room_id),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("computer {id}: room {room}");
        }
        ComputerCmd::Except { id, profile } => {
            let pid = if profile == "-" {
                0
            } else {
                ctx.profile(&profile).await?.id
            };
            a.update_computer(pb::UpdateComputerRequest {
                id,
                exception_profile_id: Some(pid),
                ..Default::default()
            })
            .await
            .map_err(status)?;
            println!("computer {id}: exception {profile}");
        }
        ComputerCmd::Revoke { id } => {
            a.revoke_computer(pb::ComputerIdRequest { id })
                .await
                .map_err(status)?;
            println!("computer {id} revoked");
        }
    }
    Ok(())
}

async fn code(ctx: &Ctx, c: CodeCmd) -> R {
    let req = match c {
        CodeCmd::Agent { room, uses, ttl } => {
            let room_id = match room {
                Some(r) => ctx.room(&r).await?.id,
                None => 0,
            };
            pb::CreateEnrollCodeRequest {
                kind: pb::EnrollCodeKind::EnrollAgent as i32,
                room_id,
                max_uses: uses.unwrap_or(if room_id == 0 { 1 } else { 0 }),
                ttl_seconds: ttl.unwrap_or(0),
            }
        }
        CodeCmd::Console => pb::CreateEnrollCodeRequest {
            kind: pb::EnrollCodeKind::EnrollConsole as i32,
            ..Default::default()
        },
    };
    let c = ctx
        .admin()
        .create_enroll_code(req)
        .await
        .map_err(status)?
        .into_inner();
    println!("{}", c.code);
    println!(
        "valid until {}, {} use{}",
        fmt_time(c.expires_at_unix),
        c.max_uses,
        if c.max_uses == 1 { "" } else { "s" }
    );
    Ok(())
}

async fn console(ctx: &Ctx, c: ConsoleCmd) -> R {
    let mut a = ctx.admin();
    match c {
        ConsoleCmd::List => {
            let list = a
                .list_consoles(Empty {})
                .await
                .map_err(status)?
                .into_inner()
                .consoles;
            if ctx.json {
                return print_json(&json!(
                    list.iter()
                        .map(|c| json!({
                            "id": c.id, "name": c.name, "created": c.created_at_unix,
                        }))
                        .collect::<Vec<_>>()
                ));
            }
            for c in &list {
                println!("{}\t{}\t{}", c.id, c.name, fmt_time(c.created_at_unix));
            }
        }
        ConsoleCmd::Revoke { id } => {
            a.revoke_console(pb::IdRequest { id })
                .await
                .map_err(status)?;
            println!("console {id} revoked");
        }
    }
    Ok(())
}

async fn send(ctx: &Ctx, a: SendArgs) -> R {
    let target = match a.room {
        Some(r) => send_command_request::Target::RoomId(ctx.room(&r).await?.id),
        None => send_command_request::Target::Computers(pb::ComputerIds { ids: a.computer }),
    };
    let kind = match a.what {
        SendWhat::Message { text, title } => command::Kind::Message(pb::MessageCmd { title, text }),
        SendWhat::Program { path, args } => {
            command::Kind::ProgramStart(pb::ProgramStartCmd { path, args })
        }
        SendWhat::Site { url } => command::Kind::SiteOpen(pb::SiteOpenCmd { url }),
        SendWhat::Shutdown { countdown } => command::Kind::Shutdown(pb::ShutdownCmd {
            countdown_seconds: countdown,
        }),
        SendWhat::Restart { countdown } => command::Kind::Restart(pb::RestartCmd {
            countdown_seconds: countdown,
        }),
    };
    let r = ctx
        .control()
        .send_command(pb::SendCommandRequest {
            target: Some(target),
            command: Some(pb::Command { kind: Some(kind) }),
            ttl_seconds: a.ttl.unwrap_or(0),
        })
        .await
        .map_err(status)?
        .into_inner();
    println!("{}", r.command_id);
    Ok(())
}

async fn command_show(ctx: &Ctx, id: &str) -> R {
    let st = ctx
        .control()
        .get_command(pb::GetCommandRequest {
            command_id: id.into(),
        })
        .await
        .map_err(status)?
        .into_inner();
    if ctx.json {
        return print_json(&json!({
            "id": st.command_id, "feature": feature_name(st.command.as_ref()),
            "created": st.created_at_unix, "expires": st.expires_at_unix,
            "results": st.results.iter().map(|r| json!({
                "computer": r.computer_id, "status": status_name(r.status), "detail": r.detail,
            })).collect::<Vec<_>>(),
        }));
    }
    println!(
        "command {} ({})",
        st.command_id,
        feature_name(st.command.as_ref())
    );
    println!(
        "created {}, expires {}",
        fmt_time(st.created_at_unix),
        fmt_time(st.expires_at_unix)
    );
    for r in &st.results {
        println!("{}\t{}\t{}", r.computer_id, status_name(r.status), r.detail);
    }
    Ok(())
}

async fn watch(ctx: &Ctx) -> R {
    let mut st = WatchClient::new(ctx.ch.clone())
        .watch(Empty {})
        .await
        .map_err(status)?
        .into_inner();
    loop {
        tokio::select! {
            m = st.message() => match m.map_err(status)? {
                None => return Ok(()),
                Some(pb::WatchEvent { event: Some(e) }) => print_event(ctx.json, &e)?,
                Some(_) => {}
            },
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

fn print_event(as_json: bool, e: &watch_event::Event) -> R {
    use watch_event::Event as E;
    let (kind, text, v) = match e {
        E::Snapshot(s) => (
            "snapshot",
            format!(
                "{} rooms, {} computers ({} online)",
                s.rooms.len(),
                s.computers.len(),
                s.computers.iter().filter(|c| c.online).count()
            ),
            json!({"rooms": s.rooms.len(), "computers": s.computers.len()}),
        ),
        E::Computer(c) => (
            "computer",
            format!(
                "{} {} {}",
                c.id,
                c.name,
                if c.online { "online" } else { "offline" }
            ),
            json!({"id": c.id, "name": c.name, "online": c.online, "room_id": c.room_id, "profile_id": c.effective_profile_id}),
        ),
        E::Room(r) => (
            "room",
            format!("{} {} profile {}", r.id, r.name, r.active_profile_id),
            json!({"id": r.id, "name": r.name, "profile_id": r.active_profile_id}),
        ),
        E::CommandResult(r) => {
            let res = r.result.clone().unwrap_or_default();
            (
                "result",
                format!(
                    "{} computer {} {} {}",
                    r.command_id,
                    res.computer_id,
                    status_name(res.status),
                    res.detail
                ),
                json!({"command": r.command_id, "computer": res.computer_id, "status": status_name(res.status), "detail": res.detail}),
            )
        }
        E::ComputerRemoved(id) => ("computer_removed", id.to_string(), json!({"id": id})),
        E::RoomRemoved(id) => ("room_removed", id.to_string(), json!({"id": id})),
    };
    if as_json {
        println!("{}", json!({"event": kind, "data": v}));
    } else {
        println!("{kind}\t{text}");
    }
    Ok(())
}

async fn settings(ctx: &Ctx, c: SettingsCmd) -> R {
    let mut a = ctx.admin();
    let mut s = a.get_settings(Empty {}).await.map_err(status)?.into_inner();
    if let SettingsCmd::Set {
        default_profile,
        retention_days,
    } = c
    {
        if let Some(p) = default_profile {
            s.default_profile_id = ctx.profile(&p).await?.id;
        }
        if let Some(d) = retention_days {
            s.retention_days = d;
        }
        s = a.update_settings(s).await.map_err(status)?.into_inner();
    }
    let profiles = ctx.profiles().await?;
    let pname = profiles
        .iter()
        .find(|p| p.id == s.default_profile_id)
        .map_or_else(|| s.default_profile_id.to_string(), |p| p.name.clone());
    if ctx.json {
        return print_json(
            &json!({"default_profile": pname, "default_profile_id": s.default_profile_id, "retention_days": s.retention_days}),
        );
    }
    println!("default profile\t{pname}");
    println!("retention days\t{}", s.retention_days);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_formatted_in_utc() {
        assert_eq!(fmt_time(0), "-");
        assert_eq!(fmt_time(1_700_000_000), "2023-11-14 22:13:20 UTC");
        assert_eq!(fmt_time(951_782_400), "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn names_win_over_ids() {
        let rooms = vec![
            pb::Room {
                id: 12,
                name: "Sala 3".into(),
                active_profile_id: 1,
            },
            pb::Room {
                id: 3,
                name: "12".into(),
                active_profile_id: 1,
            },
        ];
        assert_eq!(
            pick(rooms.clone(), "12", |r| r.id, |r| &r.name).map(|r| r.id),
            Some(3)
        );
        assert_eq!(
            pick(rooms.clone(), "3", |r| r.id, |r| &r.name).map(|r| r.id),
            Some(3)
        );
        assert_eq!(pick(rooms, "nope", |r| r.id, |r| &r.name), None);
    }
}
