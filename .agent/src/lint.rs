//! Advisory hints that read like linter output.
//!
//! Hints point at something worth a look; they never fail a check. Each one
//! names a precise location and a one-line reason. A deliberate deviation is
//! acknowledged in the data with a short written reason, which silences that
//! hint and keeps the decision reviewable. Only structural problems (an
//! unparsable file, a duplicate ID, a malformed acknowledgement) are errors.
//! See `.agent/docs/profile-interview.md#hints-and-acknowledgements`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde_json::{Map, Value};

use crate::workspace::Workspace;

pub const PLAN: &str = "interview/stations.toml";

/// The plan tables whose entries the CV source marks, with their marker name.
const ENTRY_TABLES: [(&str, &str); 3] = [
    ("stations", "station"),
    ("projects", "project"),
    ("competencies", "competency"),
];

const CANDIDATE_CONFIRMATIONS: [&str; 2] = ["author-confirmed:", "user-confirmed:"];
const THIRD_PARTY_CONFIRMATION: &str = "confirmed-by:";

// ---------------------------------------------------------------------------
// Structural validation (hard errors)
// ---------------------------------------------------------------------------

/// Validate the plan fields this module reads: project and competency
/// tables, `allow_drift` reasons, source reference forms and `not_true`.
pub fn validate_plan(document: &Value, location: &str) -> Result<()> {
    for (table, _) in ENTRY_TABLES {
        let Some(entries) = document.get(table) else {
            continue;
        };
        let entries = entries
            .as_array()
            .with_context(|| format!("{location}.{table} must be an array of tables"))?;
        let mut ids = BTreeSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let item = format!("{location}.{table}[{}]", index + 1);
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .with_context(|| format!("{item}.id: a non-empty id is required"))?;
            ensure!(ids.insert(id), "{item}.id: duplicate {table} id {id:?}");
            validate_reason(entry, "allow_drift", &item)?;
            validate_refs(entry, &item)?;
            if table == "stations" {
                // Station facts are validated with their ownership rules.
                continue;
            }
            for (fact_index, fact) in facts(entry).iter().enumerate() {
                let fact_item = format!("{item}.facts[{}]", fact_index + 1);
                fact.get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .with_context(|| format!("{fact_item}.id: a non-empty id is required"))?;
                ensure!(
                    fact.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty()),
                    "{fact_item}.text: fact text cannot be empty"
                );
            }
        }
        for (index, entry) in entries.iter().enumerate() {
            for (fact_index, fact) in facts(entry).iter().enumerate() {
                let fact_item = format!(
                    "{location}.{table}[{}].facts[{}]",
                    index + 1,
                    fact_index + 1
                );
                validate_reason(fact, "allow_drift", &fact_item)?;
                validate_refs(fact, &fact_item)?;
            }
        }
    }
    rules(document, location)?;
    Ok(())
}

fn validate_reason(value: &Value, key: &str, location: &str) -> Result<()> {
    if let Some(reason) = value.get(key) {
        ensure!(
            reason.as_str().is_some_and(|text| !text.trim().is_empty()),
            "{location}.{key}: an acknowledgement needs a short written reason"
        );
    }
    Ok(())
}

fn validate_refs(value: &Value, location: &str) -> Result<()> {
    let Some(refs) = value.get("source_refs") else {
        return Ok(());
    };
    let refs = refs
        .as_array()
        .with_context(|| format!("{location}.source_refs must be an array"))?;
    for reference in refs {
        let reference = reference
            .as_str()
            .with_context(|| format!("{location}.source_refs must contain strings"))?;
        if let Some(rest) = reference.strip_prefix(THIRD_PARTY_CONFIRMATION) {
            ensure!(
                confirmed_by(rest).is_some(),
                "{location}.source_refs: {reference:?} must read confirmed-by:<person>:<YYYY-MM-DD>"
            );
        }
    }
    Ok(())
}

/// Split `<person>:<YYYY-MM-DD>` into a non-empty person and a date.
fn confirmed_by(rest: &str) -> Option<(&str, &str)> {
    let (person, date) = rest.rsplit_once(':')?;
    let date_pattern = pattern(&DATE, r"^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])$");
    (!person.trim().is_empty() && !person.contains(':') && date_pattern.is_match(date))
        .then_some((person.trim(), date))
}

static DATE: OnceLock<Regex> = OnceLock::new();

fn pattern(cell: &'static OnceLock<Regex>, source: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source).expect("valid built-in pattern"))
}

/// Validate an application record's optional `[lint]` table:
/// `allow = [{ term = "…", reason = "…" }, { figure = "…", reason = "…" }]`.
pub fn validate_record_lint(record: &Value, location: &str) -> Result<()> {
    let Some(lint) = record.get("lint") else {
        return Ok(());
    };
    let lint = lint
        .as_object()
        .with_context(|| format!("{location}.lint must be a table"))?;
    if let Some(key) = lint.keys().find(|key| *key != "allow") {
        bail!("{location}.lint: unknown field {key}; only allow is supported");
    }
    let Some(allow) = lint.get("allow") else {
        return Ok(());
    };
    let allow = allow
        .as_array()
        .with_context(|| format!("{location}.lint.allow must be an array"))?;
    for (index, item) in allow.iter().enumerate() {
        let item_location = format!("{location}.lint.allow[{}]", index + 1);
        let item = item
            .as_object()
            .with_context(|| format!("{item_location} must be a table"))?;
        let subjects = ["term", "figure"]
            .into_iter()
            .filter(|key| item.contains_key(*key))
            .collect::<Vec<_>>();
        ensure!(
            subjects.len() == 1,
            "{item_location}: name exactly one of term or figure"
        );
        ensure!(
            item.get(subjects[0])
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty()),
            "{item_location}.{}: must be non-empty text",
            subjects[0]
        );
        ensure!(
            item.get("reason")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty()),
            "{item_location}.reason: an acknowledgement needs a short written reason"
        );
        if let Some(key) = item
            .keys()
            .find(|key| !matches!(key.as_str(), "term" | "figure" | "reason"))
        {
            bail!("{item_location}: unknown field {key}");
        }
    }
    Ok(())
}

fn record_acknowledgements(record: &Value, subject: &str) -> Vec<String> {
    record
        .pointer("/lint/allow")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get(subject).and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------------------
// Typst text and CV entries
// ---------------------------------------------------------------------------

/// Reduce inline Typst markup to the plain text a reader sees:
/// `#brand[X]` becomes `X`, `\>` becomes `>`, `*bold*` loses its stars and
/// `~` becomes a space. Function calls without a content block vanish.
#[must_use]
pub fn plain_text(source: &str) -> String {
    let chars = source.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut out = String::new();
    plain_into(&chars, &mut index, &mut out, false);
    canonical(&out)
}

fn plain_into(chars: &[char], index: &mut usize, out: &mut String, nested: bool) {
    while let Some(&ch) = chars.get(*index) {
        *index += 1;
        match ch {
            '\\' => {
                if let Some(&next) = chars.get(*index) {
                    *index += 1;
                    out.push(if next.is_whitespace() { ' ' } else { next });
                }
            }
            ']' if nested => return,
            '[' => {
                out.push('[');
                plain_into(chars, index, out, true);
                out.push(']');
            }
            '*' => {}
            '~' => out.push(' '),
            '#' if chars.get(*index).is_some_and(|next| next.is_alphabetic()) => {
                while chars
                    .get(*index)
                    .is_some_and(|next| next.is_alphanumeric() || matches!(next, '-' | '_' | '.'))
                {
                    *index += 1;
                }
                if chars.get(*index) == Some(&'(') {
                    skip_balanced(chars, index, '(', ')');
                }
                if chars.get(*index) == Some(&'[') {
                    *index += 1;
                    plain_into(chars, index, out, true);
                }
            }
            _ => out.push(ch),
        }
    }
}

fn skip_balanced(chars: &[char], index: &mut usize, open: char, close: char) {
    let mut depth = 0_usize;
    let mut quoted = false;
    while let Some(&ch) = chars.get(*index) {
        *index += 1;
        match ch {
            '\\' if quoted => *index += 1,
            '"' => quoted = !quoted,
            _ if quoted => {}
            _ if ch == open => depth += 1,
            _ if ch == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Compare text by what it says, not how it is typed: one space between
/// words and a straight apostrophe.
fn canonical(text: &str) -> String {
    text.replace('’', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CvLine {
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CvEntry {
    /// `station`, `project` or `competency`.
    pub kind: String,
    pub id: String,
    pub line: usize,
    pub subheading: Option<CvLine>,
    pub bullets: Vec<CvLine>,
}

/// Read every marked entry of a CV source with its `#cv-s[...]` subheading
/// and `#cv-b[...]` bullets as plain text. Compact lines end an entry.
#[must_use]
pub fn cv_entries(source: &str) -> Vec<CvEntry> {
    let tokens = pattern(
        &CV_TOKENS,
        r"(?m)^[ \t]*(?:// ccvl-(station|project|competency): ([a-z0-9]+(?:[-_][a-z0-9]+)*)[ \t]*$|#cv-([sb])[\[(]|#cv-(?:pagebreak|hu|compact-heading|spacious-heading|superheading)\b)",
    );
    let mut entries = Vec::new();
    let mut current: Option<CvEntry> = None;
    for capture in tokens.captures_iter(source) {
        let whole = capture.get(0).expect("whole match");
        let line = line_number(source, whole.start());
        if let (Some(kind), Some(id)) = (capture.get(1), capture.get(2)) {
            entries.extend(current.take());
            current = Some(CvEntry {
                kind: kind.as_str().to_owned(),
                id: id.as_str().to_owned(),
                line,
                subheading: None,
                bullets: Vec::new(),
            });
        } else if let Some(function) = capture.get(3) {
            let Some(entry) = current.as_mut() else {
                continue;
            };
            let Some(body) = call_body(source, whole.end() - 1) else {
                continue;
            };
            let item = CvLine {
                line,
                text: plain_text(body),
            };
            if function.as_str() == "b" {
                entry.bullets.push(item);
            } else if entry.subheading.is_none() {
                entry.subheading = Some(item);
            }
        } else {
            entries.extend(current.take());
        }
    }
    entries.extend(current);
    entries
}

static CV_TOKENS: OnceLock<Regex> = OnceLock::new();

/// The content block of a call whose optional arguments start at `start`.
fn call_body(source: &str, start: usize) -> Option<&str> {
    let mut chars = source[start..].char_indices().peekable();
    if chars.peek().is_some_and(|(_, ch)| *ch == '(') {
        let mut depth = 0_usize;
        let mut quoted = false;
        for (_, ch) in chars.by_ref() {
            match ch {
                '"' => quoted = !quoted,
                '(' if !quoted => depth += 1,
                ')' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let (open, ch) = chars.next()?;
    if ch != '[' {
        return None;
    }
    let mut depth = 1_usize;
    let mut escaped = false;
    for (offset, ch) in chars {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[start + open + 1..start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn line_number(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

/// The CV sources the interview mirrors: every English `source_files` entry
/// (a file whose stem ends in `en`, such as `entries-en.typ`) of a CV style
/// that declares source files.
pub fn reference_sources(workspace: &Workspace) -> Result<Vec<PathBuf>> {
    let mut sources = BTreeSet::new();
    for definition in crate::styles::definitions(workspace, "cv")? {
        if definition.empty {
            continue;
        }
        let contract = crate::styles::contract(workspace, "cv", &definition.id)?;
        let root = crate::styles::root(workspace, "cv")?.join(&definition.id);
        for file in contract
            .get("source_files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let path = root.join(file);
            let english = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem == "en" || stem.ends_with("-en"));
            if english && path.is_file() {
                sources.insert(workspace.existing_inside(path)?);
            }
        }
    }
    Ok(sources.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Plan helpers
// ---------------------------------------------------------------------------

fn facts(entry: &Value) -> &[Value] {
    entry
        .get("facts")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Line numbers of entry and fact IDs in the raw plan, keyed by
/// `(table, id)` for entries and `("facts", id)` for facts.
struct PlanLines(BTreeMap<(String, String), usize>);

impl PlanLines {
    fn new(source: &str) -> Self {
        let header = pattern(&TABLE_HEADER, r"^\s*\[\[?\s*([A-Za-z0-9_-]+)");
        let ids = pattern(&ID_FIELD, r#"(?:^|[{,\s])id\s*=\s*"([^"]+)""#);
        let entry_id = pattern(&ENTRY_ID, r"^\s*id\s*=");
        let mut lines = BTreeMap::new();
        let mut table = String::new();
        for (index, line) in source.lines().enumerate() {
            if let Some(capture) = header.captures(line) {
                capture[1].clone_into(&mut table);
                continue;
            }
            for capture in ids.captures_iter(line) {
                let key = if entry_id.is_match(line) {
                    table.clone()
                } else {
                    "facts".to_owned()
                };
                lines
                    .entry((key, capture[1].to_owned()))
                    .or_insert(index + 1);
            }
        }
        Self(lines)
    }

    fn get(&self, location: &str, table: &str, id: &str) -> String {
        self.0
            .get(&(table.to_owned(), id.to_owned()))
            .map_or_else(|| location.to_owned(), |line| format!("{location}:{line}"))
    }
}

static TABLE_HEADER: OnceLock<Regex> = OnceLock::new();
static ID_FIELD: OnceLock<Regex> = OnceLock::new();
static ENTRY_ID: OnceLock<Regex> = OnceLock::new();

// ---------------------------------------------------------------------------
// Interview–CV drift
// ---------------------------------------------------------------------------

/// Compare each marked CV entry's subheading and bullets with the plan's
/// `<id>-role` and `<id>-<n>` facts. A project without a role fact compares
/// its `anchor`. Other facts, such as `<id>-scope`, are not compared.
pub fn drift_hints(
    workspace: &Workspace,
    document: &Value,
    plan_source: &str,
    plan_location: &str,
) -> Result<Vec<String>> {
    if document
        .get("stations")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Ok(Vec::new());
    }
    let mut hints = Vec::new();
    for path in reference_sources(workspace)? {
        let source = fs::read_to_string(&path)?;
        let cv_location = workspace.relative(&path)?.display().to_string();
        hints.extend(drift_against(
            document,
            plan_source,
            plan_location,
            &source,
            &cv_location,
        ));
    }
    Ok(hints)
}

fn drift_against(
    document: &Value,
    plan_source: &str,
    plan_location: &str,
    cv_source: &str,
    cv_location: &str,
) -> Vec<String> {
    let lines = PlanLines::new(plan_source);
    let mut hints = Vec::new();
    for entry in cv_entries(cv_source) {
        let Some((table, _)) = ENTRY_TABLES
            .iter()
            .find(|(_, marker)| *marker == entry.kind)
        else {
            continue;
        };
        let at_cv = |line: usize| format!("{cv_location}:{line}");
        let Some(planned) = document
            .get(*table)
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|item| text(item, "id") == entry.id))
        else {
            hints.push(format!(
                "{}: CV {} {} has no interview entry in {plan_location}",
                at_cv(entry.line),
                entry.kind,
                entry.id
            ));
            continue;
        };
        if planned.get("allow_drift").is_some() {
            continue;
        }
        let by_id = facts(planned)
            .iter()
            .map(|fact| (text(fact, "id"), fact))
            .collect::<BTreeMap<_, _>>();
        let compare = |fact_id: String, cv: Option<&CvLine>, what: String| -> Option<String> {
            let fact = by_id.get(fact_id.as_str()).copied();
            if fact.is_some_and(|fact| fact.get("allow_drift").is_some()) {
                return None;
            }
            let fact_location = lines.get(plan_location, "facts", &fact_id);
            match (fact, cv) {
                (Some(fact), Some(cv)) => {
                    let interview = canonical(text(fact, "text"));
                    (interview != cv.text).then(|| {
                        format!(
                            "{fact_location}: interview fact {fact_id} differs from CV {what} at {}; interview: {interview:?}; CV: {:?}",
                            at_cv(cv.line),
                            cv.text
                        )
                    })
                }
                (None, Some(cv)) => Some(format!(
                    "{}: CV {what} of {} {} has no interview fact {fact_id}: {:?}",
                    at_cv(cv.line),
                    entry.kind,
                    entry.id,
                    cv.text
                )),
                (Some(_), None) => Some(format!(
                    "{fact_location}: interview fact {fact_id} has no CV {what} in {} {} at {}",
                    entry.kind,
                    entry.id,
                    at_cv(entry.line)
                )),
                (None, None) => None,
            }
        };
        let role_id = format!("{}-role", entry.id);
        let anchor = planned.get("anchor").and_then(Value::as_str);
        if entry.kind == "project" && !by_id.contains_key(role_id.as_str()) && anchor.is_some() {
            if let Some(cv) = &entry.subheading {
                let anchor = canonical(anchor.unwrap_or_default());
                if anchor != cv.text {
                    hints.push(format!(
                        "{}: anchor of project {} differs from CV subheading at {}; interview: {anchor:?}; CV: {:?}",
                        lines.get(plan_location, table, &entry.id),
                        entry.id,
                        at_cv(cv.line),
                        cv.text
                    ));
                }
            }
        } else {
            hints.extend(compare(
                role_id,
                entry.subheading.as_ref(),
                "subheading".to_owned(),
            ));
        }
        let prefix = format!("{}-", entry.id);
        let highest = by_id
            .keys()
            .filter_map(|id| id.strip_prefix(prefix.as_str()))
            .filter_map(|rest| rest.parse::<usize>().ok())
            .max()
            .unwrap_or(0);
        for number in 1..=highest.max(entry.bullets.len()) {
            hints.extend(compare(
                format!("{}-{number}", entry.id),
                entry.bullets.get(number - 1),
                format!("bullet {number}"),
            ));
        }
    }
    hints
}

// ---------------------------------------------------------------------------
// Confirmation by someone other than the candidate
// ---------------------------------------------------------------------------

/// Entries and facts whose provenance is only `confirmed-by:<person>:<date>`
/// are worth a second look by the candidate. A fact with its own
/// `source_refs` is judged by those; otherwise it inherits its entry's.
#[must_use]
pub fn second_look_hints(document: &Value, plan_source: &str, plan_location: &str) -> Vec<String> {
    let lines = PlanLines::new(plan_source);
    let mut hints = Vec::new();
    for (table, marker) in ENTRY_TABLES {
        for entry in document
            .get(table)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = text(entry, "id");
            if let Some(people) = only_third_party(entry) {
                hints.push(format!(
                    "{}: {marker} {id} is confirmed only by {people}, not by the candidate; worth a second look",
                    lines.get(plan_location, table, id)
                ));
            }
            for fact in facts(entry) {
                if let Some(people) = only_third_party(fact) {
                    let fact_id = text(fact, "id");
                    hints.push(format!(
                        "{}: fact {fact_id} is confirmed only by {people}, not by the candidate; worth a second look",
                        lines.get(plan_location, "facts", fact_id)
                    ));
                }
            }
        }
    }
    hints
}

fn only_third_party(value: &Value) -> Option<String> {
    let refs = value
        .get("source_refs")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let people = refs
        .iter()
        .filter_map(|reference| reference.strip_prefix(THIRD_PARTY_CONFIRMATION))
        .filter_map(confirmed_by)
        .map(|(person, date)| format!("{person} ({date})"))
        .collect::<Vec<_>>();
    (!refs.is_empty() && people.len() == refs.len()).then(|| people.join(", "))
}

/// Whether a source reference records the candidate's own confirmation.
#[must_use]
pub fn candidate_confirmation(reference: &str) -> bool {
    CANDIDATE_CONFIRMATIONS
        .iter()
        .any(|prefix| reference.starts_with(prefix))
}

// ---------------------------------------------------------------------------
// False claims (`not_true`)
// ---------------------------------------------------------------------------

/// One `[[not_true]]` rule: a term the candidate must never be presented as
/// having, matched as a whole word (case-insensitive unless
/// `case_sensitive = true`), except inside the listed `except` phrases.
#[derive(Clone, Debug)]
pub struct Rule {
    pub term: String,
    pub note: String,
    pattern: Regex,
    except: Vec<Regex>,
}

impl Rule {
    /// The matched text of every occurrence outside an `except` phrase.
    fn find<'a>(&self, line: &'a str) -> Vec<&'a str> {
        let excepted = self
            .except
            .iter()
            .flat_map(|except| except.find_iter(line).map(|found| found.range()))
            .collect::<Vec<_>>();
        self.pattern
            .find_iter(line)
            .filter(|found| {
                !excepted
                    .iter()
                    .any(|range| range.start <= found.start() && found.end() <= range.end)
            })
            .map(|found| found.as_str())
            .collect()
    }
}

fn phrase_pattern(phrase: &str, case_sensitive: bool) -> Result<Regex> {
    let words = phrase
        .split_whitespace()
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\s+");
    let starts_word = phrase
        .trim()
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric);
    let ends_word = phrase
        .trim()
        .chars()
        .last()
        .is_some_and(char::is_alphanumeric);
    Ok(Regex::new(&format!(
        "{}{}{words}{}",
        if case_sensitive { "" } else { "(?i)" },
        if starts_word { r"\b" } else { "" },
        if ends_word { r"\b" } else { "" },
    ))?)
}

/// Parse and validate the plan's optional `[[not_true]]` rules.
pub fn rules(document: &Value, location: &str) -> Result<Vec<Rule>> {
    let Some(items) = document.get("not_true") else {
        return Ok(Vec::new());
    };
    let items = items
        .as_array()
        .with_context(|| format!("{location}.not_true must be an array of tables"))?;
    let mut seen = BTreeSet::new();
    let mut rules = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_location = format!("{location}.not_true[{}]", index + 1);
        let object = item
            .as_object()
            .with_context(|| format!("{item_location} must be a table"))?;
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "term" | "note" | "except" | "case_sensitive"))
        {
            bail!("{item_location}: unknown field {key}");
        }
        let term = non_empty(object, "term", &item_location)?;
        let note = non_empty(object, "note", &item_location)?;
        ensure!(
            seen.insert(term.to_lowercase()),
            "{item_location}.term: duplicate not_true term {term:?}"
        );
        let case_sensitive = match object.get("case_sensitive") {
            None => false,
            Some(value) => value
                .as_bool()
                .with_context(|| format!("{item_location}.case_sensitive must be a boolean"))?,
        };
        let except = match object.get("except") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .with_context(|| format!("{item_location}.except must be an array of phrases"))?
                .iter()
                .map(|phrase| {
                    let phrase = phrase
                        .as_str()
                        .filter(|phrase| !phrase.trim().is_empty())
                        .with_context(|| {
                            format!("{item_location}.except must contain non-empty phrases")
                        })?;
                    phrase_pattern(phrase, case_sensitive)
                })
                .collect::<Result<Vec<_>>>()?,
        };
        rules.push(Rule {
            pattern: phrase_pattern(term, case_sensitive)?,
            term: term.to_owned(),
            note: note.to_owned(),
            except,
        });
    }
    Ok(rules)
}

fn non_empty<'a>(object: &'a Map<String, Value>, key: &str, location: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .with_context(|| format!("{location}.{key}: non-empty text is required"))
}

/// Lines of a TOML source inside the named top-level tables, skipping
/// headers and comment lines.
fn toml_lines<'a>(source: &'a str, tables: &[&str]) -> Vec<(usize, &'a str)> {
    let header = pattern(&TABLE_HEADER, r"^\s*\[\[?\s*([A-Za-z0-9_-]+)");
    let mut current = String::new();
    let mut lines = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if let Some(capture) = header.captures(line) {
            capture[1].clone_into(&mut current);
            continue;
        }
        if tables.contains(&current.as_str()) && !line.trim_start().starts_with('#') {
            lines.push((index + 1, line));
        }
    }
    lines
}

fn typst_lines(source: &str) -> Vec<(usize, &str)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(index, line)| (index + 1, line))
        .collect()
}

fn term_hints(
    rules: &[Rule],
    lines: &[(usize, &str)],
    location: &str,
    acknowledged: &[String],
) -> (Vec<String>, BTreeSet<String>) {
    let mut hints = Vec::new();
    let mut used = BTreeSet::new();
    for rule in rules {
        let acknowledgement = acknowledged
            .iter()
            .find(|term| term.trim().eq_ignore_ascii_case(&rule.term));
        for (number, line) in lines {
            for found in rule.find(line) {
                if let Some(term) = acknowledgement {
                    used.insert(term.clone());
                    continue;
                }
                hints.push(format!(
                    "{location}:{number}: {found:?} matches not_true term {:?}: {}",
                    rule.term, rule.note
                ));
            }
        }
    }
    (hints, used)
}

/// `not_true` terms in the plan's own labels, anchors and facts: a fact that
/// contradicts the candidate's list deserves the same look as a document.
fn plan_term_hints(
    rules: &[Rule],
    document: &Value,
    plan_source: &str,
    plan_location: &str,
) -> Vec<String> {
    let lines = PlanLines::new(plan_source);
    let mut hints = Vec::new();
    for (table, marker) in ENTRY_TABLES {
        for entry in document
            .get(table)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = text(entry, "id");
            let mut texts = vec![
                (
                    lines.get(plan_location, table, id),
                    format!("{marker} {id}"),
                    text(entry, "label"),
                ),
                (
                    lines.get(plan_location, table, id),
                    format!("{marker} {id}"),
                    text(entry, "anchor"),
                ),
            ];
            for fact in facts(entry) {
                let fact_id = text(fact, "id");
                texts.push((
                    lines.get(plan_location, "facts", fact_id),
                    format!("fact {fact_id}"),
                    text(fact, "text"),
                ));
            }
            for (location, subject, value) in texts {
                for rule in rules {
                    for found in rule.find(value) {
                        hints.push(format!(
                            "{location}: {subject} says {found:?}, which matches not_true term {:?}: {}",
                            rule.term, rule.note
                        ));
                    }
                }
            }
        }
    }
    hints
}

/// Application records checked by hints: `opportunities/**/application.toml`
/// outside `variants/` and `review/` folders.
fn application_records(workspace: &Workspace) -> Result<Vec<PathBuf>> {
    let root = workspace.path("opportunities");
    let mut records = Vec::new();
    if !root.is_dir() {
        return Ok(records);
    }
    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            !(entry.file_type().is_dir()
                && matches!(entry.file_name().to_str(), Some("variants" | "review")))
        })
    {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name() == "application.toml" {
            records.push(entry.into_path());
        }
    }
    Ok(records)
}

/// Every `not_true` occurrence in application wording (`[cv]` and `[cl]`),
/// in `cvl/` wording sources and in the plan's own entries.
pub fn not_true_hints(
    workspace: &Workspace,
    document: &Value,
    plan_source: &str,
    plan_location: &str,
) -> Result<Vec<String>> {
    let rules = rules(document, plan_location)?;
    if rules.is_empty() {
        return Ok(Vec::new());
    }
    let mut hints = plan_term_hints(&rules, document, plan_source, plan_location);
    let cvl = workspace.path("cvl");
    let mut wording = Vec::new();
    if cvl.is_dir() {
        for entry in walkdir::WalkDir::new(&cvl)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry?;
            let extension = entry.path().extension().and_then(|value| value.to_str());
            if entry.file_type().is_file() && matches!(extension, Some("toml" | "typ")) {
                wording.push(entry.into_path());
            }
        }
    }
    wording.extend(application_records(workspace)?);
    for path in wording {
        hints.extend(file_term_hints(workspace, &rules, &path)?);
    }
    Ok(hints)
}

fn file_term_hints(workspace: &Workspace, rules: &[Rule], path: &Path) -> Result<Vec<String>> {
    let source = fs::read_to_string(path)?;
    let location = workspace.relative(path)?.display().to_string();
    if path.extension().is_some_and(|extension| extension == "typ") {
        return Ok(term_hints(rules, &typst_lines(&source), &location, &[]).0);
    }
    let record = toml::from_str::<toml::Value>(&source)
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Value::Null);
    let acknowledged = record_acknowledgements(&record, "term");
    let (mut hints, used) = term_hints(
        rules,
        &toml_lines(&source, &["cv", "cl"]),
        &location,
        &acknowledged,
    );
    for term in acknowledged.iter().filter(|term| !used.contains(*term)) {
        hints.push(format!(
            "{}: [lint] allow for term {term:?} matches nothing here; remove the stale acknowledgement",
            ack_location(&source, &location, "term", term)
        ));
    }
    Ok(hints)
}

fn ack_location(source: &str, location: &str, key: &str, value: &str) -> String {
    let needle = format!("{key} = {}", Value::String(value.to_owned()));
    source
        .lines()
        .position(|line| line.contains(&needle))
        .map_or_else(
            || location.to_owned(),
            |index| format!("{location}:{}", index + 1),
        )
}

// ---------------------------------------------------------------------------
// Unbacked figures (opt-in)
// ---------------------------------------------------------------------------

/// A quantified claim and the key it is compared by: its number without
/// grouping marks (`1'000+` and `1,000` both read `1000`), or `<n>-figure`
/// for written magnitudes (`eight-figure`, `achtstellig`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Figure {
    pub text: String,
    pub key: String,
}

const MAGNITUDE_WORDS: [(&str, &str); 7] = [
    ("four", "vier"),
    ("five", "fünf"),
    ("six", "sechs"),
    ("seven", "sieben"),
    ("eight", "acht"),
    ("nine", "neun"),
    ("ten", "zehn"),
];

/// Quantified claims: a number with a currency, magnitude, `%`, `+`, `×` or
/// `years`, and written magnitudes. Bare numbers, dates and grades are not
/// claims and are ignored.
#[must_use]
pub fn figures(text: &str) -> Vec<Figure> {
    let numeric = pattern(
        &FIGURE,
        r"(?i)(?P<cur>\b(?:CHF|EUR|USD|GBP)\s?|[€$£]\s?)?(?P<num>\d+(?:[.,'’]\d+)*)(?P<mag>\s?(?:(?:k|m|mn|bn|million(?:en)?|billion|milliarden)\b|mio\.?|mrd\.?))?(?P<suf>\s?(?:%|\+|×|x\b|years?\b|jahren?\b))?",
    );
    let written = pattern(
        &WRITTEN_FIGURE,
        r"(?i)\b(four|five|six|seven|eight|nine|ten)[- ]figure\b|\b(vier|fünf|sechs|sieben|acht|neun|zehn)stellig\w*",
    );
    let mut out = Vec::new();
    for capture in numeric.captures_iter(text) {
        let claim = ["cur", "mag", "suf"]
            .iter()
            .any(|name| capture.name(name).is_some());
        let preceded_by_word = capture.get(0).is_some_and(|whole| {
            text[..whole.start()]
                .chars()
                .last()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '-' || ch == '/')
        });
        if claim && !preceded_by_word {
            out.push(Figure {
                text: capture[0].trim().to_owned(),
                key: number_key(&capture["num"]),
            });
        }
    }
    for capture in written.captures_iter(text) {
        let word = capture
            .get(1)
            .or_else(|| capture.get(2))
            .map(|word| word.as_str().to_lowercase())
            .unwrap_or_default();
        let digits = MAGNITUDE_WORDS
            .iter()
            .position(|(english, german)| *english == word || *german == word)
            .map_or(0, |index| index + 4);
        out.push(Figure {
            text: capture[0].to_owned(),
            key: format!("{digits}-figure"),
        });
    }
    out
}

static FIGURE: OnceLock<Regex> = OnceLock::new();
static WRITTEN_FIGURE: OnceLock<Regex> = OnceLock::new();
static NUMBER: OnceLock<Regex> = OnceLock::new();

fn number_key(number: &str) -> String {
    let number = number.replace(['\'', '’'], "");
    let grouped = |separator: char| {
        let parts = number.split(separator).collect::<Vec<_>>();
        parts.len() > 1 && parts[0].len() <= 3 && parts[1..].iter().all(|part| part.len() == 3)
    };
    if grouped(',') {
        number.replace(',', "")
    } else if grouped('.') && !number.contains(',') && number.split('.').count() > 2 {
        number.replace('.', "")
    } else {
        number.replace(',', ".")
    }
}

/// Every figure key that occurs in interview fact text or entry anchors.
fn backed_figures(document: &Value) -> BTreeSet<String> {
    let number = pattern(&NUMBER, r"\d+(?:[.,'’]\d+)*");
    let mut keys = BTreeSet::new();
    for (table, _) in ENTRY_TABLES {
        for entry in document
            .get(table)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let texts = facts(entry)
                .iter()
                .map(|fact| text(fact, "text"))
                .chain([text(entry, "anchor")]);
            for item in texts {
                keys.extend(
                    number
                        .find_iter(item)
                        .map(|found| number_key(found.as_str())),
                );
                keys.extend(figures(item).into_iter().map(|figure| figure.key));
            }
        }
    }
    keys
}

/// String contents of TOML lines: single-line basic and literal strings, and
/// every line inside a multi-line string.
fn string_contents<'a>(lines: &[(usize, &'a str)]) -> Vec<(usize, &'a str)> {
    let literal = pattern(&STRING_LITERAL, r#""((?:[^"\\]|\\.)*)"|'([^']*)'"#);
    let mut out = Vec::new();
    let mut multiline = false;
    for (number, line) in lines {
        let delimiters = line.matches("\"\"\"").count() + line.matches("'''").count();
        if multiline || delimiters > 0 {
            out.push((*number, *line));
            if delimiters % 2 == 1 {
                multiline = !multiline;
            }
            continue;
        }
        for capture in literal.captures_iter(line) {
            if let Some(content) = capture.get(1).or_else(|| capture.get(2)) {
                out.push((*number, content.as_str()));
            }
        }
    }
    out
}

static STRING_LITERAL: OnceLock<Regex> = OnceLock::new();

/// Quantified claims in application wording that occur in no interview fact.
/// Opt-in (`ccvl check --figures`): rephrased or converted figures can differ
/// from their source, so this is a reading aid rather than a default hint.
pub fn figure_hints(workspace: &Workspace) -> Result<Vec<String>> {
    let path = workspace.path(PLAN);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let document = workspace.read_toml_value(PLAN)?;
    let backed = backed_figures(&document);
    let mut hints = Vec::new();
    for path in application_records(workspace)? {
        let source = fs::read_to_string(&path)?;
        let location = workspace.relative(&path)?.display().to_string();
        let record = workspace.read_toml_value(workspace.relative(&path)?)?;
        let acknowledged = record_acknowledgements(&record, "figure")
            .iter()
            .flat_map(|figure| {
                let parsed = figures(figure);
                if parsed.is_empty() {
                    vec![number_key(figure.trim())]
                } else {
                    parsed.into_iter().map(|figure| figure.key).collect()
                }
            })
            .collect::<BTreeSet<_>>();
        let lines = toml_lines(&source, &["cv", "cl"]);
        for (number, content) in string_contents(&lines) {
            for figure in figures(content) {
                if !backed.contains(&figure.key) && !acknowledged.contains(&figure.key) {
                    hints.push(format!(
                        "{location}:{number}: figure {:?} occurs in no interview fact; add the fact to {PLAN} or acknowledge it under [lint] allow",
                        figure.text
                    ));
                }
            }
        }
    }
    Ok(hints)
}

// ---------------------------------------------------------------------------
// Workspace entry points
// ---------------------------------------------------------------------------

/// Hints printed by `ccvl check`: interview–CV drift and `not_true` terms.
pub fn check_hints(workspace: &Workspace) -> Result<Vec<String>> {
    let path = workspace.path(PLAN);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(&path)?;
    let document = workspace.read_toml_value(PLAN)?;
    let mut hints = drift_hints(workspace, &document, &source, PLAN)?;
    hints.extend(not_true_hints(workspace, &document, &source, PLAN)?);
    Ok(hints)
}

/// `not_true` hints for one application record, printed while measuring or
/// building that opportunity.
pub fn record_hints(workspace: &Workspace, record: &Path) -> Result<Vec<String>> {
    let path = workspace.path(PLAN);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let document = workspace.read_toml_value(PLAN)?;
    file_term_hints(workspace, &rules(&document, PLAN)?, record)
}

// ---------------------------------------------------------------------------
// Readable listing
// ---------------------------------------------------------------------------

/// The whole interview as plain Markdown: stations with their role line and
/// facts, then projects, competencies and the `not_true` list.
#[must_use]
pub fn interview_markdown(document: &Value) -> String {
    let mut out = vec!["# Interview record".to_owned()];
    if let Some(updated) = document.get("updated").and_then(Value::as_str)
        && !updated.is_empty()
    {
        out.push(String::new());
        out.push(format!("Updated: {updated}"));
    }
    for (table, heading) in [
        ("stations", "Stations"),
        ("projects", "Projects"),
        ("competencies", "Competencies"),
    ] {
        let entries = document
            .get(table)
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        out.push(String::new());
        out.push(format!("## {heading}"));
        if entries.is_empty() {
            out.push(String::new());
            out.push("None recorded.".to_owned());
        }
        for entry in entries {
            let id = text(entry, "id");
            out.push(String::new());
            let label = text(entry, "label");
            out.push(if label.is_empty() {
                format!("### {id}")
            } else {
                format!("### {label} ({id})")
            });
            let mut details = Vec::new();
            for key in ["anchor", "kind", "status"] {
                let value = text(entry, key);
                if !value.is_empty() {
                    details.push(value.to_owned());
                }
            }
            if table == "stations" {
                details.push(match entry.get("page").and_then(Value::as_u64) {
                    Some(page) => format!("page {page}, {}", text(entry, "section")),
                    None => "unassigned".to_owned(),
                });
            }
            if !details.is_empty() {
                out.push(String::new());
                out.push(details.join(" · "));
            }
            let role_id = format!("{id}-role");
            let entry_facts = facts(entry);
            if let Some(role) = entry_facts.iter().find(|fact| text(fact, "id") == role_id) {
                out.push(String::new());
                out.push(format!("Role: {}", text(role, "text")));
            }
            let listed = entry_facts
                .iter()
                .filter(|fact| text(fact, "id") != role_id)
                .collect::<Vec<_>>();
            if !listed.is_empty() {
                out.push(String::new());
                out.extend(
                    listed
                        .iter()
                        .map(|fact| format!("- {}: {}", text(fact, "id"), text(fact, "text"))),
                );
            }
            let refs = entry
                .get("source_refs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !refs.is_empty() {
                out.push(String::new());
                out.push(format!("Sources: {}", refs.join(", ")));
            }
        }
    }
    out.push(String::new());
    out.push("## Not true".to_owned());
    out.push(String::new());
    let rules = document
        .get("not_true")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if rules.is_empty() {
        out.push("None recorded.".to_owned());
    }
    for rule in rules {
        let except = rule
            .get("except")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let mut line = format!("- {}: {}", text(rule, "term"), text(rule, "note"));
        if !except.is_empty() {
            let _ = write!(line, " (except: {})", except.join(", "));
        }
        out.push(line);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests;
