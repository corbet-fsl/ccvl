//! Jobs CSV export and import: the dashboard's spreadsheet transfer, hosted in the engine.
//!
//! Export turns the jobs projection into the bytes the web downloads today; import turns a CSV file
//! into a mapping preview and, once the mapping is confirmed, into typed `job.create` ops (never raw
//! Yjs updates). The module has no I/O and no clock, so it builds natively and for `wasm32`: hosts
//! pass the date, the instant and an id source in.
//!
//! The reference is the TypeScript in `lib/domain/src/{export,import}.ts` of the `CareerVector`
//! monorepo and the `SheetJS` CSV reader and writer it calls; `tests/transfer.rs` proves byte and op
//! parity against fixtures that the TypeScript produces.

// Float equality in this module is deliberate: it reproduces JavaScript's `===`/`!=` on numbers.
#![allow(clippy::float_cmp)]

pub mod columns;
pub mod csvread;
pub mod export;
pub mod import;
#[doc(hidden)]
pub mod jsnum;
pub mod settings;
#[cfg(feature = "sheet")]
pub mod sheet;

pub use columns::{ColumnDef, DEFAULT_COLUMNS, column_config, default_column, is_special_group};
pub use csvread::{ImportCell, ImportRow, is_workbook, read_rows, read_rows_text};
pub use export::{
    Cell, ExportColumn, ExportData, csv_bytes, csv_text, export_csv, prepare_export_data,
};
pub use import::{
    EXTRA_IMPORT_FIELDS, ImportColumnMapping, ImportContext, ImportPreview, OpContext,
    job_create_batches, parse_import_csv, parse_import_rows, process_import_data,
};
pub use settings::{CustomColumn, Kanban, Layout, Phase, Settings, Status};

/// Why a file could not be read for import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportError {
    /// The file has no data rows (the web shows "File appears to be empty").
    Empty,
    /// The file is not CSV text (a spreadsheet, XML, SYLK, ...); only CSV is read here.
    UnsupportedFormat,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("File appears to be empty"),
            Self::UnsupportedFormat => f.write_str("The file is not a CSV file"),
        }
    }
}

impl std::error::Error for ImportError {}
