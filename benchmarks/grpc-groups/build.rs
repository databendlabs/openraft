fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");

    // Use the vendored protoc binary so builds need no system protoc install.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    // SAFETY: a build script runs on one thread, so no other thread reads the environment.
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }

    // The Raft messages and their conversions come from the gRPC example: reuse its generated types
    // instead of generating a second copy of them.
    let example_protos = "../../examples/raft-kv-memstore-grpc/proto";
    tonic_prost_build::configure()
        .extern_path(".openraftpb", "::raft_kv_memstore_grpc::protobuf")
        .compile_protos(&["proto/grpc_groups.proto"], &["proto", example_protos])?;
    Ok(())
}
