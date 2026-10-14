fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }
    tonic_prost_build::configure()
        .type_attribute(
            "AttemptFailure",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/lankide.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto/lankide.proto");
    Ok(())
}
