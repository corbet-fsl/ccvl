use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The bundle schema this engine writes.
pub const BUNDLE_SCHEMA: u32 = 2;

/// The first bundle schema, still read: its Typst helpers travel in `files` under
/// `.agent/typst/` and it has no `packages`. See [`StyleBundle::upgraded`].
pub const BUNDLE_SCHEMA_V1: u32 = 1;

/// Project path where schema 1 styles import the generated correspondence module.
pub const LEGACY_CORRESPONDENCE_PATH: &str = ".agent/typst/correspondence.typ";

/// Namespace and name of the ccvl engine Typst package. Styles import it as
/// `@local/ccvl:<version>`; its host supplies the files.
pub const ENGINE_PACKAGE: &str = "@local/ccvl";

/// Typst packages by `@namespace/name:version` spec, each with its
/// package-relative paths and bytes.
pub type Packages = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

const INPUT_PREFIX: &str = "__ccvl_inputs/";

/// One selected style/substyle/locale/page/paper variant, with exact upstream assets.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StyleBundle {
    pub schema_version: u32,
    pub engine_api: u32,
    pub id: String,
    pub version: String,
    pub document: String,
    pub style: String,
    pub substyle: String,
    pub locale: String,
    pub pages: usize,
    pub entry: String,
    pub inputs: BTreeMap<String, String>,
    pub contract: Value,
    pub scaffold: Value,
    pub fonts: Vec<String>,
    pub files: BTreeMap<String, Vec<u8>>,
    /// The engine Typst package by spec (`@local/ccvl:<version>`), with its
    /// package-relative files. The renderer adds the correspondence module.
    /// Absent in schema 1.
    #[serde(default)]
    pub packages: Packages,
}

/// User data is separate from the reusable bundle. Both file and API adapters
/// can supply this value without a filesystem-shaped core API.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StyleDocument {
    pub bundle: Arc<StyleBundle>,
    pub bundle_sha256: String,
    pub record: Value,
    pub profile: Value,
}

impl StyleBundle {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.schema_version, BUNDLE_SCHEMA_V1 | BUNDLE_SCHEMA) && self.engine_api == 1,
            "unsupported CCVL style bundle or engine API"
        );
        ensure!(
            self.schema_version != BUNDLE_SCHEMA_V1 || self.packages.is_empty(),
            "a schema 1 style bundle carries no packages"
        );
        ensure!(
            !self.id.is_empty() && !self.version.is_empty(),
            "style identity and version are required"
        );
        ensure!(
            matches!(self.document.as_str(), "cv" | "cl"),
            "unsupported document type"
        );
        ensure!(
            crate::normalize_locale(&self.locale)? == self.locale,
            "bundle locale must be canonical"
        );
        ensure!(self.pages > 0, "bundle must declare its page count");
        ensure!(!self.files.is_empty(), "style bundle has no assets");
        let mut total = 0_usize;
        for (path, bytes) in &self.files {
            validate_path(path)?;
            ensure!(
                !path.starts_with(INPUT_PREFIX),
                "style asset shadows a document input"
            );
            total = total
                .checked_add(bytes.len())
                .context("style asset size overflow")?;
        }
        for (spec, files) in &self.packages {
            ensure!(
                is_engine_package(spec),
                "a style bundle can carry only the ccvl engine package, not {spec}"
            );
            ensure!(
                files.contains_key("typst.toml"),
                "bundled package {spec} has no typst.toml"
            );
            for (path, bytes) in files {
                validate_path(path)?;
                total = total
                    .checked_add(bytes.len())
                    .context("style asset size overflow")?;
            }
        }
        ensure!(total <= 64 * 1024 * 1024, "style bundle exceeds 64 MiB");
        validate_path(&self.entry)?;
        ensure!(
            self.files.contains_key(&self.entry),
            "style entry point is missing"
        );
        for font in &self.fonts {
            ensure!(
                self.files.contains_key(font),
                "declared style font is missing: {font}"
            );
        }
        ensure!(
            self.inputs.get("locale") == Some(&self.locale),
            "bundle locale input differs"
        );
        ensure!(
            self.inputs.get("pages") == Some(&self.pages.to_string()),
            "bundle page input differs"
        );
        for name in ["application", "profile"] {
            ensure!(
                !self.inputs.contains_key(name),
                "user input {name} must not be bundled"
            );
        }
        for name in [
            "strings",
            "substyle",
            "shared-defaults",
            "layout",
            "contract",
        ] {
            if let Some(path) = self.inputs.get(name) {
                ensure!(
                    self.files.contains_key(path.trim_start_matches('/')),
                    "missing style input {name}: {path}"
                );
            }
        }
        ensure!(
            self.scaffold.is_object(),
            "style scaffold must be an object"
        );
        crate::contract::validate_contract(&self.contract)
    }

    /// The bundle's identity. A schema 1 bundle hashes as it always did, without the `packages`
    /// field it never had, so a stored document keeps verifying after the upgrade of the engine.
    pub fn sha256(&self) -> Result<String> {
        let bytes = if self.schema_version == BUNDLE_SCHEMA_V1 {
            serde_json::to_vec(&LegacyBundle::from(self))?
        } else {
            serde_json::to_vec(self)?
        };
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    /// The same bundle in the current schema. A schema 1 bundle becomes schema 2 with its
    /// files unchanged, plus the generated correspondence module at the path its styles import
    /// ([`LEGACY_CORRESPONDENCE_PATH`]), the module the v1 renderer registered from the linked
    /// cletter: nothing the styles say changes, so the rendered bytes do not either. The upgrade
    /// changes the identity ([`StyleBundle::sha256`]); upgrade a stored document with
    /// [`StyleDocument::upgraded`]. A schema 2 bundle is returned as it is.
    #[cfg(feature = "render")]
    pub fn upgraded(&self) -> Result<Self> {
        let mut bundle = self.clone();
        if bundle.schema_version == BUNDLE_SCHEMA_V1 {
            ensure!(
                !bundle.files.contains_key(LEGACY_CORRESPONDENCE_PATH),
                "a schema 1 bundle must not bundle the generated correspondence module"
            );
            bundle.files.insert(
                LEGACY_CORRESPONDENCE_PATH.to_owned(),
                crate::render::correspondence_module().into_bytes(),
            );
            bundle.schema_version = BUNDLE_SCHEMA;
        }
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn with_record(self, record: Value, profile: Value) -> Result<StyleDocument> {
        self.validate()?;
        let document = StyleDocument {
            bundle_sha256: self.sha256()?,
            bundle: Arc::new(self),
            record,
            profile,
        };
        document.validate()?;
        Ok(document)
    }
}

/// The serialized form of a schema 1 bundle: today's fields without `packages`, in order.
#[derive(Serialize)]
struct LegacyBundle<'a> {
    schema_version: u32,
    engine_api: u32,
    id: &'a str,
    version: &'a str,
    document: &'a str,
    style: &'a str,
    substyle: &'a str,
    locale: &'a str,
    pages: usize,
    entry: &'a str,
    inputs: &'a BTreeMap<String, String>,
    contract: &'a Value,
    scaffold: &'a Value,
    fonts: &'a [String],
    files: &'a BTreeMap<String, Vec<u8>>,
}

impl<'a> From<&'a StyleBundle> for LegacyBundle<'a> {
    fn from(b: &'a StyleBundle) -> Self {
        Self {
            schema_version: b.schema_version,
            engine_api: b.engine_api,
            id: &b.id,
            version: &b.version,
            document: &b.document,
            style: &b.style,
            substyle: &b.substyle,
            locale: &b.locale,
            pages: b.pages,
            entry: &b.entry,
            inputs: &b.inputs,
            contract: &b.contract,
            scaffold: &b.scaffold,
            fonts: &b.fonts,
            files: &b.files,
        }
    }
}

impl StyleDocument {
    /// The same document on an upgraded bundle (see [`StyleBundle::upgraded`]), verified first and
    /// re-bound to the new bundle identity. A schema 2 document is returned as it is. Hosts call
    /// this on load, so what they save is schema 2.
    #[cfg(feature = "render")]
    pub fn upgraded(&self) -> Result<Self> {
        self.validate()?;
        if self.bundle.schema_version != BUNDLE_SCHEMA_V1 {
            return Ok(self.clone());
        }
        let bundle = self.bundle.upgraded()?;
        let document = Self {
            bundle_sha256: bundle.sha256()?,
            bundle: Arc::new(bundle),
            record: self.record.clone(),
            profile: self.profile.clone(),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<()> {
        self.bundle.validate()?;
        ensure!(
            self.bundle.sha256()? == self.bundle_sha256,
            "style bundle changed; select its new version explicitly"
        );
        ensure!(
            self.record.is_object() && self.profile.is_object(),
            "document and profile must be objects"
        );
        ensure!(
            self.record.get("wording").is_none(),
            "consumer records must contain resolved wording"
        );
        let options = self
            .record
            .get("options")
            .context("document options are missing")?;
        ensure!(
            options.get("language").and_then(Value::as_str) == Some(&self.bundle.locale),
            "document locale differs from selected bundle"
        );
        for (suffix, expected) in [
            ("style", &self.bundle.style),
            ("substyle", &self.bundle.substyle),
        ] {
            ensure!(
                options
                    .get(format!("{}_{}", self.bundle.document, suffix))
                    .and_then(Value::as_str)
                    == Some(expected),
                "document {suffix} differs from selected bundle"
            );
        }
        let page_key = if self.bundle.document == "cv" {
            "pages"
        } else {
            "cl_pages"
        };
        if let Some(pages) = options.get(page_key) {
            ensure!(
                pages.as_u64() == Some(self.bundle.pages as u64),
                "document pages differ from selected bundle"
            );
        }
        if let Some(paper) = options.get(format!("{}_paper", self.bundle.document)) {
            ensure!(
                paper.as_str() == self.bundle.inputs.get("paper").map(String::as_str),
                "document paper differs from selected bundle"
            );
        }
        let content = self
            .record
            .get(&self.bundle.document)
            .and_then(Value::as_object)
            .context("selected document content is missing")?;
        if let Some(fields) = self
            .bundle
            .contract
            .get("content_fields")
            .and_then(Value::as_array)
        {
            for key in content.keys() {
                ensure!(
                    fields.iter().any(|field| field.as_str() == Some(key)),
                    "unknown style content field: {key}"
                );
            }
        }
        Ok(())
    }
}

/// Whether `spec` names a version of the engine package.
#[must_use]
pub fn is_engine_package(spec: &str) -> bool {
    spec.strip_prefix(ENGINE_PACKAGE)
        .and_then(|rest| rest.strip_prefix(':'))
        .is_some_and(|version| !version.is_empty())
}

pub fn validate_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.contains(['\\', ':', '\0'])
            && path.split('/').all(|part| !matches!(part, "" | "." | "..")),
        "invalid virtual asset path: {path:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_cannot_escape_or_alias() {
        for path in [
            "../data", "/data", "a/../b", "a//b", "C:/data", "a\\b", "./a",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        validate_path("outputs/cvl/cl/example/layout.typ").unwrap();
    }
}
