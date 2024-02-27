fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .type_attribute("Tag", "#[derive(serde::Deserialize, serde::Serialize)]")
        .compile(&["hooya.proto", "control.proto"], &["../../proto"])?;
    Ok(())
}
