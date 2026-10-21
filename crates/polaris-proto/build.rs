//! Generates the prost messages and tonic clients/servers from `proto/polaris/v1/*.proto`.
//!
//! The `protoc` binary and the well-known-type includes come from `protoc-bin-vendored`, so the build
//! needs neither a system `protoc` nor network access, and no environment variable is modified.
//! Output goes to `OUT_DIR`: `polaris.v1.rs` (all messages and services of the package) and
//! `polaris_v1_descriptor.bin` (the serialized `FileDescriptorSet`, exposed as
//! `polaris_proto::FILE_DESCRIPTOR_SET`).

use std::error::Error;
use std::path::PathBuf;

/// The proto files of package `polaris.v1`, relative to `proto/`.
const PROTO_FILES: [&str; 4] = [
    "polaris/v1/common.proto",
    "polaris/v1/kernel.proto",
    "polaris/v1/worker.proto",
    "polaris/v1/guest.proto",
];

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let proto_root = manifest_dir.join("proto");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=proto");

    let protos: Vec<PathBuf> = PROTO_FILES
        .iter()
        .map(|file| proto_root.join(file))
        .collect();
    let includes = vec![proto_root, protoc_bin_vendored::include_path()?];

    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("polaris_v1_descriptor.bin"))
        .compile_with_config(config, &protos, &includes)?;
    Ok(())
}
