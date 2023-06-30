fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("../../proto/hooya.proto")?;
    tonic_build::compile_protos("../../proto/control.proto")?;
    Ok(())
}
