use std::path::{Path, PathBuf};
use std::{env, fs};

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
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let proto_root = PathBuf::from(&manifest_dir).join("tests").join("proto");
    if !proto_root.exists() {
        return Ok(());
    }
    let proto_files = find_proto_files(&proto_root)?;
    if proto_files.is_empty() {
        return Ok(());
    }
    let fds = protox::compile(proto_files.iter(), [&proto_root])?;
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(PathBuf::from(env::var("OUT_DIR")?))
        .compile_fds(fds)?;
    println!("cargo:rerun-if-changed={}", proto_root.display());
    Ok(())
}
