//! Spreadsheet files (`.xlsx`, `.xls`) for the jobs import, read with `calamine` (pure Rust, builds for
//! `wasm32`). They follow the same three rules as CSV (see [`crate::transfer::csvread`]):
//!
//! - **Dates as displayed.** A cell Excel holds as a date is a number plus a date format; it reads as
//!   its ISO date (`2026-11-17`, with `HH:MM[:SS]` when the cell carries a time), never as the serial
//!   number `SheetJS` handed over.
//! - **Cell text as typed.** A text cell stays text (`80%`, `007`, `1,2`); a number cell is a number.
//! - **Numeric columns parse.** The conversion into jobs is the one the CSV uses.
//!
//! Only the first sheet is read, its first used row is the header row. Formulas read as their cached
//! value; error cells other than `#NULL!` have no key, as in CSV.

use std::collections::HashMap;
use std::io::Cursor;

use calamine::{CellErrorType, Data, Reader, open_workbook_auto_from_rs};

use crate::transfer::ImportError;
use crate::transfer::csvread::{ImportCell, ImportRow, Stored, Typed, rows_from_grid};
use crate::transfer::jsnum::number_to_string;

/// Whether the bytes start like a workbook: a ZIP container (`.xlsx`) or an OLE2 compound file (`.xls`).
#[must_use]
pub fn is_spreadsheet(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

/// The ISO text of an Excel date: `YYYY-MM-DD`, plus `HH:MM` (or `:SS`) when the value has a time.
fn iso(ymd_hms_milli: (u16, u8, u8, u8, u8, u8, u16)) -> String {
    let (y, mo, d, h, mi, s, _) = ymd_hms_milli;
    let date = format!("{y:04}-{mo:02}-{d:02}");
    match (h, mi, s) {
        (0, 0, 0) => date,
        (h, mi, 0) => format!("{date} {h:02}:{mi:02}"),
        (h, mi, s) => format!("{date} {h:02}:{mi:02}:{s:02}"),
    }
}

fn typed(cell: &Data) -> (String, Typed) {
    match cell {
        Data::Empty => (String::new(), Typed::Empty),
        Data::String(s) if s.is_empty() => (String::new(), Typed::Empty),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => {
            (s.clone(), Typed::Value(ImportCell::Text(s.clone())))
        }
        Data::Float(f) => (number_to_string(*f), Typed::Value(ImportCell::Number(*f))),
        #[allow(clippy::cast_precision_loss)]
        Data::Int(i) => (i.to_string(), Typed::Value(ImportCell::Number(*i as f64))),
        Data::Bool(b) => (
            if *b { "TRUE" } else { "FALSE" }.to_owned(),
            Typed::Value(ImportCell::Bool(*b)),
        ),
        Data::DateTime(dt) => {
            let text = if dt.is_datetime() {
                iso(dt.to_ymd_hms_milli())
            } else {
                // A duration ([h]:mm:ss): total hours, minutes and seconds.
                let total = (dt.as_f64() * 86_400.0).round();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let secs = total.abs() as u64;
                format!(
                    "{}{}:{:02}:{:02}",
                    if total < 0.0 { "-" } else { "" },
                    secs / 3600,
                    secs % 3600 / 60,
                    secs % 60
                )
            };
            (text.clone(), Typed::Value(ImportCell::Text(text)))
        }
        Data::Error(CellErrorType::Null) => ("#NULL!".to_owned(), Typed::Value(ImportCell::Null)),
        Data::Error(e) => (e.to_string(), Typed::Skipped),
    }
}

/// The rows of the first sheet, as `sheet_to_json` would return them (header row, one object per row).
pub fn read_rows(bytes: &[u8]) -> Result<Vec<ImportRow>, ImportError> {
    let mut book = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|_| ImportError::UnsupportedFormat)?;
    let range = match book.worksheet_range_at(0) {
        Some(Ok(range)) => range,
        Some(Err(_)) => return Err(ImportError::UnsupportedFormat),
        None => return Ok(Vec::new()),
    };
    let mut grid: HashMap<(usize, usize), Stored> = HashMap::new();
    let (mut max_row, mut max_col) = (0usize, 0usize);
    for (r, c, cell) in range.cells() {
        let (text, typed) = typed(cell);
        if matches!(typed, Typed::Empty) {
            continue;
        }
        max_row = max_row.max(r);
        max_col = max_col.max(c);
        grid.insert((r, c), Stored { text, typed });
    }
    if grid.is_empty() {
        return Ok(Vec::new());
    }
    Ok(rows_from_grid(&grid, max_row, max_col))
}
