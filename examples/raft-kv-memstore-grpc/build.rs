fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/*");
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");
    let proto_files = [
        "proto/internal_service.proto",
        "proto/management_service.proto",
        "proto/api_service.proto",
    ];
    tonic_build::configure()
        .type_attribute("openraftpb.Node", "#[derive(Eq, serde::Serialize, serde::Deserialize)]")
        .type_attribute(
            "openraftpb.SetRequest",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "openraftpb.Response",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .compile_protos_with_config(config, &proto_files, &["proto"])?;
    Ok(())
}
