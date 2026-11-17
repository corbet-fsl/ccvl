//! CSV import: the column-mapping guess and preview of `parseImportFile`, the row conversion of
//! `processImportData`, and the `job.create` ops the dashboard commits for the converted jobs.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};

use crate::transfer::ImportError;
use crate::transfer::columns::DEFAULT_COLUMNS;
use crate::transfer::csvread::{ImportCell, ImportRow, read_rows};
use crate::transfer::jsnum::{
    fuzzy_number, js_number, js_parse_float, js_round, js_trim, number_to_string,
};
use crate::transfer::settings::Settings;

/// How one file header maps to a workspace column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportColumnMapping {
    #[serde(rename = "fileHeader")]
    pub file_header: String,
    /// The target column id, empty when the guess is below the confidence threshold.
    #[serde(rename = "targetColumnId")]
    pub target_column_id: String,
    pub confidence: f64,
    #[serde(rename = "isCustom", default, skip_serializing_if = "Option::is_none")]
    pub is_custom: Option<bool>,
}

/// The preview shown before an import is committed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportPreview {
    #[serde(rename = "totalRows")]
    pub total_rows: usize,
    pub columns: Vec<ImportColumnMapping>,
    /// The first five rows.
    #[serde(rename = "previewData")]
    pub preview_data: Vec<ImportRow>,
    #[serde(rename = "rawData")]
    pub raw_data: Vec<ImportRow>,
}

const ALIASES: &[(&str, &[&str])] = &[
    (
        "organization",
        &["company", "firma", "employer", "unternehmen", "org"],
    ),
    ("title", &["role", "job", "position", "bezeichnung"]),
    ("location", &["city", "ort", "place", "stadt", "loc"]),
    (
        "salary",
        &[
            "pay",
            "compensation",
            "gehalt",
            "lohn",
            "verdienst",
            "money",
        ],
    ),
    ("source", &["origin", "platform", "quelle"]),
    ("url", &["link", "href", "website"]),
    ("status", &["phase", "stage", "state", "schritt"]),
    ("workload", &["percent", "pensum", "%"]),
    ("commute", &["travel", "distance", "fahrtweg"]),
    ("applied_at", &["applied", "sent", "datum", "bewerbung"]),
    ("follow_up_at", &["follow up", "next", "deadline", "frist"]),
    ("notes", &["misc", "comment", "bemerkung", "remark"]),
    (
        "description",
        &["posting", "stellenbeschreibung", "job description"],
    ),
    ("language", &["sprache", "lang"]),
];

/// Importable job fields that are not dashboard columns (`EXTRA_IMPORT_FIELDS`).
pub const EXTRA_IMPORT_FIELDS: &[(&str, &str)] = &[
    ("url", "URL"),
    ("notes", "Notes"),
    ("description", "Description"),
    ("language", "Language"),
];

const EXCLUDED: &[&str] = &["_category", "_drag", "delete", "cv", "cl", "misc_files"];

struct Candidate {
    id: String,
    label: String,
    sub_header: Option<String>,
    is_custom: bool,
}

fn guess_column_mapping(header: &str, settings: &Settings) -> ImportColumnMapping {
    let norm = js_trim(header).to_lowercase();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (id, label, sub, _) in DEFAULT_COLUMNS {
        if !EXCLUDED.contains(id) {
            candidates.push(Candidate {
                id: (*id).to_owned(),
                label: (*label).to_owned(),
                sub_header: Some((*sub).to_owned()),
                is_custom: false,
            });
        }
    }
    for (id, label) in EXTRA_IMPORT_FIELDS {
        candidates.push(Candidate {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
            sub_header: None,
            is_custom: false,
        });
    }
    for col in &settings.custom_columns {
        candidates.push(Candidate {
            id: col.id.clone(),
            label: col.label.clone(),
            sub_header: None,
            is_custom: true,
        });
    }

    let mut best: Option<(&Candidate, f64)> = None;
    let mut max_score = 0.0_f64;
    for cand in &candidates {
        let label = cand.label.to_lowercase();
        let id = cand.id.to_lowercase();
        let sub = cand
            .sub_header
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_default();
        let mut score = 0.0_f64;
        if norm == id || norm == label {
            score = 1.0;
        }
        if ALIASES
            .iter()
            .find(|(k, _)| *k == cand.id)
            .is_some_and(|(_, aliases)| aliases.iter().any(|a| norm.contains(a)))
        {
            score = score.max(0.9);
        }
        if norm.contains(&label) || label.contains(&norm) {
            score = score.max(0.8);
        }
        if !sub.is_empty() && (norm == sub || norm.contains(&sub)) {
            score = score.max(0.7);
        }
        if cand.is_custom && norm.contains(id.split('_').next().unwrap_or("")) {
            score = score.max(0.6);
        }
        if score > max_score {
            max_score = score;
            best = Some((cand, score));
        }
    }
    ImportColumnMapping {
        file_header: header.to_owned(),
        target_column_id: match best {
            Some((c, _)) if max_score > 0.5 => c.id.clone(),
            _ => String::new(),
        },
        confidence: max_score,
        is_custom: best.and_then(|(c, _)| c.is_custom.then_some(true)),
    }
}

/// `parseImportFile` for CSV bytes: parse the first sheet and guess a mapping for every header of
/// the first data row.
pub fn parse_import_csv(bytes: &[u8], settings: &Settings) -> Result<ImportPreview, ImportError> {
    let raw_data = read_rows(bytes)?;
    let Some(first) = raw_data.first() else {
        return Err(ImportError::Empty);
    };
    let columns = first
        .keys()
        .map(|h| guess_column_mapping(h, settings))
        .collect();
    Ok(ImportPreview {
        total_rows: raw_data.len(),
        columns,
        preview_data: raw_data.iter().take(5).cloned().collect(),
        raw_data,
    })
}

fn cell_string(cell: &ImportCell) -> String {
    match cell {
        ImportCell::Text(s) => s.clone(),
        ImportCell::Number(n) => number_to_string(*n),
        ImportCell::Bool(b) => b.to_string(),
        ImportCell::Null => "null".to_owned(),
    }
}

/// The columns whose cells are numbers; every other column takes the text as typed.
const NUMERIC_TARGETS: [&str; 3] = ["workload", "salary", "score"];

/// A cell for a column that wants a number: text that reads as a number (`80%`, `1,2`, `$5`) becomes
/// the number's text, which is what the reader produced for such cells before it kept them as typed.
fn numeric_string(cell: &ImportCell) -> String {
    match cell {
        ImportCell::Text(s) if !js_trim(s).is_empty() => {
            let n = fuzzy_number(s);
            if n.is_nan() {
                s.clone()
            } else {
                number_to_string(n)
            }
        }
        other => cell_string(other),
    }
}

/// A JSON number the way JavaScript would keep it: an integer when the value is integral.
fn num_value(n: f64) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Value::Number(Number::from(n as i64))
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// Inputs `processImportData` takes from its environment.
pub struct ImportContext<'a> {
    /// The current UTC date as `YYYY-MM-DD`.
    pub today: &'a str,
}

/// `processImportData`: convert the rows with the chosen mappings into job records. `new_id` mints
/// the part after `imported-` of each job id.
pub fn process_import_data(
    rows: &[ImportRow],
    mappings: &[ImportColumnMapping],
    settings: &Settings,
    ctx: &ImportContext<'_>,
    mut new_id: impl FnMut() -> String,
) -> Vec<Map<String, Value>> {
    let valid: Vec<&ImportColumnMapping> = mappings
        .iter()
        .filter(|m| !m.target_column_id.is_empty())
        .collect();
    let statuses = settings.sorted_statuses();
    let default_status = statuses
        .first()
        .map(|s| s.id.clone())
        .filter(|id| match id {
            Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
            Value::String(s) => !s.is_empty(),
            Value::Bool(b) => *b,
            _ => false,
        })
        .unwrap_or_else(|| json!(1));

    rows.iter()
        .map(|row| {
            let mut job = Map::new();
            job.insert("id".into(), Value::String(format!("imported-{}", new_id())));
            job.insert("title".into(), json!("Untitled Role"));
            job.insert("organization".into(), json!("Unknown Org"));
            job.insert("location".into(), json!(""));
            job.insert("salary".into(), json!(""));
            job.insert("commute".into(), json!(""));
            job.insert("workload".into(), json!("100"));
            job.insert("source".into(), json!("Imported"));
            job.insert("status_id".into(), default_status.clone());
            job.insert("applied_at".into(), json!(""));
            job.insert("follow_up_at".into(), json!(ctx.today));
            job.insert("snooze_count".into(), json!(0));
            job.insert("follow_up_manual".into(), json!(false));
            job.insert("files".into(), json!([]));
            job.insert("evaluations".into(), json!({}));

            for mapping in &valid {
                let Some(cell) = row.get(&mapping.file_header) else {
                    continue;
                };
                if *cell == ImportCell::Null {
                    continue;
                }
                let target = mapping.target_column_id.as_str();
                let numeric = mapping.is_custom == Some(true) || NUMERIC_TARGETS.contains(&target);
                let text = if numeric {
                    numeric_string(cell)
                } else {
                    cell_string(cell)
                };
                if mapping.is_custom == Some(true) {
                    let evaluations = job
                        .entry("evaluations")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .expect("evaluations is an object");
                    let low = text.to_lowercase();
                    let num = js_parse_float(&text);
                    if ["yes", "true", "y", "ja"].contains(&low.as_str()) {
                        evaluations.insert(target.to_owned(), json!(true));
                    } else if ["no", "false", "n", "nein"].contains(&low.as_str()) {
                        evaluations.insert(target.to_owned(), json!(false));
                    } else if !num.is_nan() && num.is_finite() {
                        evaluations.insert(target.to_owned(), num_value(num));
                    }
                } else if target == "status" {
                    let low = text.to_lowercase();
                    let found = statuses.iter().find(|s| {
                        let name = s.name.to_lowercase();
                        name == low || low.contains(&name)
                    });
                    if let Some(s) = found {
                        job.insert("status_id".into(), s.id.clone());
                    }
                } else {
                    job.insert(target.to_owned(), Value::String(text));
                }
            }
            job
        })
        .collect()
}

/// Inputs `bulkCreateJobs` takes from its environment.
pub struct OpContext<'a> {
    pub workspace_id: &'a str,
    /// The current UTC date as `YYYY-MM-DD`.
    pub today: &'a str,
    /// The current instant as an ISO 8601 string.
    pub now: &'a str,
    /// The follow-up date of the first phase, `None` when no rule gives one.
    pub initial_follow_up: Option<&'a str>,
}

fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                if x.is_null() {
                    String::new()
                } else {
                    js_string(x)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `numberOrNull` of the dashboard: a finite number, else `null`.
fn number_or_null(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if s.is_empty() => Value::Null,
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|f| f.is_finite())
            .map_or(Value::Null, num_value),
        Some(other) => {
            let n = js_number(&js_string(other));
            if n.is_finite() {
                num_value(n)
            } else {
                Value::Null
            }
        }
    }
}

/// `normalizeSalary`: a salary in thousands, as text; empty when none can be read.
fn normalize_salary(raw: Option<&Value>) -> String {
    let Some(raw) = raw else { return String::new() };
    let cleaned: String = match raw {
        Value::Null => return String::new(),
        Value::String(s) if s.is_empty() => return String::new(),
        Value::Number(n) => number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        other => js_string(other)
            .chars()
            .filter(|c| {
                !matches!(c, 'k' | 'K' | ',' | '\'') && !crate::transfer::jsnum::is_js_space(*c)
            })
            .collect(),
    };
    let Some(found) = first_decimal(&cleaned) else {
        return String::new();
    };
    let mut val = js_number(found);
    if val.is_nan() || val <= 0.0 {
        return String::new();
    }
    if val > 999.0 {
        val = js_round(val / 1000.0);
    }
    number_to_string(js_round(val))
}

/// The first match of `/-?\d+(?:\.\d+)?/`.
fn first_decimal(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    for start in 0..b.len() {
        let mut i = start;
        if b[i] == b'-' {
            i += 1;
        }
        let digits = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits {
            continue;
        }
        if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        return Some(&s[start..i]);
    }
    None
}

fn normalized_salary_number(v: Option<&Value>) -> Value {
    let normalized = normalize_salary(v);
    if normalized.is_empty() {
        return Value::Null;
    }
    let n = js_number(&normalized);
    if n.is_finite() {
        num_value(n)
    } else {
        Value::Null
    }
}

/// The `input.<key> ?? fallback` of the dashboard code.
fn or_default(input: &Map<String, Value>, key: &str, fallback: Value) -> Value {
    match input.get(key) {
        None | Some(Value::Null) => fallback,
        Some(v) => v.clone(),
    }
}

/// `jobCreateAttributes` of the dashboard.
#[allow(clippy::too_many_lines)]
fn job_create_attributes(input: &Map<String, Value>, ctx: &OpContext<'_>) -> Value {
    let mut origins: Map<String, Value> = match input.get("origins") {
        Some(Value::Object(o)) => o.clone(),
        Some(Value::String(s)) => s
            .chars()
            .enumerate()
            .map(|(i, c)| (i.to_string(), json!(c.to_string())))
            .collect(),
        _ => Map::new(),
    };
    for field in [
        "title",
        "organization",
        "location",
        "salary",
        "industry",
        "workload",
    ] {
        match input.get(field) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.is_empty() => {}
            Some(_) => {
                origins.insert(field.to_owned(), json!("human"));
            }
        }
    }
    let mut a = Map::new();
    a.insert("title".into(), or_default(input, "title", json!("")));
    a.insert(
        "organization".into(),
        or_default(input, "organization", json!("")),
    );
    a.insert(
        "industry".into(),
        or_default(input, "industry", Value::Null),
    );
    a.insert(
        "location".into(),
        or_default(input, "location", Value::Null),
    );
    a.insert("source".into(), or_default(input, "source", Value::Null));
    a.insert(
        "salary".into(),
        normalized_salary_number(input.get("salary")),
    );
    a.insert("commute".into(), or_default(input, "commute", Value::Null));
    a.insert("workload".into(), number_or_null(input.get("workload")));
    a.insert("status_id".into(), or_default(input, "status_id", json!(1)));
    a.insert("score".into(), number_or_null(input.get("score")));
    a.insert(
        "applied_at".into(),
        or_default(input, "applied_at", json!(ctx.today)),
    );
    a.insert(
        "follow_up_at".into(),
        or_default(
            input,
            "follow_up_at",
            ctx.initial_follow_up.map_or(Value::Null, |s| json!(s)),
        ),
    );
    a.insert(
        "snooze_count".into(),
        or_default(input, "snooze_count", json!(0)),
    );
    a.insert(
        "follow_up_manual".into(),
        or_default(input, "follow_up_manual", json!(false)),
    );
    a.insert(
        "description".into(),
        or_default(input, "description", Value::Null),
    );
    a.insert(
        "language".into(),
        or_default(input, "language", Value::Null),
    );
    a.insert(
        "evaluations".into(),
        or_default(input, "evaluations", json!({})),
    );
    a.insert(
        "eval_reasoning".into(),
        or_default(input, "eval_reasoning", json!({})),
    );
    a.insert("origins".into(), Value::Object(origins));
    a.insert(
        "files".into(),
        or_default(input, "files", json!({"cv": null, "cl": null, "misc": []})),
    );
    a.insert(
        "connections".into(),
        or_default(input, "connections", Value::Null),
    );
    a.insert(
        "company_context".into(),
        or_default(input, "company_context", Value::Null),
    );
    a.insert("notes".into(), or_default(input, "notes", Value::Null));
    a.insert(
        "format_overrides".into(),
        or_default(input, "format_overrides", Value::Null),
    );
    a.insert(
        "chain_overrides".into(),
        or_default(input, "chain_overrides", Value::Null),
    );
    a.insert(
        "created_at".into(),
        or_default(input, "created_at", json!(ctx.now)),
    );
    a.insert(
        "updated_at".into(),
        or_default(input, "updated_at", json!(ctx.now)),
    );
    Value::Object(a)
}

/// The typed `job.create` ops for converted jobs, in batches of at most 50 as `bulkCreateJobs`
/// commits them: `{"kind": "batch", "ops": [{"op": {"kind": "job.create", ...}}, ...]}`.
/// `new_local_id` mints the random tail of a workspace-local URL for jobs without a URL.
pub fn job_create_batches(
    jobs: &[Map<String, Value>],
    ctx: &OpContext<'_>,
    mut new_local_id: impl FnMut() -> String,
) -> Vec<Value> {
    let creates: Vec<Value> = jobs
        .iter()
        .map(|job| {
            let url = match job.get("url") {
                Some(Value::String(u)) if !js_trim(u).is_empty() => js_trim(u).to_owned(),
                _ => format!("careervector-local://{}/{}", ctx.workspace_id, new_local_id()),
            };
            json!({ "op": { "kind": "job.create", "url": url, "attributes": job_create_attributes(job, ctx) } })
        })
        .collect();
    creates
        .chunks(50)
        .map(|chunk| json!({ "kind": "batch", "ops": chunk }))
        .collect()
}
