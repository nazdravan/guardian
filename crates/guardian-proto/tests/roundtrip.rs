use guardian_proto::v1::{
    AgentFrame, AgentState, Enroll, Mode, Profile, ServerFrame, Welcome, agent_frame, server_frame,
};
use prost::Message;

#[test]
fn agent_frame_roundtrips() {
    let f = AgentFrame {
        msg: Some(agent_frame::Msg::Enroll(Enroll {
            code: "ABCD2345".into(),
            public_key: vec![7; 32],
            info: None,
        })),
    };
    let back = AgentFrame::decode(f.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back, f);
}

#[test]
fn welcome_carries_profiles() {
    let p = Profile {
        id: 2,
        name: "Lecție".into(),
        mode: Mode::Blacklist as i32,
        apps: vec!["chrome".into()],
    };
    let f = ServerFrame {
        msg: Some(server_frame::Msg::Welcome(Welcome {
            state: Some(AgentState {
                version: 1,
                effective_profile_id: 2,
                profiles: vec![p],
                room_id: 0,
                exception_profile_id: 0,
            }),
            ..Default::default()
        })),
    };
    let back = ServerFrame::decode(f.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back, f);
}

#[test]
fn descriptor_set_is_embedded() {
    assert!(!guardian_proto::FILE_DESCRIPTOR_SET.is_empty());
}

#[test]
fn stored_state_and_worker_frames_roundtrip() {
    use guardian_proto::v1::{
        AuthorizedUser, StoredState, TrustedConsole, WorkerFrame, WorkerHello, worker_frame,
    };
    let st = StoredState {
        state: Some(AgentState {
            version: 3,
            effective_profile_id: 2,
            profiles: vec![],
            room_id: 1,
            exception_profile_id: 0,
        }),
        consoles: vec![TrustedConsole {
            console_id: 1,
            public_key: vec![1; 32],
        }],
        users: vec![AuthorizedUser {
            user_id: 5,
            login: "ion".into(),
        }],
        saved_at_unix: 1_700_000_000,
        computer_id: 7,
    };
    assert_eq!(
        StoredState::decode(st.encode_to_vec().as_slice()).unwrap(),
        st
    );
    let w = WorkerFrame {
        msg: Some(worker_frame::Msg::Hello(WorkerHello {
            pid: 42,
            session_id: "2".into(),
            user: "elev".into(),
            worker_version: "3.0.0".into(),
        })),
    };
    assert_eq!(
        WorkerFrame::decode(w.encode_to_vec().as_slice()).unwrap(),
        w
    );
}
