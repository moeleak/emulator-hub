fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/emulator_controller.proto");
    println!("cargo:rerun-if-env-changed=PROTOC");
    let mut config = prost_build::Config::new();
    let protoc = std::env::var_os("PROTOC")
        .map(std::path::PathBuf::from)
        .map(Ok)
        .unwrap_or_else(protoc_bin_vendored::protoc_bin_path)?;
    config.protoc_executable(protoc);
    tonic_build::configure()
        .build_server(false)
        .compile_protos_with_config(
            config,
            &["proto/emulator_controller.proto"],
            &[
                "proto",
                protoc_bin_vendored::include_path()?
                    .to_str()
                    .ok_or("Invalid protoc path")?,
            ],
        )?;
    Ok(())
}
