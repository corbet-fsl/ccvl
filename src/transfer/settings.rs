//! The slice of workspace settings and layout that CSV export and import read.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A user-defined evaluation (scorecard) column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomColumn {
    pub id: String,
    pub label: String,
}

/// A kanban phase; only the ordering is read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Phase {
    pub id: Value,
    #[serde(default)]
    pub order: f64,
}

/// A kanban status.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub id: Value,
    pub name: String,
    #[serde(default, rename = "phaseId")]
    pub phase_id: Value,
    #[serde(default)]
    pub order: f64,
}

/// The kanban configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Kanban {
    #[serde(default)]
    pub phases: Vec<Phase>,
    #[serde(default)]
    pub statuses: Vec<Status>,
}

/// The workspace settings the transfer reads (`customColumns`, `kanban`); other fields are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "customColumns")]
    pub custom_columns: Vec<CustomColumn>,
    #[serde(default)]
    pub kanban: Kanban,
}

/// The workspace layout the export reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// Per-column overrides (`label`, `subHeader`, `superHeader`), by column id.
    #[serde(default, rename = "columnLayout")]
    pub column_layout: Option<Map<String, Value>>,
    /// Column visibility by column id; only an explicit `false` hides a column.
    #[serde(default, rename = "columnVisibility")]
    pub column_visibility: Option<Map<String, Value>>,
}

/// `SameValueZero`/`===` on ids that are numbers or strings.
pub(crate) fn same_id(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

impl Settings {
    /// `getSortedStatuses`: statuses by phase order, then status order (stable).
    #[must_use]
    pub fn sorted_statuses(&self) -> Vec<&Status> {
        let phase_order = |id: &Value| {
            self.kanban
                .phases
                .iter()
                .rev()
                .find(|p| same_id(&p.id, id))
                .map_or(0.0, |p| p.order)
        };
        let mut out: Vec<&Status> = self.kanban.statuses.iter().collect();
        out.sort_by(|a, b| {
            let (pa, pb) = (phase_order(&a.phase_id), phase_order(&b.phase_id));
            if pa != pb {
                return pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal);
            }
            a.order
                .partial_cmp(&b.order)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }
}
