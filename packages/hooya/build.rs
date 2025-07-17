fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .type_attribute(
            "Tag",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "File",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "Image",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "Video",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "Thumbnail",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "TagSuggestion",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "SuggestTagReply",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "TagQuery",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .enum_attribute(
            "ext_file",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .enum_attribute("ext_file", "#[serde(untagged)]")
        .compile(
            &["hooya.proto", "control.proto", "mesh.proto"],
            &["../../proto"],
        )?;
    Ok(())
}
