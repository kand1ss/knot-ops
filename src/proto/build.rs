use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not exists");
    println!("{}", manifest_dir);
    let workspace_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let proto_root = std::path::Path::new(&workspace_dir)
        .join("proto")
        .join("v1");

    let file_descriptor_set = protox::compile(["daemon.proto"], [&proto_root])?;

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(file_descriptor_set)?;

    println!("cargo:rerun-if-changed={}", proto_root.display());

    Ok(())
}
