//! Platform-independent identity of everything compiled into the ccvl runtime.
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;

/// Directory of the engine Typst package that the binary embeds.
pub const ENGINE_PACKAGE_DIR: &str = ".agent/typst";

pub fn inputs(root: &Path) -> io::Result<Vec<String>> {
    let mut sources = Vec::new();
    collect(root, &root.join(".agent/src"), &mut sources)?;
    collect(root, &root.join(".agent/core/src"), &mut sources)?;
    sources.extend(engine_package(root)?);
    sources.sort();
    let mut paths = [
        "Cargo.toml",
        ".agent/core/Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".agent/build.rs",
        "ccvl.json",
    ]
    .map(str::to_owned)
    .to_vec();
    paths.extend(sources);
    Ok(paths)
}

/// Workspace-relative paths of the engine package files, sorted: the `*.typ`,
/// `*.toml` and `*.json` files directly in [`ENGINE_PACKAGE_DIR`]. Fonts and
/// documentation stay data.
pub fn engine_package(root: &Path) -> io::Result<Vec<String>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(root.join(ENGINE_PACKAGE_DIR))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "package paths must be UTF-8")
        })?;
        let packaged = Path::new(name)
            .extension()
            .is_some_and(|extension| matches!(extension.to_str(), Some("typ" | "toml" | "json")));
        if packaged && entry.file_type()?.is_file() {
            paths.push(format!("{ENGINE_PACKAGE_DIR}/{name}"));
        }
    }
    paths.sort();
    Ok(paths)
}

fn collect(root: &Path, directory: &Path, paths: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(root, &path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(
                path.strip_prefix(root)
                    .expect("input is below root")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

pub fn fingerprint(root: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    for relative in inputs(root)? {
        let digest = Sha256::digest(fs::read(root.join(&relative))?);
        hash.update(format!("{relative} {digest:x}\n"));
    }
    Ok(format!("{:x}", hash.finalize()))
}
