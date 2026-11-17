//! The dashboard column catalogue the CSV transfer reads: ids, labels and header groups.

use serde_json::{Map, Value};

/// One column definition: the three fields export and import look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub label: String,
    pub sub_header: String,
    pub super_header: String,
}

/// The built-in columns in the order of `DEFAULT_COLUMNS` (id, label, sub header, super header).
/// A shared fixture compares this table with the TypeScript reference.
pub const DEFAULT_COLUMNS: &[(&str, &str, &str, &str)] = &[
    ("_category", "Row Count", "$count", "Sort"),
    ("_drag", "Drag Handle", "Drag", "Sort"),
    ("source", "Source", "Src | URL", "Opportunity"),
    ("title", "Job", "Job", "Opportunity"),
    ("organization", "Organization", "Org", "Opportunity"),
    ("industry", "Industry", "\u{1F3ED}", "Opportunity"),
    ("location", "Location", "\u{1F4CD}", "Opportunity"),
    ("workload", "Workload", "%", "Scorecard"),
    ("salary", "Salary", "\u{1F911}", "Scorecard"),
    ("commute", "Commute", "\u{1F68B}", "Scorecard"),
    ("score", "Score", "\u{00D8}", "Scorecard"),
    ("status", "Status", "Status", "Kanban"),
    ("applied_at", "Last Action", "Last", "Kanban"),
    ("follow_up_at", "Next Action", "Next", "Kanban"),
    ("cv", "Curriculum Vitae", "CV", "Files"),
    ("cl", "Cover Letter", "CL", "Files"),
    ("misc_files", "Miscellaneous", "Misc", "Files"),
    ("delete", "Delete", "Del", "Act"),
];

/// Header groups whose columns are not data (`SPECIAL_GROUPS`).
#[must_use]
pub fn is_special_group(super_header: &str) -> bool {
    matches!(super_header, "" | "Category" | "Sort" | "Act")
}

/// Look a built-in column up by id.
#[must_use]
pub fn default_column(id: &str) -> Option<ColumnDef> {
    DEFAULT_COLUMNS
        .iter()
        .find(|(i, ..)| *i == id)
        .map(|(_, label, sub, sup)| ColumnDef {
            label: (*label).to_owned(),
            sub_header: (*sub).to_owned(),
            super_header: (*sup).to_owned(),
        })
}

/// `getColumnConfig`: a layout override wins, then the built-ins, then the custom-column template
/// for `eval_*` ids, then the unknown-column fallback.
#[must_use]
pub fn column_config(id: &str, overrides: Option<&Map<String, Value>>) -> ColumnDef {
    if let Some(Value::Object(o)) = overrides.and_then(|m| m.get(id)) {
        let text = |key: &str| {
            o.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return ColumnDef {
            label: text("label"),
            sub_header: text("subHeader"),
            super_header: text("superHeader"),
        };
    }
    if let Some(def) = default_column(id) {
        return def;
    }
    if let Some(rest) = id.strip_prefix("eval_") {
        return ColumnDef {
            label: rest.to_owned(),
            sub_header: "Cst".to_owned(),
            super_header: "Scorecard".to_owned(),
        };
    }
    ColumnDef {
        label: id.to_owned(),
        sub_header: id.chars().take(3).collect::<String>().to_uppercase(),
        super_header: "Unknown".to_owned(),
    }
}
