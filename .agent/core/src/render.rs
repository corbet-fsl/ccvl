use anyhow::{Context, Result, ensure};
use ctypst::{CompileRequest, Document, Engine, EngineBuilder, PageConstraint};

use crate::{StyleBundle, StyleDocument};

/// Project path of the generated correspondence module. Styles import the
/// cletter facade from here, `#import "/.agent/typst/correspondence.typ": ...`,
/// so no Typst source names a cletter version.
pub const CORRESPONDENCE_MODULE: &str = ".agent/typst/correspondence.typ";

/// Source of [`CORRESPONDENCE_MODULE`]: a glob import of the linked cletter
/// package. Typst exports every binding of a module, imported ones included,
/// so importers see the complete cletter facade.
#[must_use]
pub fn correspondence_module() -> String {
    format!("#import \"{}\": *\n", cletter::typst::spec())
}

/// Register the correspondence family with an engine: the cletter, cnice,
/// cdate and cink Typst packages embedded in the linked crates, plus the
/// [`CORRESPONDENCE_MODULE`] that re-exports cletter at its linked version.
///
/// Cargo selects the newest compatible library releases (see
/// `cletter::typst::packages`), and the Typst code always comes from exactly
/// those crates; nothing is downloaded or copied into the project.
pub fn with_correspondence(builder: EngineBuilder) -> Result<EngineBuilder> {
    let mut builder = builder
        .source(CORRESPONDENCE_MODULE, correspondence_module())
        .context("cannot register the correspondence module")?;
    for package in cletter::typst::packages() {
        builder = builder
            .package(&package.spec, package.files)
            .with_context(|| format!("cannot register Typst package {}", package.spec))?;
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
        let mut builder = with_correspondence(Engine::builder().fonts(ctypst::fonts::documents()))?;
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

    #[test]
    fn correspondence_module_reexports_the_linked_cletter_family() {
        assert_eq!(
            correspondence_module(),
            format!(
                "#import \"@preview/cletter:{}\": *\n",
                cletter::typst::VERSION
            )
        );
        let source = "#import \"/.agent/typst/correspondence.typ\": closing, long-date, opening, salutation, signature-image\n\
            #set text(font: \"Archivo\")\n\
            #metadata((\n\
              opening(\"de-ch\", name: \"Frau Dr. Müller\"),\n\
              salutation(\"en-ch\", \"Ms Smith\"),\n\
              closing(\"de-ch\"),\n\
              long-date(\"de-ch\", 2026, 9, 7),\n\
            )) <probe>\n\
            #signature-image(bytes(\"<svg xmlns='http://www.w3.org/2000/svg' width='80' height='20'><path d='M0 10L80 10' stroke='black'/></svg>\"), 20)\n";
        let engine = with_correspondence(Engine::builder().fonts(ctypst::fonts::documents()))
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
}
