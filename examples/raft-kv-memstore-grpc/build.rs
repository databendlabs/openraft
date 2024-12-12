fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/*");
    tonic_build::configure()
        .type_attribute("openraftpb.Node", "#[derive(Eq, serde::Serialize, serde::Deserialize)]")
        .compile_protos(
            &[
                "proto/internal_service.proto",
                "proto/management_service.proto",
                "proto/api_service.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
