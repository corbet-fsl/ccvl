#[path = "src/runtime_source.rs"]
mod runtime_source;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=.agent/src");
    println!(
        "cargo:rerun-if-changed={}",
        runtime_source::ENGINE_PACKAGE_DIR
    );
    for path in runtime_source::inputs(&root).expect("runtime inputs must exist") {
        println!("cargo:rerun-if-changed={path}");
    }
    let identity = runtime_source::fingerprint(&root).expect("runtime inputs must be readable");
    println!("cargo:rustc-env=CCVL_RUNTIME_ID={identity}");
    embed_engine_package(&root);
    embed_layout(&root);
}

/// Generate `engine_package.rs`: the package spec from its `typst.toml` and
/// every package file through `include_bytes!`, so the binary never reads the
/// engine Typst from a checkout.
fn embed_engine_package(root: &Path) {
    let directory = runtime_source::ENGINE_PACKAGE_DIR;
    let manifest = std::fs::read_to_string(root.join(directory).join("typst.toml"))
        .expect("the engine package needs a typst.toml");
    let field = |key: &str| {
        manifest
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| panic!("the engine package typst.toml needs {key}"))
    };
    let spec = format!("@local/{}:{}", field("name"), field("version"));
    let mut source = format!(
        "/// The engine package spec that styles import.\n\
         pub const SPEC: &str = {spec:?};\n\
         /// Package-relative paths and bytes of the embedded engine package.\n\
         pub const FILES: &[(&str, &[u8])] = &[\n"
    );
    for relative in runtime_source::engine_package(root).expect("engine package must be readable") {
        let name = relative
            .strip_prefix(&format!("{directory}/"))
            .expect("package file is inside its directory");
        let path = root.join(&relative).display().to_string();
        writeln!(source, "    ({name:?}, include_bytes!({path:?})),").unwrap();
    }
    source.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("engine_package.rs"), source).unwrap();
}

/// Generate `layout.rs`: every path of the `workspace_groups` table in
/// `ccvl.json` as a constant named `<GROUP>_<KEY>`, each group's entries as
/// `<GROUP>_GROUP`, plus the table itself, so
/// the manifest is the single source of truth for workspace paths and the
/// binary never has to read it to find them.
fn embed_layout(root: &Path) {
    let text = std::fs::read_to_string(root.join("ccvl.json")).expect("ccvl.json must exist");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("ccvl.json must be JSON");
    let groups = manifest
        .get("workspace_groups")
        .and_then(serde_json::Value::as_object)
        .expect("ccvl.json needs a workspace_groups table");
    let mut source = String::new();
    for (group, entries) in groups {
        let entries = entries
            .as_object()
            .unwrap_or_else(|| panic!("workspace_groups.{group} must be a table"));
        let mut table = String::new();
        for (key, value) in entries {
            let value = value
                .as_str()
                .unwrap_or_else(|| panic!("workspace_groups.{group}.{key} must be a string"));
            let name = format!("{group}_{key}").to_uppercase().replace('-', "_");
            writeln!(source, "pub const {name}: &str = {value:?};").unwrap();
            write!(table, "({key:?}, {name}), ").unwrap();
        }
        let name = format!("{group}_group").to_uppercase().replace('-', "_");
        writeln!(
            source,
            "/// Every `(key, path)` entry of `workspace_groups.{group}`.\n\
             pub const {name}: &[(&str, &str)] = &[{table}];"
        )
        .unwrap();
    }
    let table = serde_json::to_string(groups).unwrap();
    writeln!(
        source,
        "/// The `workspace_groups` table this binary was built from."
    )
    .unwrap();
    writeln!(source, "pub const GROUPS_JSON: &str = {table:?};").unwrap();
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("layout.rs"), source).unwrap();
}
