//! `ccvl compare`: one review PDF that shows n document variants side by side.
//!
//! Every item is a style selection, an application record or an existing
//! PDF. Selections and records render freshly through the `build-cv` and
//! `build-cl` path into `.agent/cache/comparisons/<name>/`; checked-in
//! outputs are never written. The variants are then interleaved page by page
//! (1A 1B … 1N, 2A …) or composed into one wide sheet per page row.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::render::{Compiler, DocumentSpec};
use crate::styles::{self, Selection};
use crate::workspace::Workspace;

mod assemble;

use assemble::{Column, Sheet};

/// Workspace-relative home of every comparison; ignored and never committed.
pub const OUTPUT_DIR: &str = ".agent/cache/comparisons";

/// Columns are lettered A to Z.
const MAX_VARIANTS: usize = 26;

/// The locale a style item renders in when neither `@<locale>` nor `--locale`
/// selects one and the style declares it.
const PREFERRED_LOCALE: &str = "en-ch";

static SELECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<style>[a-z0-9][a-z0-9_-]*)(?:/(?P<substyle>[a-z0-9][a-z0-9_-]*))?(?:@(?P<locale>[A-Za-z0-9_-]+))?(?::(?P<pages>[0-9]+))?$",
    )
    .expect("the selection pattern is valid")
});

static PATH_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<path>.+?)(?:@(?P<locale>[A-Za-z0-9_-]+))?(?::(?P<pages>[0-9]+))?$")
        .expect("the path suffix pattern is valid")
});

static NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("the comparison name pattern is valid")
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// 1A 1B … 1N, 2A 2B … 2N: original pages for an n-column viewer.
    #[default]
    Interleaved,
    /// One wide page per row with captions; works in every viewer and prints.
    Sheet,
}

impl Layout {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "interleaved" => Ok(Self::Interleaved),
            "sheet" => Ok(Self::Sheet),
            _ => bail!("unknown comparison layout {value:?}; expected interleaved or sheet"),
        }
    }
}

/// One `ccvl compare` invocation.
#[derive(Clone, Debug)]
pub struct Request {
    /// `cv` or `cl`.
    pub document: &'static str,
    pub items: Vec<String>,
    /// Locale for style items without `@<locale>`.
    pub locale: Option<String>,
    /// Page preset for rendered items without `:<preset>`.
    pub pages: Option<usize>,
    /// Page rows to keep, such as `1-2` or `1,3`.
    pub only: Option<String>,
    pub layout: Layout,
    pub name: Option<String>,
    /// Also write one PNG sheet per row.
    pub png: bool,
    /// Report line measurements per rendered variant.
    pub metrics: bool,
}

/// What a comparison produced, for people (`Display`) and agents (JSON).
#[derive(Debug, Serialize)]
pub struct Outcome {
    pub output: String,
    pub name: String,
    pub document: &'static str,
    pub layout: Layout,
    /// Page numbers shown, one row each.
    pub rows: Vec<u32>,
    /// Pages in the comparison PDF.
    pub pages: usize,
    pub page_layout: Option<&'static str>,
    pub viewer: String,
    /// Directory holding the freshly rendered variant PDFs and previews.
    pub work: String,
    pub previews: Vec<String>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Serialize)]
pub struct Variant {
    pub column: String,
    pub label: String,
    /// `style`, `record` or `pdf`.
    pub kind: &'static str,
    /// The item as given on the command line.
    pub item: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substyle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<usize>,
    /// Pages of this variant.
    pub pages: usize,
    /// Rows where this variant has no page and shows a blank one.
    pub blank_rows: Vec<u32>,
    pub pdf: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Metrics>,
}

/// Line measurements of one rendered variant, as `ccvl measure` reports them.
#[derive(Debug, Default, Serialize)]
pub struct Metrics {
    pub measured_lines: usize,
    pub outside_contract: Vec<String>,
    pub summary_fill: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Target {
    Style {
        style: String,
        substyle: Option<String>,
    },
    Record(PathBuf),
    Pdf(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Item {
    raw: String,
    target: Target,
    locale: Option<String>,
    pages: Option<usize>,
}

/// One resolved item, validated before anything renders.
struct Planned {
    item: Item,
    label: String,
    kind: &'static str,
    selection: Option<Selection>,
    locale: Option<String>,
    /// Pages the variant has: the rendered preset or the PDF's page count.
    pages: usize,
    spec: Option<DocumentSpec>,
    pdf: Option<(PathBuf, Vec<u8>)>,
}

const ITEM_FORMS: &str = "expected <style>/<substyle>[@<locale>][:<preset>], `default`, \
    an application.toml, a variant directory or a PDF";

/// Parse a `<style>[/<substyle>][@<locale>][:<preset>]` selection.
fn parse_selection(raw: &str) -> Option<Item> {
    let captures = SELECTION.captures(raw)?;
    let pages = match captures.name("pages") {
        Some(pages) => Some(pages.as_str().parse().ok()?),
        None => None,
    };
    Some(Item {
        raw: raw.to_owned(),
        target: Target::Style {
            style: captures["style"].to_owned(),
            substyle: captures
                .name("substyle")
                .map(|name| name.as_str().to_owned()),
        },
        locale: captures
            .name("locale")
            .map(|locale| locale.as_str().to_owned()),
        pages,
    })
}

fn locate(workspace: &Workspace, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.path(path)
    }
}

/// An existing PDF, record or variant directory, or `None` when nothing exists.
fn path_item(
    workspace: &Workspace,
    raw: &str,
    path: &str,
    locale: Option<&str>,
    pages: Option<usize>,
) -> Result<Option<Item>> {
    let located = locate(workspace, path);
    let target = if located.is_file() {
        match located
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("pdf") => {
                ensure!(
                    locale.is_none() && pages.is_none(),
                    "{raw}: a PDF is compared as is; it takes no @<locale> or :<preset>, and --only selects its pages"
                );
                Target::Pdf(located)
            }
            Some("toml") => Target::Record(located),
            _ => bail!("{raw}: {ITEM_FORMS}"),
        }
    } else if located.is_dir() {
        let record = located.join("application.toml");
        ensure!(
            record.is_file(),
            "{raw}: the directory has no application.toml; {ITEM_FORMS}"
        );
        Target::Record(record)
    } else {
        return Ok(None);
    };
    Ok(Some(Item {
        raw: raw.to_owned(),
        target,
        locale: locale.map(str::to_owned),
        pages,
    }))
}

fn looks_like_path(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.contains(".pdf")
        || lower.contains(".toml")
        || raw.starts_with(['.', '/', '~'])
        || raw.contains('\\')
        || raw.matches('/').count() > 1
}

/// An existing path wins over the selection grammar; a path may carry the
/// same `@<locale>` and `:<preset>` suffixes as a selection.
fn parse_item(workspace: &Workspace, raw: &str) -> Result<Item> {
    ensure!(
        !raw.trim().is_empty(),
        "empty comparison item; {ITEM_FORMS}"
    );
    if let Some(item) = path_item(workspace, raw, raw, None, None)? {
        return Ok(item);
    }
    if let Some(captures) = PATH_SUFFIX.captures(raw)
        && (captures.name("locale").is_some() || captures.name("pages").is_some())
    {
        let pages = captures
            .name("pages")
            .map(|pages| pages.as_str().parse::<usize>())
            .transpose()
            .with_context(|| format!("{raw}: invalid page preset"))?;
        let locale = captures.name("locale").map(|locale| locale.as_str());
        if let Some(item) = path_item(workspace, raw, &captures["path"], locale, pages)? {
            return Ok(item);
        }
    }
    if let Some(item) = parse_selection(raw) {
        return Ok(item);
    }
    if looks_like_path(raw) {
        bail!("{raw}: no such file or directory");
    }
    bail!("{raw}: {ITEM_FORMS}")
}

/// Parse `--only`: comma-separated page numbers and inclusive ranges.
fn parse_rows(value: &str) -> Result<Vec<u32>> {
    let mut rows = Vec::new();
    for part in value.split(',').map(str::trim) {
        let number = |text: &str| -> Result<u32> {
            let page = text
                .parse::<u32>()
                .with_context(|| format!("--only {value:?}: {text:?} is not a page number"))?;
            ensure!(page > 0, "--only {value:?}: pages count from 1");
            Ok(page)
        };
        if let Some((first, last)) = part.split_once('-') {
            let (first, last) = (number(first.trim())?, number(last.trim())?);
            ensure!(
                first <= last,
                "--only {value:?}: range {part} runs backwards"
            );
            rows.extend(first..=last);
        } else {
            rows.push(number(part)?);
        }
    }
    rows.sort_unstable();
    rows.dedup();
    Ok(rows)
}

/// The rows to show: `--only`, or every page of the longest variant.
fn select_rows(only: Option<&str>, longest: usize) -> Result<Vec<u32>> {
    let longest = u32::try_from(longest)?;
    let Some(only) = only else {
        return Ok((1..=longest).collect());
    };
    let rows = parse_rows(only)?;
    if let Some(beyond) = rows.iter().find(|row| **row > longest) {
        bail!("--only page {beyond} is beyond the longest variant ({longest} pages)");
    }
    Ok(rows)
}

fn slug(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// `<doc>-<a>-vs-<b>…[-<shared suffix>]`, such as
/// `cv-harvard-d-plus-vs-cluster-d-plus-en-ch-4`; shortened with a content
/// hash when it grows long.
fn derive_name(document: &str, labels: &[String]) -> String {
    // State an `@<locale>:<preset>` that every variant shares once, at the end.
    let suffix = |label: &str| label.rfind('@').map(|at| label[at..].to_owned());
    let shared = suffix(&labels[0])
        .filter(|first| {
            labels
                .iter()
                .all(|label| suffix(label).as_ref() == Some(first))
        })
        .unwrap_or_default();
    let joined = labels
        .iter()
        .map(|label| slug(&label[..label.len() - shared.len()]))
        .collect::<Vec<_>>()
        .join("-vs-");
    let name = if shared.is_empty() {
        format!("{document}-{joined}")
    } else {
        format!("{document}-{joined}-{}", slug(&shared))
    };
    if name.len() <= 96 {
        return name;
    }
    let digest = format!("{:x}", Sha256::digest(name.as_bytes()));
    format!("{}-{}", name[..80].trim_end_matches('-'), &digest[..8])
}

fn letter(index: usize) -> char {
    char::from(b'A' + u8::try_from(index).expect("at most 26 variants"))
}

/// Make labels unique: PDFs that share a file name show their path, and
/// remaining duplicates get their column letter.
fn unique_labels(planned: &mut [Planned]) {
    let counts = |planned: &[Planned]| {
        let mut counts = BTreeMap::<String, usize>::new();
        for entry in planned {
            *counts.entry(entry.label.clone()).or_default() += 1;
        }
        counts
    };
    let seen = counts(planned);
    for entry in planned.iter_mut() {
        if seen[&entry.label] > 1 && entry.kind == "pdf" {
            entry.label.clone_from(&entry.item.raw);
        }
    }
    let seen = counts(planned);
    for (index, entry) in planned.iter_mut().enumerate() {
        if seen[&entry.label] > 1 {
            entry.label = format!("{} #{}", entry.label, letter(index));
        }
    }
}

fn display_path(workspace: &Workspace, path: &Path) -> String {
    workspace
        .relative(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
        .replace('\\', "/")
}

/// The locale a style item renders in without an explicit selection.
fn default_locale(workspace: &Workspace, document: &str, style: &str) -> Result<String> {
    let definition = styles::definition(workspace, document, style)?;
    if definition
        .supports_locales
        .iter()
        .any(|locale| locale == PREFERRED_LOCALE)
    {
        return Ok(PREFERRED_LOCALE.to_owned());
    }
    definition
        .supports_locales
        .first()
        .cloned()
        .with_context(|| format!("{document} style {style} declares no locale"))
}

fn plan(workspace: &Workspace, request: &Request, item: Item) -> Result<Planned> {
    let document = request.document;
    match &item.target {
        Target::Style { style, substyle } => {
            let selection =
                styles::selection(workspace, document, Some(style), substyle.as_deref())?;
            let locale = match item.locale.as_ref().or(request.locale.as_ref()) {
                Some(locale) => locale.clone(),
                None => default_locale(workspace, document, &selection.style)?,
            };
            let leaf = styles::leaf(workspace, document, &locale, &selection)?;
            let mut spec = crate::render::build_spec(
                workspace,
                &leaf,
                None,
                item.pages.or(request.pages),
                None,
                None,
                None,
            )?;
            // The real output is assigned in the comparison's work directory.
            spec.output = PathBuf::new();
            Ok(Planned {
                label: format!(
                    "{}/{}@{}:{}",
                    selection.style, selection.substyle, leaf.locale, spec.expected_pages
                ),
                kind: "style",
                selection: Some(selection),
                locale: Some(leaf.locale.clone()),
                pages: spec.expected_pages,
                spec: Some(spec),
                pdf: None,
                item,
            })
        }
        Target::Record(path) => {
            let path = workspace.existing_inside(path)?;
            let location = display_path(workspace, &path);
            let record = crate::content::read_record(workspace, &path)?;
            let language = record
                .pointer("/options/language")
                .and_then(Value::as_str)
                .with_context(|| format!("{location}: options.language is missing"))?;
            if let Some(locale) = &item.locale {
                bail!(
                    "{}: a record renders in its own language ({language}); remove @{locale}",
                    item.raw
                );
            }
            let selection = styles::record_selection(workspace, document, &record, &location)?;
            let leaf = styles::leaf(workspace, document, language, &selection)?;
            let mut spec = crate::render::build_spec(
                workspace,
                &leaf,
                Some(&path),
                item.pages.or(request.pages),
                None,
                None,
                None,
            )?;
            spec.output = PathBuf::new();
            let directory = location
                .strip_suffix("/application.toml")
                .unwrap_or(&location)
                .to_owned();
            Ok(Planned {
                label: format!("{directory}@{}:{}", leaf.locale, spec.expected_pages),
                kind: "record",
                selection: Some(selection),
                locale: Some(leaf.locale.clone()),
                pages: spec.expected_pages,
                spec: Some(spec),
                pdf: None,
                item,
            })
        }
        Target::Pdf(path) => {
            // Read now: the work directory is cleared before rendering, and a
            // PDF from an earlier comparison may live there.
            let bytes = fs::read(path).with_context(|| format!("cannot read {}", item.raw))?;
            let pages = assemble::page_sizes(&bytes)
                .with_context(|| format!("{}: not a usable PDF", item.raw))?
                .len();
            Ok(Planned {
                label: path.file_name().map_or_else(
                    || item.raw.clone(),
                    |name| name.to_string_lossy().into_owned(),
                ),
                kind: "pdf",
                selection: None,
                locale: None,
                pages,
                spec: None,
                pdf: Some((path.clone(), bytes)),
                item,
            })
        }
    }
}

/// Refuse symlinked output locations so a comparison cannot write elsewhere.
fn guard(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink(),
            "comparison output path must not be a symlink: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", path.display()));
        }
    }
    Ok(())
}

/// Compile in report mode, as `check` does for the PDF it exports, so a draft
/// outside its line contracts still renders; `--metrics` shows the findings.
fn render(
    workspace: &Workspace,
    compiler: &Compiler,
    spec: &DocumentSpec,
    metrics: bool,
) -> Result<Option<Metrics>> {
    let mut report = spec.clone();
    report
        .inputs
        .insert("line-contracts".to_owned(), "report".to_owned());
    let document = compiler.compile(workspace, &report)?;
    compiler.export(spec, &document)?;
    Ok(metrics.then(|| {
        measure(workspace, spec, &document).unwrap_or_else(|error| Metrics {
            error: Some(format!("{error:#}")),
            ..Metrics::default()
        })
    }))
}

fn measure(
    workspace: &Workspace,
    spec: &DocumentSpec,
    document: &ctypst::Document,
) -> Result<Metrics> {
    let values = crate::measure::document_metrics(workspace, spec, document)?;
    let mut outside_contract = crate::measure::summary_failures(workspace, spec, &values)?;
    for (index, metric) in values.iter().enumerate() {
        if let Some(failure) = crate::measure::line_failure(spec, index, metric)? {
            outside_contract.push(failure);
        }
    }
    let summary_fill = values
        .iter()
        .filter(|metric| metric.get("kind").and_then(Value::as_str) == Some("cv-summary"))
        .filter_map(|metric| metric.get("actual_fill").and_then(Value::as_f64))
        .collect();
    Ok(Metrics {
        measured_lines: values.len(),
        outside_contract,
        summary_fill,
        error: None,
    })
}

fn viewer_hint(layout: Layout, columns: usize) -> String {
    match layout {
        Layout::Sheet => {
            "one page per row with every variant side by side; works in any viewer and prints"
                .to_owned()
        }
        Layout::Interleaved if columns == 2 => {
            "opens as two pages side by side (PageLayout TwoColumnLeft); otherwise choose a \
             two-page view without a separate cover page"
                .to_owned()
        }
        Layout::Interleaved => format!(
            "set the viewer to {columns} pages per row (for example Okular's Overview mode with \
             {columns} columns); PDF has no standard {columns}-column layout, so --layout sheet \
             works everywhere"
        ),
    }
}

pub fn run(workspace: &Workspace, request: &Request) -> Result<Outcome> {
    ensure!(
        matches!(request.document, "cv" | "cl"),
        "unknown document: {}",
        request.document
    );
    ensure!(
        request.items.len() >= 2,
        "a comparison needs at least two items; {ITEM_FORMS}"
    );
    ensure!(
        request.items.len() <= MAX_VARIANTS,
        "a comparison shows at most {MAX_VARIANTS} variants"
    );
    if let Some(name) = &request.name {
        ensure!(
            NAME.is_match(name),
            "--name must be lowercase letters, digits and hyphens: {name}"
        );
    }
    if let Some(only) = &request.only {
        parse_rows(only)?;
    }

    // Resolve and validate every item and the page rows before rendering.
    let mut planned = Vec::with_capacity(request.items.len());
    for raw in &request.items {
        let item = parse_item(workspace, raw)?;
        planned.push(
            plan(workspace, request, item).with_context(|| format!("comparison item {raw}"))?,
        );
    }
    let longest = planned
        .iter()
        .map(|entry| entry.pages)
        .max()
        .expect("at least two variants");
    let rows = select_rows(request.only.as_deref(), longest)?;
    unique_labels(&mut planned);
    let labels = planned
        .iter()
        .map(|entry| entry.label.clone())
        .collect::<Vec<_>>();
    let name = request
        .name
        .clone()
        .unwrap_or_else(|| derive_name(request.document, &labels));

    let root = workspace.path(OUTPUT_DIR);
    let work = root.join(&name);
    let output = root.join(format!("{name}.pdf"));
    for path in [
        workspace.path(".agent"),
        workspace.path(".agent/cache"),
        root.clone(),
        work.clone(),
        output.clone(),
    ] {
        guard(&path)?;
    }
    if work.exists() {
        fs::remove_dir_all(&work).with_context(|| format!("cannot clear {}", work.display()))?;
    }
    fs::create_dir_all(&work).with_context(|| format!("cannot create {}", work.display()))?;

    let compiler = if planned.iter().any(|entry| entry.spec.is_some()) {
        Some(Compiler::new(workspace)?)
    } else {
        None
    };
    let mut pdfs = Vec::with_capacity(planned.len());
    let mut metrics = Vec::with_capacity(planned.len());
    for (index, entry) in planned.iter_mut().enumerate() {
        if let Some(spec) = &mut entry.spec {
            spec.output = work.join(format!(
                "{}-{}.pdf",
                letter(index).to_ascii_lowercase(),
                slug(&entry.label)
            ));
            let compiler = compiler
                .as_ref()
                .expect("a compiler exists for rendered items");
            metrics.push(
                render(workspace, compiler, spec, request.metrics)
                    .with_context(|| match &entry.item.target {
                        Target::Record(path) if display_path(workspace, path).contains("/variants/") => format!(
                            "cannot render {}; variants are never built or checked, so compare the PDF kept beside its record if the record no longer renders",
                            entry.item.raw
                        ),
                        _ => format!("cannot render {}", entry.item.raw),
                    })?,
            );
            let bytes = fs::read(&spec.output)
                .with_context(|| format!("cannot read {}", spec.output.display()))?;
            entry.pdf = Some((spec.output.clone(), bytes));
        } else {
            metrics.push(None);
        }
        let (path, bytes) = entry.pdf.as_ref().expect("every item has a PDF");
        pdfs.push((path.clone(), bytes.clone()));
    }

    let columns = planned
        .iter()
        .zip(&pdfs)
        .enumerate()
        .map(|(index, (entry, (_, bytes)))| Column {
            letter: letter(index),
            label: &entry.label,
            pdf: bytes,
        })
        .collect::<Vec<_>>();
    let title = format!(
        "ccvl comparison: {}",
        columns
            .iter()
            .map(|column| format!("{} {}", column.letter, column.label))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let mut sheet = None;
    let bytes = match request.layout {
        Layout::Interleaved => assemble::interleave(&columns, &rows, &title)?,
        Layout::Sheet => sheet
            .insert(Sheet::compose(&columns, &rows, &title)?)
            .pdf()?,
    };
    fs::write(&output, &bytes).with_context(|| format!("cannot write {}", output.display()))?;

    let mut previews = Vec::new();
    if request.png {
        if sheet.is_none() {
            sheet = Some(Sheet::compose(&columns, &rows, &title)?);
        }
        let sheet = sheet.as_ref().expect("the preview sheet exists");
        for (index, row) in rows.iter().enumerate() {
            let path = work.join(format!("page-{row}.png"));
            fs::write(&path, sheet.png(index)?)
                .with_context(|| format!("cannot write {}", path.display()))?;
            previews.push(display_path(workspace, &path));
        }
    }

    let variants = planned
        .into_iter()
        .zip(pdfs)
        .zip(metrics)
        .enumerate()
        .map(|(index, ((entry, (path, _)), metrics))| Variant {
            column: letter(index).to_string(),
            label: entry.label,
            kind: entry.kind,
            item: entry.item.raw,
            style: entry
                .selection
                .as_ref()
                .map(|selection| selection.style.clone()),
            substyle: entry.selection.map(|selection| selection.substyle),
            locale: entry.locale,
            preset: entry.spec.as_ref().map(|spec| spec.expected_pages),
            pages: entry.pages,
            blank_rows: rows
                .iter()
                .copied()
                .filter(|row| usize::try_from(*row).is_ok_and(|row| row > entry.pages))
                .collect(),
            pdf: display_path(workspace, &path),
            metrics,
        })
        .collect::<Vec<_>>();
    Ok(Outcome {
        output: display_path(workspace, &output),
        name,
        document: request.document,
        layout: request.layout,
        pages: match request.layout {
            Layout::Interleaved => rows.len() * variants.len(),
            Layout::Sheet => rows.len(),
        },
        page_layout: (request.layout == Layout::Interleaved && variants.len() == 2)
            .then_some("TwoColumnLeft"),
        viewer: viewer_hint(request.layout, variants.len()),
        work: display_path(workspace, &work),
        previews,
        rows,
        variants,
    })
}

fn describe_rows(rows: &[u32]) -> String {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for &row in rows {
        match ranges.last_mut() {
            Some((_, last)) if *last + 1 == row => *last = row,
            _ => ranges.push((row, row)),
        }
    }
    ranges
        .iter()
        .map(|(first, last)| {
            if first == last {
                first.to_string()
            } else {
                format!("{first}-{last}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let document = if self.document == "cv" {
            "CV"
        } else {
            "cover-letter"
        };
        writeln!(
            formatter,
            "Compared {} {document} variants: {}",
            self.variants.len(),
            self.output
        )?;
        let order = self
            .rows
            .iter()
            .take(2)
            .map(|row| {
                self.variants
                    .iter()
                    .map(|variant| format!("{row}{}", variant.column))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(", ");
        match self.layout {
            Layout::Interleaved => writeln!(
                formatter,
                "Layout: interleaved, {} rows (pages {}) x {} columns = {} pages, order {order}{}",
                self.rows.len(),
                describe_rows(&self.rows),
                self.variants.len(),
                self.pages,
                if self.rows.len() > 2 { ", ..." } else { "" }
            )?,
            Layout::Sheet => writeln!(
                formatter,
                "Layout: sheet, {} pages (one per row: pages {}), {} variants side by side",
                self.pages,
                describe_rows(&self.rows),
                self.variants.len()
            )?,
        }
        let width = self
            .variants
            .iter()
            .map(|variant| variant.label.chars().count())
            .max()
            .unwrap_or(0);
        for variant in &self.variants {
            let blank = if variant.blank_rows.is_empty() {
                String::new()
            } else {
                format!(", blank on row {}", describe_rows(&variant.blank_rows))
            };
            writeln!(
                formatter,
                "  {}  {:<width$}  {} page{}{blank}",
                variant.column,
                variant.label,
                variant.pages,
                if variant.pages == 1 { "" } else { "s" }
            )?;
        }
        writeln!(formatter, "Rendered variants: {}/", self.work)?;
        if !self.previews.is_empty() {
            writeln!(formatter, "Previews: {}", self.previews.join(", "))?;
        }
        if self
            .variants
            .iter()
            .any(|variant| variant.metrics.is_some())
        {
            formatter.write_str(&metrics_table(&self.variants))?;
        }
        write!(formatter, "Viewer: {}", self.viewer)
    }
}

/// Measurements side by side: one column per variant, one row per figure.
fn metrics_table(variants: &[Variant]) -> String {
    let summary_lines = variants
        .iter()
        .filter_map(|variant| variant.metrics.as_ref())
        .map(|metrics| metrics.summary_fill.len())
        .max()
        .unwrap_or(0);
    let mut rows = vec![(
        "Metrics".to_owned(),
        variants
            .iter()
            .map(|variant| variant.column.clone())
            .collect::<Vec<_>>(),
    )];
    let cell = |variant: &Variant, value: &dyn Fn(&Metrics) -> String| match &variant.metrics {
        None => "n/a".to_owned(),
        Some(metrics) if metrics.error.is_some() => "error".to_owned(),
        Some(metrics) => value(metrics),
    };
    rows.push((
        "measured lines".to_owned(),
        variants
            .iter()
            .map(|variant| cell(variant, &|metrics| metrics.measured_lines.to_string()))
            .collect(),
    ));
    rows.push((
        "outside contract".to_owned(),
        variants
            .iter()
            .map(|variant| {
                cell(variant, &|metrics| {
                    metrics.outside_contract.len().to_string()
                })
            })
            .collect(),
    ));
    for line in 0..summary_lines {
        rows.push((
            format!("summary line {} fill", line + 1),
            variants
                .iter()
                .map(|variant| {
                    cell(variant, &|metrics| {
                        metrics
                            .summary_fill
                            .get(line)
                            .map_or_else(|| "-".to_owned(), |fill| format!("{fill:.1}%"))
                    })
                })
                .collect(),
        ));
    }
    let first = rows.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let widths = (0..variants.len())
        .map(|index| {
            rows.iter()
                .map(|(_, cells)| cells[index].len())
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let mut table = String::new();
    for (name, cells) in &rows {
        let mut line = format!("  {name:<first$}");
        for (cell, width) in cells.iter().zip(&widths) {
            write!(line, "  {cell:>width$}").expect("writing to a String is infallible");
        }
        table.push_str(line.trim_end());
        table.push('\n');
    }
    for variant in variants {
        let Some(metrics) = &variant.metrics else {
            continue;
        };
        if let Some(error) = &metrics.error {
            writeln!(table, "  {} error: {error}", variant.column)
                .expect("writing to a String is infallible");
        }
        for finding in &metrics.outside_contract {
            writeln!(table, "  {} {finding}", variant.column)
                .expect("writing to a String is infallible");
        }
    }
    table
}

#[cfg(test)]
mod tests;
