fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/*");
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");
    let proto_files = ["proto/raft.proto", "proto/app_types.proto", "proto/app.proto"];

    // TODO: remove serde

    tonic_prost_build::configure()
        .btree_map(".")
        .type_attribute("openraftpb.NodeIdSet", "#[derive(Eq)]")
        .type_attribute("openraftpb.Membership", "#[derive(Eq)]")
        .type_attribute("openraftpb.Entry", "#[derive(Eq)]")
        .compile_with_config(config, &proto_files, &["proto"])?;
    Ok(())
}
