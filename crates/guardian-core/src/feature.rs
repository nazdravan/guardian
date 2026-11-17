//! The feature registry: stable ids for every command an agent can run.

use guardian_proto::v1::command::Kind;
use uuid::Uuid;

/// Version of the agent WebSocket protocol.
pub const PROTOCOL_VERSION: u32 = 1;

/// Stable identifier of a feature (never changes once published).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeatureId(pub Uuid);

/// One feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureSpec {
    /// Stable id.
    pub id: FeatureId,
    /// Name used in `Hello.capabilities`.
    pub name: &'static str,
    /// Whether the local (offline) channel may trigger it.
    pub allowed_offline: bool,
}

const fn spec(id: u128, name: &'static str, allowed_offline: bool) -> FeatureSpec {
    FeatureSpec {
        id: FeatureId(Uuid::from_u128(id)),
        name,
        allowed_offline,
    }
}

/// `message`
pub const MESSAGE: FeatureSpec = spec(0x6f612c55_6c31_4b31_969b_9fbcbe7fd340, "message", true);
/// `program.start`
pub const PROGRAM_START: FeatureSpec = spec(
    0x4a182ff4_acc0_406c_8939_596a31618bdc,
    "program.start",
    true,
);
/// `site.open`
pub const SITE_OPEN: FeatureSpec = spec(0x99c80159_9d0f_4b3d_b0f7_b0510a89ec11, "site.open", true);
/// `power.shutdown`
pub const POWER_SHUTDOWN: FeatureSpec = spec(
    0x99d2affe_9035_4e2b_96d2_2c89c6ef8b53,
    "power.shutdown",
    true,
);
/// `power.restart`
pub const POWER_RESTART: FeatureSpec = spec(
    0x6d7c62ea_e047_46fe_8720_73f2671c9c55,
    "power.restart",
    true,
);
/// `files.push` (stage 4)
pub const FILES_PUSH: FeatureSpec =
    spec(0xad82cfb6_0ca9_41ff_8e41_98f9cc4b98cd, "files.push", false);
/// `files.collect` (stage 4)
pub const FILES_COLLECT: FeatureSpec = spec(
    0xdaaf0b70_e7d9_4259_bc9f_1967bf287699,
    "files.collect",
    false,
);

/// Every feature.
pub const ALL_FEATURES: &[FeatureSpec] = &[
    MESSAGE,
    PROGRAM_START,
    SITE_OPEN,
    POWER_SHUTDOWN,
    POWER_RESTART,
    FILES_PUSH,
    FILES_COLLECT,
];

/// Looks a feature up by name.
pub fn by_name(name: &str) -> Option<&'static FeatureSpec> {
    ALL_FEATURES.iter().find(|f| f.name == name)
}

/// The feature a command belongs to.
pub fn feature_of(kind: &Kind) -> &'static FeatureSpec {
    match kind {
        Kind::Message(_) => &MESSAGE,
        Kind::ProgramStart(_) => &PROGRAM_START,
        Kind::SiteOpen(_) => &SITE_OPEN,
        Kind::Shutdown(_) => &POWER_SHUTDOWN,
        Kind::Restart(_) => &POWER_RESTART,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn feature_names_and_ids_are_unique() {
        let names: HashSet<_> = ALL_FEATURES.iter().map(|f| f.name).collect();
        let ids: HashSet<_> = ALL_FEATURES.iter().map(|f| f.id).collect();
        assert_eq!(names.len(), ALL_FEATURES.len());
        assert_eq!(ids.len(), ALL_FEATURES.len());
    }

    #[test]
    fn command_kinds_map_to_features() {
        use guardian_proto::v1::{ShutdownCmd, command::Kind};
        assert_eq!(
            feature_of(&Kind::Shutdown(ShutdownCmd {
                countdown_seconds: 60
            }))
            .name,
            "power.shutdown"
        );
        assert_eq!(
            by_name("files.push").map(|f| f.allowed_offline),
            Some(false)
        );
    }
}
