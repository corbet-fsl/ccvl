//! Schema 1 style bundles: the bundle `ccvl export-style` wrote before the engine Typst became a
//! package. The fixture is a real export (cover letter, harvard/frame, en-ch; the font bytes are
//! emptied because the engine supplies its document fonts, and a signature image is added).
//! The expected hashes come from the schema 1 engine: the bundle identity from its
//! `StyleBundle::sha256`, the PDF from its `render-style` on the same record and profile.
#![cfg(feature = "render")]

use std::io::Read;

use ccvl::render::StyleRenderer;
use ccvl::style::{BUNDLE_SCHEMA, BUNDLE_SCHEMA_V1};
use ccvl::{StyleBundle, StyleDocument};
use serde_json::Value;
use sha2::{Digest, Sha256};

const V1_BUNDLE_SHA256: &str = "921e90e255a69032984d255f216c19b1d4fe0cbac53c0fdb3b00982ac105a2ed";
const V1_PDF_SHA256: &str = "cdf63e869bf063a2cd9ef08dfe01d96ad8220917b2e8ba2e4497cb21f17f58d7";

fn fixture_bundle() -> StyleBundle {
    let gz = include_bytes!("fixtures/v1/cl-harvard-en-ch.bundle.json.gz");
    let mut json = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut json)
        .expect("gunzip");
    serde_json::from_slice(&json).expect("a schema 1 bundle loads")
}

fn toml_value(text: &str) -> Value {
    serde_json::to_value(text.parse::<toml::Table>().expect("toml")).expect("json")
}

fn document(bundle: StyleBundle) -> StyleDocument {
    bundle
        .with_record(
            toml_value(include_str!("fixtures/v1/application.toml")),
            toml_value(include_str!("fixtures/v1/profile.toml")),
        )
        .expect("document")
}

fn pdf_sha256(document: &StyleDocument) -> String {
    let renderer = StyleRenderer::new(&document.bundle).expect("renderer");
    let compiled = renderer.compile(document).expect("compile");
    format!(
        "{:x}",
        Sha256::digest(renderer.pdf(&compiled).expect("pdf"))
    )
}

#[test]
fn a_schema_1_bundle_loads_and_keeps_the_identity_the_old_engine_gave_it() {
    let bundle = fixture_bundle();
    assert_eq!(bundle.schema_version, BUNDLE_SCHEMA_V1);
    assert!(bundle.packages.is_empty());
    bundle.validate().expect("valid");
    assert_eq!(bundle.sha256().unwrap(), V1_BUNDLE_SHA256);
}

#[test]
fn a_schema_1_bundle_renders_the_bytes_the_old_engine_rendered() {
    assert_eq!(pdf_sha256(&document(fixture_bundle())), V1_PDF_SHA256);
}

#[test]
fn the_upgrade_is_schema_2_keeps_every_file_and_renders_the_same_bytes() {
    let old = document(fixture_bundle());
    let new = old.upgraded().expect("upgrade");
    assert_eq!(new.bundle.schema_version, BUNDLE_SCHEMA);
    assert!(new.bundle.packages.is_empty());
    for (path, bytes) in &old.bundle.files {
        assert_eq!(new.bundle.files.get(path), Some(bytes), "{path}");
    }
    assert_eq!(new.bundle.files.len(), old.bundle.files.len() + 1);
    assert_ne!(new.bundle_sha256, old.bundle_sha256);
    new.validate().expect("the upgraded document verifies");
    assert_eq!(pdf_sha256(&new), V1_PDF_SHA256);
    // Upgrading again changes nothing.
    assert_eq!(new.upgraded().unwrap().bundle_sha256, new.bundle_sha256);
    // The old document still verifies and renders on its own.
    old.validate().expect("the old document still verifies");
}

#[test]
fn a_schema_2_bundle_round_trips_unchanged() {
    let new = document(fixture_bundle()).upgraded().expect("upgrade");
    let json = serde_json::to_vec(&*new.bundle).expect("serialize");
    let back: StyleBundle = serde_json::from_slice(&json).expect("deserialize");
    assert_eq!(back.sha256().unwrap(), new.bundle_sha256);
    assert_eq!(serde_json::to_vec(&back).unwrap(), json);
    let doc = serde_json::to_vec(&new).expect("serialize document");
    let doc_back: StyleDocument = serde_json::from_slice(&doc).expect("deserialize document");
    doc_back.validate().expect("verifies");
    assert_eq!(serde_json::to_vec(&doc_back).unwrap(), doc);
}

#[test]
fn a_schema_1_document_round_trips_through_json_and_still_verifies() {
    let old = document(fixture_bundle());
    let json = serde_json::to_vec(&old).expect("serialize");
    let back: StyleDocument =
        serde_json::from_slice(&json).expect("a stored schema 1 document loads");
    back.validate().expect("verifies");
    assert_eq!(back.bundle_sha256, V1_BUNDLE_SHA256);
}
