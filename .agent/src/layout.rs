//! Workspace paths, generated at build time from the `workspace_groups`
//! table of `ccvl.json` (see `.agent/build.rs`). Code names a path by its
//! role, never by a literal, so moving a folder is a manifest edit.
use std::collections::BTreeSet;

include!(concat!(env!("OUT_DIR"), "/layout.rs"));

/// Every workspace data path of the manifest.
pub const DATA_PATHS: [&str; 18] = [
    INPUTS_ROOT,
    INPUTS_LIBRARY,
    OUTPUTS_ROOT,
    INPUTS_PROFILE,
    INPUTS_INTERVIEWS,
    INPUTS_STATIONS,
    INPUTS_ASSETS,
    INPUTS_SIGNATURE,
    OUTPUTS_CVL,
    OUTPUTS_CV,
    OUTPUTS_CL,
    OUTPUTS_SHARED,
    OUTPUTS_FLYERS,
    OUTPUTS_WEB,
    OUTPUTS_VIDEOS,
    OUTPUTS_PRESENTATIONS,
    OUTPUTS_PORTFOLIOS,
    OPPORTUNITIES_ROOT,
];

/// The top-level directories that hold workspace data, derived once from
/// [`DATA_PATHS`].
#[must_use]
pub fn data_roots() -> BTreeSet<&'static str> {
    DATA_PATHS
        .into_iter()
        .filter_map(|path| path.split('/').next())
        .collect()
}

/// `<directory>/README.md` for a manifest directory.
#[must_use]
pub fn readme(directory: &str) -> String {
    format!("{directory}/README.md")
}

/// Roots of `paths` with every path nested inside another one dropped, so a
/// walk over the result visits each file once.
#[must_use]
pub fn outermost<'a>(paths: &[&'a str]) -> Vec<&'a str> {
    let mut roots: Vec<&str> = paths.to_vec();
    roots.sort_unstable();
    roots.dedup();
    let mut kept: Vec<&str> = Vec::with_capacity(roots.len());
    for root in roots {
        if !kept.iter().any(|outer| {
            root.strip_prefix(outer)
                .is_some_and(|rest| rest.starts_with('/'))
        }) {
            kept.push(root);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn layout_matches_the_manifest() {
        let manifest: Value = serde_json::from_str(include_str!("../../ccvl.json")).unwrap();
        let built: Value = serde_json::from_str(GROUPS_JSON).unwrap();
        assert_eq!(manifest["workspace_groups"], built);
    }

    #[test]
    fn nested_roots_are_walked_once() {
        assert_eq!(
            outermost(&[
                "outputs/cvl",
                "outputs/cvl/assets",
                ".agent/typst",
                "outputs/cvl"
            ]),
            [".agent/typst", "outputs/cvl"]
        );
        assert_eq!(
            outermost(&["inputs/assets", "outputs/cvl"]),
            ["inputs/assets", "outputs/cvl"]
        );
    }
}
