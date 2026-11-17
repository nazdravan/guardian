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
