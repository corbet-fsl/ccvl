use anyhow::{Context, Result, ensure};
use ctypst::{CompileRequest, Document, Engine, EngineBuilder, PageConstraint};

pub use crate::style::{ENGINE_PACKAGE, Packages, is_engine_package};
use crate::{StyleBundle, StyleDocument};

/// Package-relative path of the generated correspondence module inside the
/// engine package. Styles import the cletter facade through it,
/// `#import correspondence: opening, ...`, so no Typst source names a cletter
/// version.
pub const CORRESPONDENCE_MODULE: &str = "correspondence.typ";

/// Source of [`CORRESPONDENCE_MODULE`]: a glob import of the linked cletter
/// package. Typst exports every binding of a module, imported ones included,
/// so importers see the complete cletter facade.
#[must_use]
pub fn correspondence_module() -> String {
    format!("#import \"{}\": *\n", cletter::typst::spec())
}

/// Register every Typst package a ccvl engine serves: the supplied engine
/// package, extended with the generated [`CORRESPONDENCE_MODULE`], and the
/// cletter, cnice, cdate and cink packages embedded in the linked crates.
///
/// Cargo selects the newest compatible library releases (see
/// `cletter::typst::packages`), and the Typst code always comes from exactly
/// those crates. Everything is served from memory; nothing is downloaded,
/// installed or read from a project directory.
pub fn with_packages(builder: EngineBuilder, packages: &Packages) -> Result<EngineBuilder> {
    let mut builder = builder;
    for package in cletter::typst::packages() {
        builder = builder
            .package(&package.spec, package.files)
            .with_context(|| format!("cannot register Typst package {}", package.spec))?;
    }
    for (spec, files) in packages {
        ensure!(
            is_engine_package(spec),
            "only the ccvl engine package can be supplied, not {spec}"
        );
        ensure!(
            !files.contains_key(CORRESPONDENCE_MODULE),
            "{spec} must not supply the generated {CORRESPONDENCE_MODULE}"
        );
        let files = files
            .iter()
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .chain([(
                CORRESPONDENCE_MODULE.to_owned(),
                correspondence_module().into_bytes(),
            )]);
        builder = builder
            .package(spec, files)
            .with_context(|| format!("cannot register Typst package {spec}"))?;
    }
    Ok(builder)
}

/// An in-memory renderer: no project root, database, shell, or host file access.
pub struct StyleRenderer {
    engine: Engine,
    bundle_sha256: String,
}

impl StyleRenderer {
    pub fn new(bundle: &StyleBundle) -> Result<Self> {
        bundle.validate()?;
        let mut builder = with_packages(
            Engine::builder().fonts(ctypst::fonts::documents()),
            &bundle.packages,
        )?;
        for (path, bytes) in &bundle.files {
            builder = if std::path::Path::new(path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("typ"))
            {
                builder.source(
                    path,
                    std::str::from_utf8(bytes).context("Typst source must be UTF-8")?,
                )?
            } else {
                builder.binary(path, bytes.clone())?
            };
        }
        for path in &bundle.fonts {
            builder = builder.fonts([bundle.files[path].clone()]);
        }
        Ok(Self {
            bundle_sha256: bundle.sha256()?,
            engine: builder
                .build()
                .context("cannot initialize bundled CCVL style")?,
        })
    }

    pub fn compile(&self, document: &StyleDocument) -> Result<Document> {
        document.validate()?;
        ensure!(
            document.bundle_sha256 == self.bundle_sha256,
            "renderer belongs to a different style bundle"
        );
        let mut inputs = document.bundle.inputs.clone();
        inputs.insert(
            "application".into(),
            "/__ccvl_inputs/application.toml".into(),
        );
        inputs.insert("profile".into(), "/__ccvl_inputs/profile.toml".into());
        Ok(self
            .engine
            .compile(
                CompileRequest::new(&document.bundle.entry)
                    .inputs(inputs)
                    .pages(PageConstraint::Exactly(document.bundle.pages))
                    .binary_file(
                        "__ccvl_inputs/application.toml",
                        toml::to_string(&document.record)?.into_bytes(),
                    )
                    .binary_file(
                        "__ccvl_inputs/profile.toml",
                        toml::to_string(&document.profile)?.into_bytes(),
                    ),
            )
            .context("CCVL style compilation failed")?
            .document)
    }

    pub fn pdf(&self, document: &Document) -> Result<Vec<u8>> {
        crate::pdf::lowercase_locales(
            &self
                .engine
                .pdf(document, 0)
                .context("CCVL PDF export failed")?,
        )
    }

    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn correspondence_module_reexports_the_linked_cletter_family() {
        assert_eq!(
            correspondence_module(),
            format!(
                "#import \"@preview/cletter:{}\": *\n",
                cletter::typst::VERSION
            )
        );
        let source = "#import \"@local/ccvl:1.0.0\": correspondence\n\
            #import correspondence: closing, long-date, opening, salutation, signature-image\n\
            #set text(font: \"Archivo\")\n\
            #metadata((\n\
              opening(\"de-ch\", name: \"Frau Dr. Müller\"),\n\
              salutation(\"en-ch\", \"Ms Smith\"),\n\
              closing(\"de-ch\"),\n\
              long-date(\"de-ch\", 2026, 9, 7),\n\
            )) <probe>\n\
            #signature-image(bytes(\"<svg xmlns='http://www.w3.org/2000/svg' width='80' height='20'><path d='M0 10L80 10' stroke='black'/></svg>\"), 20)\n";
        // A minimal engine package: the generated module is its only content.
        let packages = Packages::from([(
            "@local/ccvl:1.0.0".to_owned(),
            BTreeMap::from([
                (
                    "typst.toml".to_owned(),
                    b"[package]\nname = \"ccvl\"\nversion = \"1.0.0\"\nentrypoint = \"lib.typ\"\n"
                        .to_vec(),
                ),
                (
                    "lib.typ".to_owned(),
                    b"#import \"correspondence.typ\"\n".to_vec(),
                ),
            ]),
        )]);
        let engine = with_packages(
            Engine::builder().fonts(ctypst::fonts::documents()),
            &packages,
        )
        .unwrap()
        .source("main.typ", source)
        .unwrap()
        .build()
        .unwrap();
        let output = engine.compile(CompileRequest::new("main.typ")).unwrap();
        assert_eq!(
            ctypst::query_json(&output.document, "probe").unwrap(),
            [serde_json::json!([
                "Sehr geehrte Frau Dr. Müller",
                cletter::salutation("en-ch", "Ms Smith"),
                "Freundliche Grüsse",
                "7. September 2026",
            ])]
        );
    }

    #[test]
    fn only_the_engine_package_can_be_supplied() {
        assert!(is_engine_package("@local/ccvl:1.0.0"));
        assert!(!is_engine_package("@local/ccvl"));
        assert!(!is_engine_package("@local/ccvl-extra:1.0.0"));
        let other = Packages::from([("@preview/other:1.0.0".to_owned(), BTreeMap::new())]);
        assert!(with_packages(Engine::builder(), &other).is_err());
        let shadowing = Packages::from([(
            "@local/ccvl:1.0.0".to_owned(),
            BTreeMap::from([(CORRESPONDENCE_MODULE.to_owned(), Vec::new())]),
        )]);
        assert!(with_packages(Engine::builder(), &shadowing).is_err());
    }
}
