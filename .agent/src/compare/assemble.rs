//! Comparison PDF assembly from finished variant PDFs.
//!
//! `interleave` copies every page object unchanged with lopdf, so vectors,
//! fonts and the text layer survive exactly. `Sheet` composes one wide page
//! per row with Typst, which embeds each variant page as a vector image.

use anyhow::{Context, Result, ensure};
use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat, dictionary};
use serde_json::json;
use sha2::{Digest, Sha256};

/// One finished variant: its column letter, label and PDF bytes.
pub(crate) struct Column<'a> {
    pub letter: char,
    pub label: &'a str,
    pub pdf: &'a [u8],
}

/// Page sizes in points, in page order.
pub(crate) fn page_sizes(pdf: &[u8]) -> Result<Vec<(f64, f64)>> {
    let document = Document::load_mem(pdf).context("cannot parse PDF")?;
    ensure!(
        !document.is_encrypted(),
        "encrypted PDFs cannot be compared"
    );
    document
        .get_pages()
        .values()
        .map(|id| {
            let media = crate::pdf::inherited(&document, *id, b"MediaBox")
                .context("PDF page has no MediaBox")?
                .as_array()?;
            ensure!(media.len() == 4, "PDF page has an invalid MediaBox");
            let width = f64::from(media[2].as_float()? - media[0].as_float()?);
            let height = f64::from(media[3].as_float()? - media[1].as_float()?);
            ensure!(
                width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
                "PDF page has invalid dimensions"
            );
            Ok((width, height))
        })
        .collect()
}

/// The page label of one interleaved page: `<row>·<variant label>`.
pub(crate) fn page_label(row: u32, label: &str, blank: bool) -> String {
    if blank {
        format!("{row}·{label} (blank)")
    } else {
        format!("{row}·{label}")
    }
}

/// Interleave the selected `rows` (1-based page numbers) of every column:
/// 1A 1B … 1N, 2A 2B … 2N. A column without a row gets a blank page the
/// size of its own last page so rows stay aligned. Two columns open side by
/// side (`/PageLayout /TwoColumnLeft`); page labels and a row outline name
/// every page without touching its content.
pub(crate) fn interleave(columns: &[Column], rows: &[u32], title: &str) -> Result<Vec<u8>> {
    ensure!(
        columns.len() >= 2,
        "a comparison needs at least two variants"
    );
    ensure!(!rows.is_empty(), "a comparison needs at least one page row");
    let mut output = Document::with_version("1.7");
    let pages_id = output.new_object_id();
    let mut column_pages = Vec::with_capacity(columns.len());
    for column in columns {
        let (pages, version) = import(&mut output, column, pages_id)
            .with_context(|| format!("cannot import {}", column.label))?;
        if version > output.version {
            output.version = version;
        }
        column_pages.push(pages);
    }

    let mut kids = Vec::new();
    let mut labels = Vec::new();
    let mut row_pages = Vec::new();
    for &row in rows {
        let mut cells = Vec::new();
        for (column, pages) in columns.iter().zip(&column_pages) {
            let index = usize::try_from(row)? - 1;
            let (id, blank) = if let Some(id) = pages.get(index) {
                (*id, false)
            } else {
                let last = *pages.last().context("variant has no pages")?;
                let media = output
                    .get_dictionary(last)?
                    .get(b"MediaBox")
                    .context("page has no MediaBox")?
                    .clone();
                let blank = output.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => media,
                    "Resources" => Dictionary::new(),
                });
                (blank, true)
            };
            kids.push(Object::Reference(id));
            labels.push(Object::Integer(i64::try_from(labels.len() / 2)?));
            labels.push(Object::Dictionary(dictionary! {
                "P" => text(&page_label(row, column.label, blank)),
            }));
            cells.push((id, column, blank));
        }
        row_pages.push((row, cells));
    }
    let count = i64::try_from(kids.len())?;
    output.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );

    let outlines_id = output.new_object_id();
    let row_ids = row_pages
        .iter()
        .map(|_| output.new_object_id())
        .collect::<Vec<_>>();
    let mut visible = 0;
    for (position, (row, cells)) in row_pages.iter().enumerate() {
        let child_ids = cells
            .iter()
            .map(|_| output.new_object_id())
            .collect::<Vec<_>>();
        for (index, (page, column, blank)) in cells.iter().enumerate() {
            let mut child = dictionary! {
                "Title" => text(&if *blank {
                    format!("{} {} (no page {row})", column.letter, column.label)
                } else {
                    format!("{} {}", column.letter, column.label)
                }),
                "Parent" => row_ids[position],
                "Dest" => vec![Object::Reference(*page), "Fit".into()],
            };
            link_siblings(&mut child, &child_ids, index);
            output
                .objects
                .insert(child_ids[index], Object::Dictionary(child));
        }
        let mut entry = dictionary! {
            "Title" => text(&format!("Page {row}")),
            "Parent" => outlines_id,
            "Dest" => vec![Object::Reference(cells[0].0), "Fit".into()],
            "First" => child_ids[0],
            "Last" => *child_ids.last().expect("a row has cells"),
            "Count" => i64::try_from(child_ids.len())?,
        };
        link_siblings(&mut entry, &row_ids, position);
        output
            .objects
            .insert(row_ids[position], Object::Dictionary(entry));
        visible += 1 + child_ids.len();
    }
    output.objects.insert(
        outlines_id,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => row_ids[0],
            "Last" => *row_ids.last().expect("rows are not empty"),
            "Count" => i64::try_from(visible)?,
        }),
    );

    let mut catalog = dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "PageLabels" => dictionary! { "Nums" => labels },
        "Outlines" => outlines_id,
    };
    if columns.len() == 2 {
        catalog.set("PageLayout", "TwoColumnLeft");
    }
    let catalog_id = output.add_object(catalog);
    let info_id = output.add_object(dictionary! {
        "Title" => text(title),
        "Producer" => text("ccvl compare"),
    });
    let identifier = identifier(columns, rows, title);
    output.trailer.set("Root", catalog_id);
    output.trailer.set("Info", info_id);
    output.trailer.set(
        "ID",
        vec![
            Object::String(identifier.clone(), StringFormat::Hexadecimal),
            Object::String(identifier, StringFormat::Hexadecimal),
        ],
    );
    output.prune_objects();
    output.renumber_objects();
    let mut bytes = Vec::new();
    output
        .save_to(&mut bytes)
        .context("cannot write comparison PDF")?;
    Ok(bytes)
}

/// Move one variant's objects into `output` and reparent its pages. Its
/// catalog, structure tree and metadata stay behind and are pruned later.
fn import(
    output: &mut Document,
    column: &Column,
    pages_id: ObjectId,
) -> Result<(Vec<ObjectId>, String)> {
    let mut source = Document::load_mem(column.pdf).context("cannot parse PDF")?;
    ensure!(!source.is_encrypted(), "encrypted PDFs cannot be compared");
    source.renumber_objects_with(output.max_id + 1);
    let pages = source.get_pages().into_values().collect::<Vec<_>>();
    ensure!(!pages.is_empty(), "PDF has no pages");
    for &id in &pages {
        let mut resolved = Vec::new();
        for key in [b"MediaBox".as_slice(), b"CropBox", b"Resources", b"Rotate"] {
            if let Some(value) = crate::pdf::inherited(&source, id, key) {
                resolved.push((key, value.clone()));
            }
        }
        let page = source.get_dictionary_mut(id)?;
        for (key, value) in resolved {
            page.set(key, value);
        }
        page.set("Parent", pages_id);
        // The structure tree stays behind, so its back references go too.
        page.remove(b"StructParents");
    }
    for object in source.objects.values_mut() {
        if let Object::Dictionary(dictionary) = object
            && dictionary
                .get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Annot")
        {
            dictionary.remove(b"StructParent");
        }
    }
    output.max_id = source.max_id;
    output.objects.extend(source.objects);
    Ok((pages, source.version))
}

fn link_siblings(dictionary: &mut Dictionary, ids: &[ObjectId], index: usize) {
    if index > 0 {
        dictionary.set("Prev", ids[index - 1]);
    }
    if let Some(next) = ids.get(index + 1) {
        dictionary.set("Next", *next);
    }
}

/// A PDF text string: ASCII literal, otherwise UTF-16BE with a byte order mark.
fn text(value: &str) -> Object {
    if value.is_ascii() {
        Object::String(value.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in value.encode_utf16() {
            bytes.extend(unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

/// A content-derived file identifier, so identical inputs give identical bytes.
fn identifier(columns: &[Column], rows: &[u32], title: &str) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(title.as_bytes());
    for row in rows {
        hash.update(row.to_be_bytes());
    }
    for column in columns {
        hash.update(column.label.as_bytes());
        hash.update(Sha256::digest(column.pdf));
    }
    hash.finalize()[..16].to_vec()
}

const SHEET_SOURCE: &str = r##"#let data = json("/comparison.json")
#set document(title: data.title)
#set text(font: "Archivo", size: 8pt, fill: rgb("#333333"))
#set page(width: data.width * 1pt, height: data.height * 1pt, margin: 0pt, fill: rgb("#e9e9e9"))
#for (index, sheet) in data.sheets.enumerate() {
  if index > 0 { pagebreak() }
  for cell in sheet.cells {
    place(top + left, dx: cell.x * 1pt, dy: data.caption_y * 1pt, box(
      width: cell.width * 1pt,
      height: data.caption * 1pt,
      clip: true,
      text(cell.caption),
    ))
    place(top + left, dx: cell.x * 1pt, dy: data.page_y * 1pt, if cell.file == none {
      rect(
        width: cell.width * 1pt,
        height: cell.height * 1pt,
        fill: white,
        stroke: (paint: rgb("#999999"), thickness: 0.6pt, dash: "dashed"),
        align(center + horizon, text(size: 11pt, fill: rgb("#777777"), cell.note)),
      )
    } else {
      box(stroke: 0.4pt + rgb("#b3b3b3"), image(
        cell.file,
        page: cell.page,
        width: cell.width * 1pt,
        height: cell.height * 1pt,
      ))
    })
  }
}
"##;

const MARGIN: f64 = 24.0;
const GAP: f64 = 18.0;
const CAPTION: f64 = 12.0;
const CAPTION_GAP: f64 = 4.0;

/// One page per row with every variant's page side by side under a caption.
pub(crate) struct Sheet {
    engine: ctypst::Engine,
    document: ctypst::Document,
}

impl Sheet {
    pub(crate) fn compose(columns: &[Column], rows: &[u32], title: &str) -> Result<Self> {
        ensure!(
            columns.len() >= 2,
            "a comparison needs at least two variants"
        );
        ensure!(!rows.is_empty(), "a comparison needs at least one page row");
        let sizes = columns
            .iter()
            .map(|column| page_sizes(column.pdf))
            .collect::<Result<Vec<_>>>()?;
        // Size each column by its widest selected page and size every sheet
        // alike, so the variants line up from row to row and print evenly.
        let cell = |sizes: &[(f64, f64)], row: u32| {
            let index = usize::try_from(row).expect("page rows fit in usize") - 1;
            sizes
                .get(index)
                .or(sizes.last())
                .copied()
                .expect("variant has pages")
        };
        let widths = sizes
            .iter()
            .map(|sizes| {
                rows.iter()
                    .map(|row| cell(sizes, *row).0)
                    .fold(0.0, f64::max)
            })
            .collect::<Vec<_>>();
        let tallest = sizes
            .iter()
            .flat_map(|sizes| rows.iter().map(|row| cell(sizes, *row).1))
            .fold(0.0, f64::max);
        let gaps = f64::from(u32::try_from(widths.len() - 1)?);
        let width = 2.0 * MARGIN + widths.iter().sum::<f64>() + GAP * gaps;
        let height = 2.0 * MARGIN + CAPTION + CAPTION_GAP + tallest;
        let mut sheets = Vec::new();
        for &row in rows {
            let mut x = MARGIN;
            let mut cells = Vec::new();
            for ((index, column), sizes) in columns.iter().enumerate().zip(&sizes) {
                let (page_width, page_height) = cell(sizes, row);
                let present = usize::try_from(row)? <= sizes.len();
                cells.push(json!({
                    "x": x,
                    "width": page_width,
                    "height": page_height,
                    "caption": if present {
                        format!("{} · {} · page {row}", column.letter, column.label)
                    } else {
                        format!("{} · {} · no page {row}", column.letter, column.label)
                    },
                    "file": present.then(|| format!("/variant-{}.pdf", index + 1)),
                    "page": row,
                    "note": format!("no page {row}"),
                }));
                x += widths[index] + GAP;
            }
            sheets.push(json!({ "cells": cells }));
        }
        let data = json!({
            "title": title,
            "width": width,
            "height": height,
            "caption": CAPTION,
            "caption_y": MARGIN,
            "page_y": MARGIN + CAPTION + CAPTION_GAP,
            "sheets": sheets,
        });
        let engine = ctypst::Engine::builder()
            .fonts(ctypst::fonts::archivo())
            .build()
            .context("cannot initialize the comparison sheet engine")?;
        let mut request = ctypst::CompileRequest::new("/comparison.typ")
            .source_file("/comparison.typ", SHEET_SOURCE)
            .binary_file("/comparison.json", serde_json::to_vec(&data)?)
            .pages(ctypst::PageConstraint::Exactly(rows.len()));
        for (index, column) in columns.iter().enumerate() {
            request = request.binary_file(format!("/variant-{}.pdf", index + 1), column.pdf);
        }
        let document = engine
            .compile(request)
            .context("cannot compose the comparison sheet")?
            .document;
        Ok(Self { engine, document })
    }

    pub(crate) fn pdf(&self) -> Result<Vec<u8>> {
        self.engine
            .pdf(&self.document, crate::render::source_date_epoch()?)
            .context("cannot export the comparison sheet")
    }

    /// PNG bytes of one zero-based sheet page at two pixels per point.
    pub(crate) fn png(&self, page: usize) -> Result<Vec<u8>> {
        let raster = self
            .engine
            .rasterize(&self.document, page)
            .context("cannot rasterize the comparison sheet; compare fewer variants per preview")?;
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, raster.width, raster.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&raster.pixels)?;
        }
        Ok(bytes)
    }
}
