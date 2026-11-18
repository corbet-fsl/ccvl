//! Spreadsheet imports (`.xlsx`, `.xls`) follow the CSV rules: dates as displayed, cell text as typed,
//! numeric columns parse. The fixtures: `jobs.xlsx` and `jobs.xls` are `LibreOffice` exports of
//! `jobs.fods`; `excel-style.xlsx` is built the way Excel stores a workbook (shared strings, serial
//! dates with numFmt 14 and a custom format, a percent format, booleans).

use std::path::PathBuf;

use ccvl::transfer::{
    ImportContext, ImportError, Settings, parse_import_csv, process_import_data, read_rows,
};
use serde_json::{Value, json};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transfer/sheets")
        .join(name);
    std::fs::read(path).unwrap()
}

fn jobs_of(name: &str) -> Vec<serde_json::Map<String, Value>> {
    let settings: Settings = serde_json::from_value(json!({
        "kanban": {"phases": [], "statuses": [{"id": 1, "name": "Applied", "phaseId": 1, "order": 1}]},
        "customColumns": [{"id": "remote", "label": "Remote"}]
    }))
    .unwrap();
    let preview = parse_import_csv(&fixture(name), &settings).unwrap();
    process_import_data(
        &preview.raw_data,
        &preview.columns,
        &settings,
        &ImportContext {
            today: "2026-11-18",
        },
        || "x".into(),
    )
}

#[test]
fn a_libreoffice_workbook_reads_with_dates_as_displayed_and_text_as_typed() {
    for name in ["jobs.xlsx", "jobs.xls"] {
        let rows = serde_json::to_value(read_rows(&fixture(name)).unwrap()).unwrap();
        assert_eq!(rows[0]["Job"], "007 Agent", "{name}");
        assert_eq!(rows[0]["Organization"], "Müller AG", "{name}");
        assert_eq!(
            rows[0]["Last Action"], "2026-11-17",
            "{name}: a date cell, not 46343"
        );
        assert_eq!(
            rows[1]["Last Action"], "2026-01-05 10:30",
            "{name}: a time shows"
        );
        assert_eq!(rows[0]["Commute"], "1,2", "{name}: text cells stay text");
        assert_eq!(rows[1]["Commute"], "80%", "{name}");
        assert_eq!(rows[0]["Salary"].as_f64(), Some(95000.0), "{name}");
    }
}

#[test]
fn an_excel_style_workbook_reads_dates_percent_and_booleans() {
    let rows = serde_json::to_value(read_rows(&fixture("excel-style.xlsx")).unwrap()).unwrap();
    assert_eq!(rows[0]["Last Action"], "2026-11-17");
    assert_eq!(
        rows[0]["Next Action"], "2026-01-05 12:00",
        "a custom date-time format"
    );
    assert_eq!(rows[1]["Last Action"], "2026-12-24");
    assert_eq!(rows[0]["Remote"], json!(true));
    assert_eq!(rows[1]["Remote"], json!(false));
    assert_eq!(rows[1].get("Next Action"), None, "an empty cell has no key");
}

#[test]
fn a_workload_percent_cell_imports_as_the_percentage_it_shows() {
    // Excel stores 80% as 0.8 and 100% as 1; a range typed as text is its first number
    let excel = jobs_of("excel-style.xlsx");
    assert_eq!(excel[0]["workload"], "80");
    assert_eq!(excel[1]["workload"], "100");
    for name in ["jobs.xlsx", "jobs.xls"] {
        let jobs = jobs_of(name);
        assert_eq!(jobs[0]["workload"], "80", "{name}");
        assert_eq!(jobs[1]["workload"], "80", "{name}: 80–100%");
        assert_eq!(jobs[0]["commute"], "1,2", "{name}");
    }
}

#[test]
fn the_workbook_dates_become_job_dates_through_the_mapping() {
    let jobs = jobs_of("jobs.xlsx");
    // "Last Action" maps to applied_at by its label
    assert_eq!(jobs[0]["applied_at"], "2026-11-17");
    assert_eq!(jobs[0]["salary"], "95000");
}

#[test]
fn a_file_that_is_not_a_workbook_is_refused() {
    assert_eq!(
        read_rows(b"PK\x03\x04 not a workbook").unwrap_err(),
        ImportError::UnsupportedFormat
    );
    assert_eq!(
        read_rows(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0]).unwrap_err(),
        ImportError::UnsupportedFormat
    );
}

#[test]
fn a_date_stored_as_iso_text_reads_like_a_date_cell() {
    // not reachable through the fixtures (Excel never writes ISO date cells), so through the helper's contract
    use ccvl::transfer::sheet::iso_text;
    assert_eq!(iso_text("2026-11-17"), "2026-11-17");
    assert_eq!(iso_text("2026-11-17T00:00:00Z"), "2026-11-17");
    assert_eq!(iso_text("2026-11-17T10:30:00.000Z"), "2026-11-17 10:30");
    assert_eq!(iso_text("2026-11-17 10:30:15"), "2026-11-17 10:30:15");
    assert_eq!(iso_text("2026-11-17T09"), "2026-11-17");
    assert_eq!(iso_text("not a date"), "not a date");
    assert_eq!(iso_text("17.11.2026"), "17.11.2026");
}

#[test]
fn every_cell_kind_of_a_workbook_has_a_reading() {
    let rows = serde_json::to_value(read_rows(&fixture("edge.xlsx")).unwrap()).unwrap();
    assert_eq!(rows[0]["Num"].as_f64(), Some(7.0), "an integer cell");
    assert_eq!(rows[0]["Flag"], json!(true));
    assert_eq!(rows[0].get("Err"), None, "#DIV/0! has no key");
    assert_eq!(rows[0]["Null"], Value::Null, "#NULL! reads as null");
    assert_eq!(
        rows[0]["Duration"], "36:00:00",
        "[h]:mm:ss is the total time"
    );
    assert_eq!(
        rows[0].get("Blank"),
        None,
        "an empty string cell has no key"
    );
    assert_eq!(rows[1]["Num"], "text", "an inline string");
    assert_eq!(rows[1]["Flag"], json!(false));
    assert_eq!(rows[1]["Duration"], "-6:00:00", "a negative duration");
    assert_eq!(rows[1]["Blank"], "kept");
}

#[test]
fn a_workbook_without_data_is_empty() {
    assert_eq!(read_rows(&fixture("empty.xlsx")).unwrap().len(), 0);
    let settings = Settings::default();
    assert_eq!(
        parse_import_csv(&fixture("empty.xlsx"), &settings).unwrap_err(),
        ImportError::Empty
    );
}
