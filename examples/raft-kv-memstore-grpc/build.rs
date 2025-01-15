fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/*");
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");
    let proto_files = [
        "proto/internal_service.proto",
        "proto/management_service.proto",
        "proto/api_service.proto",
    ];

    // TODO: remove serde

    tonic_build::configure()
        .btree_map(["."])
        .type_attribute("openraftpb.Node", "#[derive(Eq, serde::Serialize, serde::Deserialize)]")
        .type_attribute(
            "openraftpb.SetRequest",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "openraftpb.Response",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "openraftpb.LeaderId",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute("openraftpb.Vote", "#[derive(Eq, serde::Serialize, serde::Deserialize)]")
        .type_attribute(
            "google.protobuf.Empty",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "openraftpb.NodeIdSet",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "openraftpb.Membership",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "openraftpb.Entry",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .compile_protos_with_config(config, &proto_files, &["proto"])?;
    Ok(())
}
