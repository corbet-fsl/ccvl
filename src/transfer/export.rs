//! Jobs to CSV: the column selection of `prepareExportData` and the byte stream `SheetJS` writes for
//! `exportToExcel(..., 'csv')`.

use serde::Serialize;
use serde_json::Value;

use crate::transfer::columns::{column_config, is_special_group};
use crate::transfer::jsnum::{general_number, order_properties};
use crate::transfer::settings::{Layout, Settings, same_id};

/// A spreadsheet cell as `json_to_sheet` stores it.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Text(String),
    Number(f64),
    Bool(bool),
    /// No cell is stored (arrays and objects are skipped by `SheetJS`).
    Blank,
}

impl Cell {
    fn text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Number(n) => general_number(*n),
            Self::Bool(true) => "TRUE".to_owned(),
            Self::Bool(false) => "FALSE".to_owned(),
            Self::Blank => String::new(),
        }
    }
}

/// One selected export column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportColumn {
    pub id: String,
    pub label: String,
    #[serde(rename = "isCustom", skip_serializing_if = "Option::is_none")]
    pub is_custom: Option<bool>,
}

/// The selected columns and one row per job, keyed by label in JavaScript property order.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportData {
    pub columns: Vec<ExportColumn>,
    pub rows: Vec<Vec<(String, Cell)>>,
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn cell_of(v: &Value) -> Cell {
    match v {
        Value::Null => Cell::Text(String::new()),
        Value::Bool(b) => Cell::Bool(*b),
        Value::Number(n) => Cell::Number(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Cell::Text(s.clone()),
        Value::Array(_) | Value::Object(_) => Cell::Blank,
    }
}

fn replace_first_eval(id: &str) -> String {
    id.replacen("eval_", "", 1)
}

/// `prepareExportData`: choose and label the columns, then map every job to a row.
#[must_use]
pub fn prepare_export_data(
    jobs: &[Value],
    ordered_column_ids: &[String],
    settings: &Settings,
    layout: &Layout,
) -> ExportData {
    let mut columns: Vec<ExportColumn> = Vec::new();
    let category = |job: &Value| job.get("categoryName").filter(|v| truthy(v)).cloned();
    if jobs.iter().any(|j| category(j).is_some()) {
        columns.push(ExportColumn {
            id: "categoryName".into(),
            label: "Category".into(),
            is_custom: None,
        });
    }
    for col_id in ordered_column_ids {
        let config = column_config(col_id, layout.column_layout.as_ref());
        if is_special_group(&config.super_header) || config.super_header == "Files" {
            continue;
        }
        let visibility_key = if col_id.starts_with("eval_") {
            replace_first_eval(col_id)
        } else {
            col_id.clone()
        };
        if layout
            .column_visibility
            .as_ref()
            .and_then(|m| m.get(&visibility_key))
            .is_some_and(|v| *v == Value::Bool(false))
        {
            continue;
        }
        let mut label = config.label;
        let mut is_custom = false;
        if col_id.starts_with("eval_") {
            is_custom = true;
            let raw_id = replace_first_eval(col_id);
            if let Some(def) = settings.custom_columns.iter().find(|c| c.id == raw_id) {
                label.clone_from(&def.label);
            }
        }
        columns.push(ExportColumn {
            id: col_id.clone(),
            label,
            is_custom: Some(is_custom),
        });
    }

    let rows = jobs
        .iter()
        .map(|job| {
            let entries = columns
                .iter()
                .map(|col| {
                    let value: Value = if col.id == "categoryName" {
                        category(job).unwrap_or_else(|| Value::String(String::new()))
                    } else if col.is_custom == Some(true) {
                        let raw_id = replace_first_eval(&col.id);
                        match job.get("evaluations").and_then(|e| e.get(&raw_id)) {
                            Some(Value::Bool(b)) => {
                                Value::String(if *b { "Yes" } else { "No" }.into())
                            }
                            Some(v) => v.clone(),
                            None => Value::Null,
                        }
                    } else if col.id == "status" {
                        let id = job.get("status_id").cloned().unwrap_or(Value::Null);
                        let name = settings
                            .kanban
                            .statuses
                            .iter()
                            .find(|s| same_id(&s.id, &id))
                            .map(|s| s.name.clone())
                            .filter(|n| !n.is_empty());
                        Value::String(name.unwrap_or_default())
                    } else {
                        job.get(&col.id).cloned().unwrap_or(Value::Null)
                    };
                    (col.label.clone(), cell_of(&value))
                })
                .collect();
            order_properties(entries)
        })
        .collect();
    ExportData { columns, rows }
}

fn quote_if_needed(text: String) -> String {
    if text.contains([',', '\n', '"']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// The text of the worksheet `json_to_sheet` builds, as `sheet_to_csv` writes it (no byte-order
/// mark).
#[must_use]
pub fn csv_text(data: &ExportData) -> String {
    let Some(first) = data.rows.first() else {
        return String::new();
    };
    let header: Vec<&str> = first.iter().map(|(k, _)| k.as_str()).collect();
    let mut lines: Vec<String> = Vec::with_capacity(data.rows.len() + 1);
    let head: Vec<String> = header
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let text = quote_if_needed((*h).to_owned());
            // `SheetJS` quotes a leading "ID" so spreadsheet programs do not read the file as SYLK.
            if i == 0 && text == "ID" {
                "\"ID\"".to_owned()
            } else {
                text
            }
        })
        .collect();
    lines.push(head.join(","));
    for row in &data.rows {
        let cells: Vec<String> = header
            .iter()
            .map(|h| {
                row.iter()
                    .find(|(k, _)| k == h)
                    .map_or_else(String::new, |(_, c)| quote_if_needed(c.text()))
            })
            .collect();
        lines.push(cells.join(","));
    }
    lines.join("\n")
}

/// The bytes of the downloaded file: a UTF-8 byte-order mark and the CSV text.
#[must_use]
pub fn csv_bytes(data: &ExportData) -> Vec<u8> {
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend_from_slice(csv_text(data).as_bytes());
    out
}

/// Export jobs to the bytes `exportToExcel(..., 'csv')` writes.
#[must_use]
pub fn export_csv(
    jobs: &[Value],
    ordered_column_ids: &[String],
    settings: &Settings,
    layout: &Layout,
) -> Vec<u8> {
    csv_bytes(&prepare_export_data(
        jobs,
        ordered_column_ids,
        settings,
        layout,
    ))
}
