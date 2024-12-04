fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile_protos(
        &[
            "protos/common.proto",
            "protos/management_service.proto",
            "protos/internal_service.proto",
        ],
        &["protos"],
    )?;
    Ok(())
}
