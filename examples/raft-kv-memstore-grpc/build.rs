fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/*");
    tonic_build::configure().compile_protos(
        &[
            "proto/internal_service.proto",
            "proto/management_service.proto",
            "proto/api_service.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
