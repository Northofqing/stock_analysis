//! Generate independent LocalBridgeV1 and ExternalV1 wire contracts.
//! Local is frozen with its private 61/62 extensions; External uses the public
//! client-bundle proto verbatim. Their identical package names have separate
//! generated modules and descriptors.
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "src/production_root.rs"]
mod production_root;

fn main() {
    println!("cargo:rerun-if-env-changed=STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT");
    println!("cargo:rerun-if-changed=src/production_root.rs");
    let configured = match std::env::var("STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => panic!("invalid STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT: {error}"),
    };
    production_root::select_build_root(env!("CARGO_MANIFEST_DIR"), configured.as_deref())
        .unwrap_or_else(|error| panic!("invalid STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT: {error}"));

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let local_source = "contracts/local_bridge_v1/market.proto";
    let local_dir = out_dir.join("local_bridge_v1");
    std::fs::create_dir_all(&local_dir).expect("create LocalBridgeV1 output directory");
    // Keep the historical descriptor file.name as well as the exact frozen bytes.
    let local_input = local_dir.join("market_local.proto");
    std::fs::copy(local_source, &local_input).expect("copy frozen LocalBridgeV1 proto");
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&local_dir)
        .file_descriptor_set_path(local_dir.join("descriptor.bin"))
        .compile_protos(&[local_input.as_path()], &[local_dir.as_path()])
        .expect("compile frozen LocalBridgeV1 contract");

    let external_source = "client-bundle/market.proto";
    let external_dir = out_dir.join("external_v1");
    std::fs::create_dir_all(&external_dir).expect("create ExternalV1 output directory");
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&external_dir)
        .file_descriptor_set_path(external_dir.join("descriptor.bin"))
        .compile_protos(&[Path::new(external_source)], &[Path::new("client-bundle")])
        .expect("compile unmodified ExternalV1 contract");

    println!("cargo:rerun-if-changed={local_source}");
    println!("cargo:rerun-if-changed={external_source}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROTOC");
    println!("cargo:rerun-if-env-changed=PROTOC_INCLUDE");
}
