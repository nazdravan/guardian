//! Generated protobuf types and gRPC stubs for Guardian 3.0.

#![forbid(unsafe_code)]

/// Package `guardian.v1`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod v1 {
    tonic::include_proto!("guardian.v1");
}

/// The encoded `FileDescriptorSet` of all Guardian protos (for gRPC reflection).
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/guardian_descriptor.bin"));
