use std::fs;

use lopdf::{Document, Object};
use pretty_assertions::assert_eq;

use super::assemble::{self, Column, Sheet};
use super::*;

/// A small Typst PDF whose pages carry the given texts.
fn pdf(width: f64, height: f64, pages: &[&str]) -> Vec<u8> {
    let engine = ctypst::Engine::builder()
        .fonts(ctypst::fonts::archivo())
        .build()
        .unwrap();
    let source = format!(
        "#set page(width: {width}pt, height: {height}pt)\n#set text(font: \"Archivo\")\n{}",
        pages.join("\n#pagebreak()\n")
    );
    let document = engine
        .compile(ctypst::CompileRequest::new("/main.typ").source_file("/main.typ", source))
        .unwrap()
        .document;
    engine.pdf(&document, 0).unwrap()
}

fn page_texts(bytes: &[u8]) -> Vec<String> {
    let document = Document::load_mem(bytes).unwrap();
    document
        .get_pages()
        .keys()
        .map(|page| document.extract_text(&[*page]).unwrap().trim().to_owned())
        .collect()
}

fn page_labels(bytes: &[u8]) -> Vec<String> {
    let document = Document::load_mem(bytes).unwrap();
    let catalog = document.catalog().unwrap();
    let labels = catalog.get(b"PageLabels").unwrap().as_dict().unwrap();
    labels
        .get(b"Nums")
        .unwrap()
        .as_array()
        .unwrap()
        .chunks(2)
        .map(|pair| {
            let prefix = pair[1].as_dict().unwrap().get(b"P").unwrap();
            let Object::String(bytes, _) = prefix else {
                panic!("page label prefix is not a string")
            };
            if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
                String::from_utf16(
                    &utf16
                        .chunks(2)
                        .map(|unit| u16::from_be_bytes([unit[0], unit[1]]))
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            } else {
                String::from_utf8(bytes.clone()).unwrap()
            }
        })
        .collect()
}

fn columns<'a>(inputs: &'a [(&'a str, Vec<u8>)]) -> Vec<Column<'a>> {
    inputs
        .iter()
        .enumerate()
        .map(|(index, (label, pdf))| Column {
            letter: letter(index),
            label,
            pdf,
        })
        .collect()
}

#[test]
fn selections_follow_the_documented_grammar() {
    let item = parse_selection("harvard/d-plus@en-ch:4").unwrap();
    assert_eq!(
        item.target,
        Target::Style {
            style: "harvard".into(),
            substyle: Some("d-plus".into())
        }
    );
    assert_eq!(
        (item.locale.as_deref(), item.pages),
        (Some("en-ch"), Some(4))
    );
    for (raw, style, substyle, locale, pages) in [
        ("default", "default", None, None, None),
        ("cluster/default", "cluster", Some("default"), None, None),
        ("harvard", "harvard", None, None, None),
        ("harvard:2", "harvard", None, None, Some(2)),
        (
            "harvard/compact@de",
            "harvard",
            Some("compact"),
            Some("de"),
            None,
        ),
    ] {
        let item = parse_selection(raw).unwrap();
        assert_eq!(
            item.target,
            Target::Style {
                style: style.into(),
                substyle: substyle.map(Into::into)
            },
            "{raw}"
        );
        assert_eq!(
            (item.locale.as_deref(), item.pages),
            (locale, pages),
            "{raw}"
        );
    }
    for raw in [
        "Harvard/d-plus",
        "a/b/c",
        "harvard/d-plus:",
        "harvard@",
        "harvard/d plus",
        "",
    ] {
        assert!(parse_selection(raw).is_none(), "{raw}");
    }
}

#[test]
fn page_rows_accept_numbers_and_ranges() {
    assert_eq!(parse_rows("1-2").unwrap(), [1, 2]);
    assert_eq!(parse_rows("1,3").unwrap(), [1, 3]);
    assert_eq!(parse_rows("3, 1-2,2").unwrap(), [1, 2, 3]);
    for (value, message) in [
        ("0", "pages count from 1"),
        ("2-1", "runs backwards"),
        ("x", "is not a page number"),
        ("", "is not a page number"),
        ("1,", "is not a page number"),
    ] {
        let error = parse_rows(value).unwrap_err().to_string();
        assert!(error.contains(message), "{value}: {error}");
    }
    assert_eq!(select_rows(None, 3).unwrap(), [1, 2, 3]);
    assert_eq!(select_rows(Some("2"), 3).unwrap(), [2]);
    assert_eq!(
        select_rows(Some("1,5"), 4).unwrap_err().to_string(),
        "--only page 5 is beyond the longest variant (4 pages)"
    );
    assert_eq!(describe_rows(&[1, 2, 3, 5, 7, 8]), "1-3,5,7-8");
}

#[test]
fn names_are_safe_short_and_state_shared_selections_once() {
    let labels = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        derive_name(
            "cv",
            &labels(&["harvard/d-plus@en-ch:4", "cluster/d-plus@en-ch:4"])
        ),
        "cv-harvard-d-plus-vs-cluster-d-plus-en-ch-4"
    );
    assert_eq!(
        derive_name(
            "cl",
            &labels(&["harvard/frame@de-ch:1", "harvard/frame@en-ch:1"])
        ),
        "cl-harvard-frame-de-ch-1-vs-harvard-frame-en-ch-1"
    );
    assert_eq!(
        derive_name("cv", &labels(&["Old CV (4).pdf", "harvard/d-plus@en-ch:4"])),
        "cv-old-cv-4-pdf-vs-harvard-d-plus-en-ch-4"
    );
    let long = derive_name(
        "cv",
        &labels(&[
            "opportunities/acme/platform-lead@en-ch:2",
            "opportunities/acme/platform-lead/variants/harvard/compact/values@en-ch:2",
        ]),
    );
    assert!(long.len() <= 96, "{long}");
    assert!(NAME.is_match(&long), "{long}");
    assert_eq!(slug("Ünïcode · Name"), "n-code-name");
    assert_eq!(Layout::parse("sheet").unwrap(), Layout::Sheet);
    assert!(Layout::parse("grid").is_err());
}

#[test]
fn interleaving_orders_rows_pads_short_variants_and_labels_pages() {
    let inputs = [
        ("alpha", pdf(200.0, 300.0, &["A1", "A2", "A3"])),
        ("beta", pdf(220.0, 280.0, &["B1", "B2"])),
        ("gamma", pdf(200.0, 300.0, &["C1", "C2", "C3"])),
    ];
    let bytes = assemble::interleave(&columns(&inputs), &[1, 2, 3], "test").unwrap();
    assert_eq!(
        page_texts(&bytes),
        ["A1", "B1", "C1", "A2", "B2", "C2", "A3", "", "C3"]
    );
    assert_eq!(
        page_labels(&bytes),
        [
            "1·alpha",
            "1·beta",
            "1·gamma",
            "2·alpha",
            "2·beta",
            "2·gamma",
            "3·alpha",
            "3·beta (blank)",
            "3·gamma"
        ]
    );
    // The blank page takes the size of its own variant's pages.
    let sizes = assemble::page_sizes(&bytes).unwrap();
    assert_eq!(sizes[7], sizes[4]);
    assert_eq!(sizes[7], (220.0, 280.0));
    let document = Document::load_mem(&bytes).unwrap();
    let catalog = document.catalog().unwrap();
    assert!(
        !catalog.has(b"PageLayout"),
        "no standard three-column layout"
    );
    let outlines = document
        .get_dictionary(catalog.get(b"Outlines").unwrap().as_reference().unwrap())
        .unwrap();
    assert_eq!(outlines.get(b"Count").unwrap().as_i64().unwrap(), 3 + 9);
    let first = document
        .get_dictionary(outlines.get(b"First").unwrap().as_reference().unwrap())
        .unwrap();
    assert_eq!(
        first.get(b"Title").unwrap().as_str().unwrap(),
        b"Page 1".as_slice()
    );
}

#[test]
fn interleaving_copies_pages_exactly_and_deterministically() {
    let inputs = [
        ("alpha", pdf(200.0, 300.0, &["A1", "A2"])),
        ("beta", pdf(200.0, 300.0, &["B1", "B2"])),
    ];
    let bytes = assemble::interleave(&columns(&inputs), &[2], "pair").unwrap();
    assert_eq!(page_texts(&bytes), ["A2", "B2"]);
    let output = Document::load_mem(&bytes).unwrap();
    let catalog = output.catalog().unwrap();
    assert_eq!(
        catalog.get(b"PageLayout").unwrap().as_name().unwrap(),
        b"TwoColumnLeft"
    );
    let pages = output.get_pages();
    for (index, (_, source)) in inputs.iter().enumerate() {
        let source = Document::load_mem(source).unwrap();
        let copied = output.get_page_content(pages[&(u32::try_from(index).unwrap() + 1)]);
        assert_eq!(copied, source.get_page_content(source.get_pages()[&2]));
    }
    assert_eq!(
        assemble::interleave(&columns(&inputs), &[2], "pair").unwrap(),
        bytes
    );
    assert_ne!(
        assemble::interleave(&columns(&inputs), &[1], "pair").unwrap(),
        bytes
    );
    let error = assemble::interleave(&columns(&inputs[..1]), &[1], "one").unwrap_err();
    assert_eq!(
        error.to_string(),
        "a comparison needs at least two variants"
    );
}

#[test]
fn sheets_place_every_variant_of_a_row_on_one_captioned_page() {
    let inputs = [
        ("alpha", pdf(200.0, 300.0, &["A1", "A2"])),
        ("beta", pdf(220.0, 280.0, &["B1"])),
    ];
    let sheet = Sheet::compose(&columns(&inputs), &[1, 2], "sheet").unwrap();
    let bytes = sheet.pdf().unwrap();
    assert_eq!(
        bytes,
        Sheet::compose(&columns(&inputs), &[1, 2], "sheet")
            .unwrap()
            .pdf()
            .unwrap()
    );
    let sizes = assemble::page_sizes(&bytes).unwrap();
    // Margins, captions and the gap around the widest page of each column.
    let width = 2.0 * 24.0 + 200.0 + 220.0 + 18.0;
    let height = 2.0 * 24.0 + 12.0 + 4.0 + 300.0;
    assert_eq!(sizes.len(), 2);
    for (actual_width, actual_height) in sizes {
        assert!((actual_width - width).abs() < 0.01, "{actual_width}");
        assert!((actual_height - height).abs() < 0.01, "{actual_height}");
    }
    let texts = page_texts(&bytes);
    for expected in ["A · alpha · page 1", "B · beta · page 1"] {
        assert!(texts[0].contains(expected), "{expected}: {}", texts[0]);
    }
    for expected in ["A · alpha · page 2", "B · beta · no page 2"] {
        assert!(texts[1].contains(expected), "{expected}: {}", texts[1]);
    }
    // Variant pages are embedded as vector forms of their own size.
    assert_eq!(embedded_pages(&bytes, 1), [(200.0, 300.0), (220.0, 280.0)]);
    assert_eq!(embedded_pages(&bytes, 2), [(200.0, 300.0)]);
    assert!(sheet.png(1).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
}

/// Sizes of the form objects drawn on one page, found through nested
/// resources, in ascending order.
fn embedded_pages(bytes: &[u8], page: u32) -> Vec<(f32, f32)> {
    fn visit(document: &Document, resources: &lopdf::Dictionary, found: &mut Vec<(f32, f32)>) {
        let Ok(xobjects) = resources
            .get(b"XObject")
            .and_then(|object| document.dereference(object))
            .and_then(|(_, object)| object.as_dict())
        else {
            return;
        };
        for (_, reference) in xobjects {
            let Ok((_, Object::Stream(stream))) = document.dereference(reference) else {
                continue;
            };
            if let Ok(bbox) = stream.dict.get(b"BBox").and_then(Object::as_array) {
                let value = |index: usize| bbox[index].as_float().unwrap();
                found.push((value(2) - value(0), value(3) - value(1)));
            }
            if let Ok((_, Object::Dictionary(inner))) = stream
                .dict
                .get(b"Resources")
                .and_then(|object| document.dereference(object))
            {
                visit(document, inner, found);
            }
        }
    }
    let document = Document::load_mem(bytes).unwrap();
    let id = document.get_pages()[&page];
    let resources = crate::pdf::inherited(&document, id, b"Resources")
        .and_then(|object| document.dereference(object).ok())
        .and_then(|(_, object)| object.as_dict().ok())
        .unwrap();
    let mut found = Vec::new();
    visit(&document, resources, &mut found);
    found.retain(|(width, height)| {
        [(200.0, 300.0), (220.0, 280.0)].contains(&(width.round(), height.round()))
    });
    found.sort_by(|left, right| left.partial_cmp(right).unwrap());
    found.dedup();
    found
}

fn request(items: &[&str]) -> Request {
    Request {
        document: "cv",
        items: items.iter().map(|item| (*item).to_owned()).collect(),
        locale: None,
        pages: Some(2),
        only: None,
        layout: Layout::Interleaved,
        name: None,
        png: false,
        metrics: false,
    }
}

#[test]
fn style_items_render_fresh_into_the_comparison_cache() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    let mut compare = request(&["default", "ledger/compact@de-ch:3"]);
    compare.locale = Some("en-ch".into());
    compare.metrics = true;
    let outcome = run(&workspace, &compare).unwrap();
    assert_eq!(
        outcome.name,
        "cv-ledger-primary-en-ch-2-vs-ledger-compact-de-ch-3"
    );
    assert_eq!(
        outcome.output,
        ".agent/cache/comparisons/cv-ledger-primary-en-ch-2-vs-ledger-compact-de-ch-3.pdf"
    );
    let labels = outcome
        .variants
        .iter()
        .map(|variant| variant.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels, ["ledger/primary@en-ch:2", "ledger/compact@de-ch:3"]);
    assert_eq!(outcome.rows, [1, 2, 3]);
    assert_eq!(outcome.pages, 6);
    assert_eq!(outcome.page_layout, Some("TwoColumnLeft"));
    assert_eq!(outcome.variants[0].blank_rows, [3]);
    assert!(outcome.variants.iter().all(|variant| {
        variant.kind == "style"
            && variant.pdf.starts_with(".agent/cache/comparisons/")
            && variant
                .metrics
                .as_ref()
                .is_some_and(|metrics| metrics.error.is_none())
    }));
    let bytes = fs::read(workspace.path(&outcome.output)).unwrap();
    assert_eq!(Document::load_mem(&bytes).unwrap().get_pages().len(), 6);
    assert_eq!(page_labels(&bytes)[5], "3·ledger/compact@de-ch:3");
    // Checked-in leaf outputs are never written.
    for (substyle, locale) in [("primary", "en/ch"), ("compact", "de/ch")] {
        assert!(
            !workspace
                .path(format!("cvl/cv/ledger/{substyle}/{locale}/pdf"))
                .exists()
        );
    }
    let text = outcome.to_string();
    for expected in [
        "Compared 2 CV variants",
        "order 1A 1B, 2A 2B, ...",
        "B  ledger/compact@de-ch:3  3 pages",
        "blank on row 3",
        "measured lines",
        "PageLayout TwoColumnLeft",
    ] {
        assert!(text.contains(expected), "{expected}:\n{text}");
    }
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["layout"], "interleaved");
    assert_eq!(json["variants"][1]["preset"], 3);

    // The same inputs give the same bytes, and a rerun clears stale files.
    let stale = workspace.path(format!("{}/stale.png", outcome.work));
    fs::write(&stale, b"stale").unwrap();
    run(&workspace, &compare).unwrap();
    assert_eq!(fs::read(workspace.path(&outcome.output)).unwrap(), bytes);
    assert!(!stale.exists());
}

#[test]
fn records_variant_directories_and_pdfs_compare_with_their_own_options() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    let mut record =
        crate::content::read_record(&workspace, "cvl/cv/ledger/primary/en/ch/content.toml")
            .unwrap();
    let job = record["job"].as_object_mut().unwrap();
    job.retain(|key, _| {
        ["id", "title", "organization", "location", "cl_recipient"].contains(&key.as_str())
    });
    let write = |relative: &str, text: &str| {
        let path = workspace.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "opportunities/acme/lead/posting.md",
        "# Posting reference\n",
    );
    write(
        "opportunities/acme/lead/application.toml",
        &toml::to_string(&record).unwrap(),
    );
    record["options"]["cv_substyle"] = "compact".into();
    record["options"]["pages"] = 3.into();
    write(
        "opportunities/acme/lead/variants/ledger/compact/draft/application.toml",
        &toml::to_string(&record).unwrap(),
    );
    let first = run(&workspace, &request(&["ledger/primary", "ledger/compact"])).unwrap();
    let older = workspace.path(&first.variants[0].pdf);
    let mut compare = request(&[
        "opportunities/acme/lead",
        "opportunities/acme/lead/variants/ledger/compact/draft/:3",
        older.to_str().unwrap(),
    ]);
    compare.pages = None;
    compare.only = Some("2-3".into());
    compare.layout = Layout::Sheet;
    compare.name = Some("application-variants".into());
    compare.metrics = true;
    let outcome = run(&workspace, &compare).unwrap();
    let summary = outcome
        .variants
        .iter()
        .map(|variant| (variant.kind, variant.label.as_str(), variant.pages))
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            ("record", "opportunities/acme/lead@en-ch:4", 4),
            (
                "record",
                "opportunities/acme/lead/variants/ledger/compact/draft@en-ch:3",
                3
            ),
            ("pdf", "a-ledger-primary-en-ch-2.pdf", 2),
        ]
    );
    assert_eq!(outcome.rows, [2, 3]);
    assert_eq!(outcome.pages, 2);
    assert_eq!(outcome.page_layout, None);
    assert_eq!(outcome.variants[2].blank_rows, [3]);
    assert!(outcome.variants[2].metrics.is_none());
    assert!(outcome.to_string().contains("n/a"));
    assert_eq!(
        outcome.output,
        ".agent/cache/comparisons/application-variants.pdf"
    );
}

#[test]
fn invalid_items_fail_before_rendering_with_actionable_messages() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    let error = |items: &[&str]| format!("{:#}", run(&workspace, &request(items)).unwrap_err());
    for (items, message) in [
        (
            &["ledger/primary", "ledger/slot-3"][..],
            "cv ledger/slot-3 is an empty slot; it has no design yet",
        ),
        (
            &["ledger/primary", "slot-3"],
            "cv slot-3 is an empty style slot; it has no design yet",
        ),
        (&["ledger/primary"], "a comparison needs at least two items"),
        (
            &["ledger/primary", "ledger/unknown"],
            "unknown cv substyle \"unknown\"",
        ),
        (
            &["ledger/primary", "missing/old/cv-4.pdf"],
            "missing/old/cv-4.pdf: no such file or directory",
        ),
        (
            &["ledger/primary", "Ledger"],
            "Ledger: expected <style>/<substyle>",
        ),
        (
            &["ledger/primary", "ledger/primary@fr-ch"],
            "comparison item ledger/primary@fr-ch",
        ),
        (
            &["ledger/primary", "drafts/cv.toml@en-ch"],
            "drafts/cv.toml@en-ch: no such file or directory",
        ),
        (
            &["ledger/primary", "cvl/profile.toml"],
            "cvl/profile.toml: options.language is missing",
        ),
        (
            &["ledger/primary", "cvl"],
            "the directory has no application.toml",
        ),
    ] {
        let actual = error(items);
        assert!(actual.contains(message), "{items:?}: {actual}");
    }
    let mut bad_name = request(&["ledger/primary", "ledger/compact"]);
    bad_name.name = Some("Bad Name".into());
    assert!(
        run(&workspace, &bad_name)
            .unwrap_err()
            .to_string()
            .contains("--name must be lowercase")
    );
    let mut bad_rows = request(&["ledger/primary", "ledger/compact"]);
    bad_rows.only = Some("3".into());
    assert_eq!(
        run(&workspace, &bad_rows).unwrap_err().to_string(),
        "--only page 3 is beyond the longest variant (2 pages)"
    );
    // Every rejection happened before anything rendered or was written.
    assert!(!workspace.path(OUTPUT_DIR).exists());

    let pdf_path = workspace.path(".agent/cache/old.pdf");
    fs::create_dir_all(pdf_path.parent().unwrap()).unwrap();
    fs::write(&pdf_path, pdf(100.0, 100.0, &["old"])).unwrap();
    for item in [".agent/cache/old.pdf:2", ".agent/cache/old.pdf@en-ch"] {
        let message = error(&["ledger/primary", item]);
        assert!(message.contains("a PDF is compared as is"), "{message}");
    }
    let record = workspace.path("drafts/application.toml");
    fs::create_dir_all(record.parent().unwrap()).unwrap();
    fs::copy(
        workspace.path("cvl/cv/ledger/compact/en/ch/content.toml"),
        &record,
    )
    .unwrap();
    let message = error(&["ledger/primary", "drafts/application.toml"]);
    assert!(
        message.contains("wording references are allowed only"),
        "{message}"
    );
}

#[test]
fn record_items_reject_a_locale_override() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    let record =
        crate::content::read_record(&workspace, "cvl/cv/ledger/compact/en/ch/content.toml")
            .unwrap();
    let path = workspace.path("drafts/application.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, toml::to_string(&record).unwrap()).unwrap();
    let item = parse_item(&workspace, "drafts:2").unwrap();
    assert_eq!(
        (item.target, item.locale, item.pages),
        (Target::Record(path.clone()), None, Some(2))
    );
    let item = parse_item(&workspace, "drafts/application.toml").unwrap();
    assert_eq!((item.target, item.pages), (Target::Record(path), None));
    let with_locale = parse_item(&workspace, "drafts@de-ch:2").unwrap();
    assert_eq!(with_locale.locale.as_deref(), Some("de-ch"));
    let error = plan(&workspace, &request(&[]), with_locale)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        error,
        "drafts@de-ch:2: a record renders in its own language (en-ch); remove @de-ch"
    );
}
