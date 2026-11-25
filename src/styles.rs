//! The `CareerVector` styles and examples, handed on from `cvst`.
//!
//! `cvst` is the only home of the style trees: the style catalogue and contracts, the Typst
//! templates, the engine package and the showcase content. Products (CLI, TUI, web) take them from
//! here and never depend on `cvst` or carry a copy. Paths keep the style repository layout:
//! `outputs/<document>/<style>/...` and `.agent/typst/...`.
//!
//! Only text files are carried (TOML, Typst, Markdown, JSON, TXT). Fonts are not: they come from
//! `ctypst`'s `document-fonts`.
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

pub use cvst::{Style, catalogue, file, files, load_contract};

/// Directory of the engine Typst package inside the style tree.
pub const ENGINE_PACKAGE_DIR: &str = ".agent/typst";

/// Every style and example file as `(tree-relative path, contents)`, sorted by path.
#[must_use]
pub fn tree() -> &'static [(&'static str, &'static str)] {
    files()
}

/// The engine package files, keyed by their package-relative name: the `*.typ`, `*.toml` and
/// `*.json` files directly in [`ENGINE_PACKAGE_DIR`]. Documentation and fonts are data, not package.
pub fn engine_package() -> impl Iterator<Item = (&'static str, &'static str)> {
    files().iter().filter_map(|(path, text)| {
        let name = path.strip_prefix(ENGINE_PACKAGE_DIR)?.strip_prefix('/')?;
        let packaged = !name.contains('/')
            && [".typ", ".toml", ".json"]
                .iter()
                .any(|suffix| name.ends_with(suffix));
        packaged.then_some((name, *text))
    })
}

/// The engine package spec that styles import, such as `@local/ccvl:1.0.0`, from its `typst.toml`.
#[must_use]
pub fn engine_package_spec() -> Option<String> {
    let manifest = file(&format!("{ENGINE_PACKAGE_DIR}/typst.toml"))?;
    let field = |key: &str| {
        manifest.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
        })
    };
    Some(format!("@local/{}:{}", field("name")?, field("version")?))
}

/// Lowercase hex sha256 identity of the whole tree: over `<path> <sha256>\n` lines in path order.
/// Changes whenever a style or example file changes.
#[must_use]
pub fn fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    FINGERPRINT.get_or_init(|| {
        let mut lines: Vec<(&str, String)> = files()
            .iter()
            .map(|(path, text)| (*path, hex(&Sha256::digest(text.as_bytes()))))
            .collect();
        lines.sort();
        let mut hash = Sha256::new();
        for (path, digest) in &lines {
            hash.update(format!("{path} {digest}\n"));
        }
        hex(&hash.finalize())
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_carries_styles_and_the_engine_package() {
        assert!(catalogue().len() >= 8);
        assert!(tree().iter().any(|(p, _)| p.starts_with("outputs/")));
        let names: Vec<&str> = engine_package().map(|(name, _)| name).collect();
        assert!(names.contains(&"lib.typ") && names.contains(&"typst.toml"));
        assert!(names.contains(&"document-settings.json"));
        assert!(names.iter().all(|name| !name.contains('/')));
    }

    #[test]
    fn the_engine_package_spec_is_the_one_the_renderer_serves() {
        let spec = engine_package_spec().unwrap();
        #[cfg(feature = "render")]
        assert!(crate::render::is_engine_package(&spec));
        assert!(spec.starts_with("@local/ccvl:"), "{spec}");
    }

    #[test]
    fn the_fingerprint_is_a_sha256() {
        assert_eq!(fingerprint().len(), 64);
        assert!(fingerprint().bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
