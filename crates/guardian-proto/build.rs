//! Compiles `proto/` with protox (no `protoc` needed) and generates prost + tonic code.

use prost::Message;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../../proto");
    let fds = protox::compile(
        [
            "guardian/v1/common.proto",
            "guardian/v1/agent.proto",
            "guardian/v1/api.proto",
            "guardian/v1/local.proto",
            "guardian/v1/worker.proto",
        ],
        ["../../proto"],
    )?;
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    std::fs::write(out.join("guardian_descriptor.bin"), fds.encode_to_vec())?;
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(fds)?;
    Ok(())
}
