//! The engine Typst package `@local/ccvl`, embedded at build time from
//! `.agent/typst` (see its README). Every engine serves it from memory, so
//! styles import it without a workspace path, and bundles carry its files.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::Result;
use ccvl_core::render::Packages;
use ctypst::EngineBuilder;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/engine_package.rs"));
}

/// The `@local/ccvl:<version>` spec that styles import, from `typst.toml`.
pub use embedded::SPEC;

/// The embedded engine package, keyed by its spec.
pub fn packages() -> &'static Packages {
    static PACKAGES: OnceLock<Packages> = OnceLock::new();
    PACKAGES.get_or_init(|| {
        let files = embedded::FILES
            .iter()
            .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
            .collect();
        BTreeMap::from([(SPEC.to_owned(), files)])
    })
}

/// Register the engine package and the correspondence libraries.
pub fn with_packages(builder: EngineBuilder) -> Result<EngineBuilder> {
    ccvl_core::render::with_packages(builder, packages())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_package_matches_the_engine_sources() {
        let root = crate::test_support::repository();
        let files = &packages()[SPEC];
        let expected = crate::runtime_source::engine_package(root).unwrap();
        assert_eq!(
            files
                .keys()
                .map(|name| format!("{}/{name}", crate::runtime_source::ENGINE_PACKAGE_DIR))
                .collect::<Vec<_>>(),
            expected
        );
        for relative in expected {
            let name = relative.rsplit('/').next().unwrap();
            assert_eq!(files[name], std::fs::read(root.join(&relative)).unwrap());
        }
        assert!(ccvl_core::render::is_engine_package(SPEC));
    }
}
