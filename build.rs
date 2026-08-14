fn main() {
    // tonic 0.14 重构: configure()/compile() 从 tonic_build 移到 tonic-prost-build
    // (tonic_build 0.14 只保留 Service codegen, "Prost functionality has been moved
    //  to tonic-prost-build" — 见 tonic-build-0.14.6 lib.rs 顶部注释)。API 等价。
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["grpc/market.proto"], &["grpc/"])
        .expect("compile grpc/market.proto (合同唯一源, 不得修改)");
    println!("cargo:rerun-if-changed=grpc/market.proto");
}
