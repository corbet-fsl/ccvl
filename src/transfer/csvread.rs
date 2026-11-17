//! A reader for the CSV files the web import accepts, reproducing what `SheetJS`
//! (`XLSX.read` + `sheet_to_json`) hands the TypeScript import: encoding rules, separator guess,
//! cell typing, header naming and blank-row handling.
//!
//! Three deliberate differences from `SheetJS`, each a user story rather than an accident:
//!
//! - Dates stay as written. `SheetJS` turns date-looking text into spreadsheet serial numbers; this
//!   reader keeps the text, so a file exported by the app imports with the same dates.
//! - Encoding. A byte-order mark selects UTF-8; without one the bytes are UTF-8 when they are valid
//!   UTF-8 (umlauts from another tool survive) and Windows-1252 otherwise. `SheetJS` read everything
//!   without a mark as Latin-1.
//! - Cells arrive as typed. A cell that reads as a number keeps its text unless the number prints
//!   as exactly that text: `80%`, `007`, `1,2` and `7.50` stay text, `42` and `7.5` become numbers.
//!   A column that wants the number (salary, score, custom evaluations) parses the text
//!   again with [`crate::transfer::jsnum::fuzzy_number`], which is what `SheetJS` did up front. A workload reads percentages: `80%` is 80, `80-100%` is 80.
//!
//! Its readers for non-CSV formats are not reproduced (such files are refused).

use std::collections::HashMap;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

use crate::transfer::ImportError;
use crate::transfer::jsnum::{fuzzy_number, js_trim, number_to_string, order_properties};

/// A value in a parsed row.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportCell {
    Text(String),
    Number(f64),
    Bool(bool),
    /// The `#NULL!` error cell, which `sheet_to_json` reports as `null`.
    Null,
}

impl Serialize for ImportCell {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(t) => s.serialize_str(t),
            Self::Number(n) if n.is_finite() => s.serialize_f64(*n),
            Self::Number(_) | Self::Null => s.serialize_none(),
            Self::Bool(b) => s.serialize_bool(*b),
        }
    }
}

impl<'de> Deserialize<'de> for ImportCell {
    /// The inverse of the serialization: a string is text, a number a number, `null` the `#NULL!`
    /// cell. This is how the host hands the previewed rows back for the conversion.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct CellVisitor;
        impl Visitor<'_> for CellVisitor {
            type Value = ImportCell;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string, number, boolean or null")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ImportCell, E> {
                Ok(ImportCell::Text(v.to_owned()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<ImportCell, E> {
                Ok(ImportCell::Text(v))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<ImportCell, E> {
                Ok(ImportCell::Bool(v))
            }
            #[allow(clippy::cast_precision_loss)]
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<ImportCell, E> {
                Ok(ImportCell::Number(v as f64))
            }
            #[allow(clippy::cast_precision_loss)]
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<ImportCell, E> {
                Ok(ImportCell::Number(v as f64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<ImportCell, E> {
                Ok(ImportCell::Number(v))
            }
            fn visit_unit<E: de::Error>(self) -> Result<ImportCell, E> {
                Ok(ImportCell::Null)
            }
        }
        d.deserialize_any(CellVisitor)
    }
}

/// One data row: header name to value, in JavaScript property order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportRow(pub Vec<(String, ImportCell)>);

impl ImportRow {
    /// The value under a header, `None` when the cell was empty.
    #[must_use]
    pub fn get(&self, header: &str) -> Option<&ImportCell> {
        self.0.iter().find(|(k, _)| k == header).map(|(_, v)| v)
    }

    /// The header names, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }
}

impl Serialize for ImportRow {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ImportRow {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct RowVisitor;
        impl<'de> Visitor<'de> for RowVisitor {
            type Value = ImportRow;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object of header names to cells")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ImportRow, A::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry::<String, ImportCell>()? {
                    entries.push(entry);
                }
                Ok(ImportRow(entries))
            }
        }
        d.deserialize_map(RowVisitor)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Typed {
    /// No cell stored.
    Empty,
    Value(ImportCell),
    /// An error cell other than `#NULL!`: no key in the row.
    Skipped,
}

#[derive(Debug, Clone)]
pub(crate) struct Stored {
    pub(crate) text: String,
    pub(crate) typed: Typed,
}

fn type_cell(raw: &str) -> Stored {
    let mut s = raw.to_owned();
    if s.ends_with('\r') {
        s.pop();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.first() == Some(&'"') && chars.last() == Some(&'"') {
        let inner: String = if chars.len() >= 2 {
            chars[1..chars.len() - 1].iter().collect()
        } else {
            String::new()
        };
        s = inner.replace("\"\"", "\"");
    }
    let text = s.clone();
    let typed = if s.is_empty() {
        Typed::Empty
    } else if js_trim(&s).is_empty() {
        Typed::Value(ImportCell::Text(s))
    } else if s.starts_with('=') {
        let chars: Vec<char> = s.chars().collect();
        if chars.get(1) == Some(&'"') && chars.last() == Some(&'"') && chars.len() >= 2 {
            let inner: String = chars[2..chars.len() - 1].iter().collect();
            Typed::Value(ImportCell::Text(inner.replace("\"\"", "\"")))
        } else {
            Typed::Value(ImportCell::Text(s))
        }
    } else if s == "TRUE" {
        Typed::Value(ImportCell::Bool(true))
    } else if s == "FALSE" {
        Typed::Value(ImportCell::Bool(false))
    } else {
        let n = fuzzy_number(&s);
        if n.is_nan() {
            match s.as_str() {
                "#NULL!" => Typed::Value(ImportCell::Null),
                "#DIV/0!" | "#VALUE!" | "#REF!" | "#NAME?" | "#NUM!" | "#N/A" | "#GETTING_DATA" => {
                    Typed::Skipped
                }
                _ => Typed::Value(ImportCell::Text(s)),
            }
        } else {
            // The number only when it prints as the typed text; otherwise the typed text itself.
            if number_to_string(n) == s {
                Typed::Value(ImportCell::Number(n))
            } else {
                Typed::Value(ImportCell::Text(s))
            }
        }
    };
    Stored { text, typed }
}

fn guess_separator(head: &[char]) -> char {
    let mut counts: [(char, u32, u32); 4] = [(',', 0, 3), ('\t', 0, 2), (';', 0, 1), ('|', 0, 0)];
    let mut in_str = false;
    for &c in head {
        if c == '"' {
            in_str = !in_str;
        } else if !in_str && let Some(slot) = counts.iter_mut().find(|(sep, ..)| *sep == c) {
            slot.1 += 1;
        }
    }
    let seen: Vec<&(char, u32, u32)> = counts.iter().filter(|(_, n, _)| *n > 0).collect();
    if seen.is_empty() {
        return ',';
    }
    seen.iter()
        .max_by_key(|(_, n, w)| (*n, *w))
        .map_or(',', |(c, ..)| *c)
}

/// Split the text into a sparse grid the way `dsv_to_sheet_str` does. Returns the cells keyed by
/// row and column plus the extent `(rows, columns)`.
fn split_grid(text: &str) -> (HashMap<(usize, usize), Stored>, usize, usize) {
    let mut chars: Vec<char> = text.chars().collect();
    let sep;
    if chars.len() >= 4 && chars[..4] == ['s', 'e', 'p', '='] {
        let c5 = chars.get(5).copied();
        let c6 = chars.get(6).copied();
        if c5 == Some('\r') && c6 == Some('\n') {
            sep = chars[4];
            chars.drain(..7);
        } else if c5 == Some('\r') || c5 == Some('\n') {
            sep = chars[4];
            chars.drain(..6);
        } else {
            sep = guess_separator(&chars[..chars.len().min(1024)]);
        }
    } else {
        sep = guess_separator(&chars[..chars.len().min(1024)]);
    }
    let mut grid = HashMap::new();
    let (mut row, mut col) = (0usize, 0usize);
    let (mut max_row, mut max_col) = (0usize, 0usize);
    let mut start = 0usize;
    let mut end = 0usize;
    let mut in_str = false;
    let mut start_char = chars.first().copied();
    let mut cc = '\0';
    let mut finish = |start: &mut usize, end: usize, cc: char, start_char: &mut Option<char>| {
        let raw: String = chars[*start..end].iter().collect();
        let cell = type_cell(&raw);
        if !matches!(cell.typed, Typed::Empty) {
            grid.insert((row, col), cell);
        }
        *start = end + 1;
        *start_char = chars.get(*start).copied();
        max_col = max_col.max(col);
        max_row = max_row.max(row);
        if cc == sep {
            col += 1;
        } else {
            col = 0;
            row += 1;
        }
    };
    while end < chars.len() {
        cc = chars[end];
        match cc {
            '"' => {
                if start_char == Some('"') {
                    in_str = !in_str;
                }
            }
            '\r' => {
                if !in_str {
                    if chars.get(end + 1) == Some(&'\n') {
                        end += 1;
                    }
                    finish(&mut start, end, cc, &mut start_char);
                }
            }
            c if (c == sep || c == '\n') && !in_str => {
                finish(&mut start, end, cc, &mut start_char);
            }
            _ => {}
        }
        end += 1;
    }
    if end > start {
        finish(&mut start, end, cc, &mut start_char);
    }
    (grid, max_row, max_col)
}

/// Bytes at the start of a file that `SheetJS` routes to a non-CSV reader.
fn sniffs_as_other_format(b: &[u8]) -> bool {
    let n = |i: usize| b.get(i).copied().unwrap_or(0);
    match n(0) {
        0xD0 => b.len() >= 8 && b[1..8] == [0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
        0x09 => n(1) <= 0x08,
        0x49 => (n(1) == 0x49 && n(2) == 0x2A && n(3) == 0) || n(1) == 0x44,
        0x54 => n(1) == 0x41 && n(2) == 0x42 && n(3) == 0x4C,
        0x50 => n(1) == 0x4B && n(2) < 0x09 && n(3) < 0x09,
        0xEF => n(3) == 0x3C,
        0xFF => n(1) == 0xFE || (n(1) == 0 && n(2) == 2 && n(3) == 0),
        0x00 => n(1) == 0 && ((n(2) >= 2 && n(3) == 0) || (n(2) == 0 && (n(3) == 8 || n(3) == 9))),
        0x7B => n(1) == 0x5C && n(2) == 0x72 && n(3) == 0x74,
        0x0A | 0x0D | 0x20 | 0x89 | 0x3C | 0x03 | 0x83 | 0x8B | 0x8C => true,
        0x08 => n(1) == 0xE7,
        0x0C => n(1) == 0xEC || n(1) == 0xED,
        0x02 | 0x30 | 0x31 | 0xF5 => n(2) <= 12 && n(3) <= 31,
        _ => false,
    }
}

/// Windows-1252 for the bytes 0x80 to 0x9F; the five bytes it leaves undefined map to the C1
/// control of the same value, as browsers do.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

/// Decode the file: a UTF-8 byte-order mark wins; without one the bytes are UTF-8 when they are
/// valid UTF-8 and Windows-1252 (a superset of Latin-1 for text) otherwise.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// Parse a file into the rows `sheet_to_json` returns for the first sheet: CSV text, or an `.xlsx` /
/// `.xls` workbook (see [`crate::transfer::sheet`]).
pub fn read_rows(bytes: &[u8]) -> Result<Vec<ImportRow>, ImportError> {
    if crate::transfer::sheet::is_spreadsheet(bytes) {
        return crate::transfer::sheet::read_rows(bytes);
    }
    if sniffs_as_other_format(bytes) {
        return Err(ImportError::UnsupportedFormat);
    }
    Ok(read_rows_text(&decode(bytes)))
}

/// As [`read_rows`] for text that is already decoded (a leading byte-order mark is kept, as `SheetJS`
/// does for string input).
#[must_use]
pub fn read_rows_text(text: &str) -> Vec<ImportRow> {
    let (grid, max_row, max_col) = split_grid(text);
    rows_from_grid(&grid, max_row, max_col)
}

/// The header naming and row assembly of `sheet_to_json`, over a sparse grid (row 0 is the header).
pub(crate) fn rows_from_grid(
    grid: &HashMap<(usize, usize), Stored>,
    max_row: usize,
    max_col: usize,
) -> Vec<ImportRow> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut headers: Vec<String> = Vec::with_capacity(max_col + 1);
    for c in 0..=max_col {
        let base = grid
            .get(&(0, c))
            .map_or_else(|| "__EMPTY".to_owned(), |s| s.text.clone());
        let mut name = base.clone();
        let mut counter = counts.get(&base).copied().unwrap_or(0);
        if counter == 0 {
            counts.insert(base, 1);
        } else {
            loop {
                name = format!("{base}_{counter}");
                counter += 1;
                if !counts.contains_key(&name) {
                    break;
                }
            }
            counts.insert(base, counter);
            counts.insert(name.clone(), 1);
        }
        headers.push(name);
    }
    let mut rows = Vec::new();
    for r in 1..=max_row {
        let mut entries: Vec<(String, ImportCell)> = Vec::new();
        let mut non_empty = false;
        for (c, header) in headers.iter().enumerate() {
            let Some(cell) = grid.get(&(r, c)) else {
                continue;
            };
            match &cell.typed {
                Typed::Empty | Typed::Skipped => {}
                Typed::Value(v) => {
                    if *v != ImportCell::Null {
                        non_empty = true;
                    }
                    entries.push((header.clone(), v.clone()));
                }
            }
        }
        if non_empty {
            rows.push(ImportRow(order_properties(entries)));
        }
    }
    rows
}
