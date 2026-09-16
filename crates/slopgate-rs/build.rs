//! Reproducible source identity. No subprocesses, downloads, or timestamps.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
fn collect(root: &Path, path: &Path, files: &mut Vec<PathBuf>) {
    if !path.exists() {
        return;
    }
    let metadata = std::fs::symlink_metadata(path).expect("source metadata");
    assert!(
        !metadata.file_type().is_symlink(),
        "build identity does not follow source symlinks"
    );
    if metadata.is_file() {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if ["rs", "toml", "lock", "json", "yml", "yaml", "md"].contains(&extension) {
            files.push(
                path.strip_prefix(root)
                    .expect("source under workspace")
                    .to_path_buf(),
            );
        }
    } else if metadata.is_dir() {
        for entry in std::fs::read_dir(path).expect("source directory") {
            let entry = entry.expect("source entry");
            if !["target", "node_modules", ".git", ".worktrees"]
                .contains(&entry.file_name().to_string_lossy().as_ref())
            {
                collect(root, &entry.path(), files);
            }
        }
    }
}
fn main() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let inputs = [
        "Cargo.toml",
        "Cargo.lock",
        "crates",
        "tools",
        "rules",
        "docs/specs",
        "docs/architecture",
    ];
    let mut files = Vec::new();
    for input in inputs {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
        collect(root, &root.join(input), &mut files);
    }
    files.sort();
    files.dedup();
    let mut digest = Sha256::new();
    for file in files {
        digest.update(file.to_string_lossy().replace('\\', "/").as_bytes());
        digest.update([0]);
        digest.update(std::fs::read(root.join(file)).expect("source content"));
        digest.update([0]);
    }
    println!(
        "cargo:rustc-env=SLOPGATE_SOURCE_DIGEST={:x}",
        digest.finalize()
    );
    println!("cargo:rerun-if-env-changed=SLOPGATE_BUILD_REVISION");
    let revision = std::env::var("SLOPGATE_BUILD_REVISION")
        .unwrap_or_else(|_| "unrecorded-development-build".into());
    assert!(
        revision.len() <= 128
            && revision
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "invalid build revision"
    );
    println!("cargo:rustc-env=SLOPGATE_BUILD_REVISION={revision}");
}
