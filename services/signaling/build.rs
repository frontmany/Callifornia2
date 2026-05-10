fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);

    tonic_prost_build::configure().compile_protos(
        &[
            "../../common/proto/signaling.proto",
            "../../common/proto/signaling_admin.proto",
        ],
        &["../../common/proto"],
    )?;
    Ok(())
}
