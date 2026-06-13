use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn find_proto_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut proto_files = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                proto_files.extend(find_proto_files(&path)?);
            } else if path.extension().is_some_and(|ext| ext == "proto") {
                proto_files.push(path);
            }
        }
    }
    Ok(proto_files)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not exists");
    let workspace_dir = PathBuf::from(manifest_dir)
        .parent()
        .ok_or("manifest dir has no parent (expected src/proto -> src)")?
        .parent()
        .ok_or("manifest dir grandparent missing (expected src -> workspace root)")?
        .to_path_buf();

    let proto_root = workspace_dir.join("proto");
    let proto_files = find_proto_files(&proto_root)?;

    if proto_files.is_empty() {
        println!(
            "cargo:warning=No .proto files found in {}",
            proto_root.display()
        );
    }

    let file_descriptor_set = protox::compile(proto_files.iter(), [&proto_root])?;

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(file_descriptor_set)?;

    println!("cargo:rerun-if-changed={}", proto_root.display());
    Ok(())
}
