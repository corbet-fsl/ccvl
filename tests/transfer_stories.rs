//! User stories of the jobs CSV transfer that deliberately differ from the old `SheetJS` reading.
//! Each test is one sentence a person would say; the parity suite (`transfer.rs`) covers the rest.

use ccvl::transfer::{
    ImportContext, ImportPreview, Layout, Settings, export_csv, parse_import_csv,
    process_import_data, read_rows,
};
use serde_json::{Map, Value, json};

fn settings() -> Settings {
    serde_json::from_value(json!({
        "kanban": {"phases": [], "statuses": [{"id": 1, "name": "Applied", "phaseId": 1, "order": 1}]},
        "customColumns": [{"id": "remote", "label": "Remote"}, {"id": "fit", "label": "Fit"}]
    }))
    .unwrap()
}

fn import(bytes: &[u8]) -> (ImportPreview, Vec<Map<String, Value>>) {
    let settings = settings();
    let preview = parse_import_csv(bytes, &settings).unwrap();
    // The host hands the previewed rows back as JSON: go through it, as the web does.
    let rows: Vec<ccvl::transfer::ImportRow> =
        serde_json::from_value(serde_json::to_value(&preview.raw_data).unwrap()).unwrap();
    let jobs = process_import_data(
        &rows,
        &preview.columns,
        &settings,
        &ImportContext {
            today: "2026-11-17",
        },
        || "x".into(),
    );
    (preview, jobs)
}

#[test]
fn a_file_from_another_tool_without_a_bom_keeps_its_umlauts() {
    let (_, jobs) = import("Job,Organization,Location\nIngenieur,Müller AG,Zürich\n".as_bytes());
    assert_eq!(jobs[0]["organization"], "Müller AG");
    assert_eq!(jobs[0]["location"], "Zürich");
}

#[test]
fn a_bom_wins_over_guessing() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("Job,Location\nA,Genève\n".as_bytes());
    let (_, jobs) = import(&bytes);
    assert_eq!(jobs[0]["location"], "Genève");
    assert_eq!(jobs[0]["title"], "A");
    // the mark itself is not part of the first header
    assert_eq!(read_rows(&bytes).unwrap()[0].keys().next(), Some("Job"));
}

#[test]
fn a_windows_file_is_read_as_windows_1252_when_it_is_not_utf8() {
    // "Müller € – Zürich" in Windows-1252: ü 0xFC, € 0x80, en dash 0x96
    let bytes = b"Job,Organization\nA,M\xFCller \x80 \x96 Z\xFCrich\n";
    assert!(String::from_utf8(bytes.to_vec()).is_err());
    let (_, jobs) = import(bytes);
    assert_eq!(jobs[0]["organization"], "Müller € – Zürich");
}

#[test]
fn a_utf8_character_is_never_split_into_latin_1_pairs() {
    let (_, jobs) = import("Job,Organization\nA,Ã¼ and ü\n".as_bytes());
    assert_eq!(jobs[0]["organization"], "Ã¼ and ü");
}

#[test]
fn cell_text_arrives_as_typed_in_text_columns() {
    let csv = "Job,Organization,Location,Commute\n007,80%,\"1,2\",7.50\n";
    let (preview, jobs) = import(csv.as_bytes());
    assert_eq!(
        preview.preview_data[0]
            .get("Job")
            .map(|c| serde_json::to_value(c).unwrap()),
        Some(json!("007"))
    );
    assert_eq!(jobs[0]["title"], "007");
    assert_eq!(jobs[0]["organization"], "80%");
    assert_eq!(jobs[0]["location"], "1,2");
    assert_eq!(jobs[0]["commute"], "7.50");
}

#[test]
fn plain_numbers_stay_numbers_in_the_preview() {
    let (preview, _) = import(b"Job,Score\nA,42\nB,7.5\n");
    let rows = serde_json::to_value(&preview.raw_data).unwrap();
    assert_eq!(rows[0]["Score"].as_f64(), Some(42.0));
    assert_eq!(rows[1]["Score"].as_f64(), Some(7.5));
}

#[test]
fn numeric_columns_keep_their_parsing() {
    let (_, jobs) = import(b"Job,Workload,Salary,Score\nA,80%,\"$95,000\",\"1,2\"\n");
    // workload and score keep SheetJS's reading (80% is 0.8, 1,2 is 12); the salary text is read by
    // the salary rules downstream
    assert_eq!(jobs[0]["workload"], "0.8");
    assert_eq!(jobs[0]["score"], "12");
    assert_eq!(jobs[0]["salary"], "95000");
}

#[test]
fn custom_evaluation_columns_keep_their_parsing() {
    let (_, jobs) = import(b"Job,Remote,Fit\nA,Yes,80%\nB,nein,\"1,5\"\n");
    assert_eq!(jobs[0]["evaluations"]["remote"], json!(true));
    assert_eq!(jobs[0]["evaluations"]["fit"], json!(0.8));
    assert_eq!(jobs[1]["evaluations"]["remote"], json!(false));
    assert_eq!(jobs[1]["evaluations"]["fit"], json!(15));
}

#[test]
fn my_own_export_imports_with_the_dates_and_text_as_written() {
    let settings = settings();
    let jobs = vec![
        json!({"id": "a", "title": "007 Agent", "organization": "Müller AG", "location": "Zürich",
               "source": "x", "applied_at": "2026-11-17", "follow_up_at": "17.11.2026",
               "status_id": 1, "commute": "80%"}),
        json!({"id": "b", "title": "1,2", "organization": "Café – Genève", "location": "Bern",
               "source": "y", "applied_at": "2026-01-05", "follow_up_at": "01/02/2026",
               "status_id": 1, "commute": "007"}),
    ];
    let ids: Vec<String> = [
        "title",
        "organization",
        "location",
        "source",
        "commute",
        "status",
        "applied_at",
        "follow_up_at",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let bytes = export_csv(&jobs, &ids, &settings, &Layout::default());
    let (_, imported) = import(&bytes);
    assert_eq!(imported.len(), 2);
    for (job, original) in imported.iter().zip(&jobs) {
        for field in [
            "title",
            "organization",
            "location",
            "source",
            "commute",
            "applied_at",
            "follow_up_at",
        ] {
            assert_eq!(job[field], original[field], "{field}");
        }
    }
}

#[test]
fn a_second_round_trip_changes_nothing() {
    let settings = settings();
    let jobs = vec![
        json!({"id": "a", "title": "007", "organization": "Müller", "location": "Zürich",
        "source": "x", "applied_at": "2026-11-17", "follow_up_at": "2026-11-24", "status_id": 1}),
    ];
    let ids: Vec<String> = [
        "title",
        "organization",
        "location",
        "source",
        "status",
        "applied_at",
        "follow_up_at",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let first = export_csv(&jobs, &ids, &settings, &Layout::default());
    let (_, imported) = import(&first);
    let again: Vec<Value> = imported.into_iter().map(Value::Object).collect();
    let second = export_csv(&again, &ids, &settings, &Layout::default());
    assert_eq!(first, second);
}

#[test]
fn previewed_rows_survive_the_trip_through_json() {
    use ccvl::transfer::{ImportCell, ImportColumnMapping};
    let row: ccvl::transfer::ImportRow = serde_json::from_value(
        json!({"t": "text", "n": 42, "u": 18_446_744_073_709_551_615u64, "i": -7, "f": 7.5, "b": true, "z": null}),
    )
    .unwrap();
    let get = |k: &str| row.get(k).unwrap().clone();
    assert_eq!(get("t"), ImportCell::Text("text".into()));
    assert_eq!(get("n"), ImportCell::Number(42.0));
    assert_eq!(get("i"), ImportCell::Number(-7.0));
    assert_eq!(get("f"), ImportCell::Number(7.5));
    assert_eq!(get("b"), ImportCell::Bool(true));
    assert_eq!(get("z"), ImportCell::Null);
    assert!(matches!(get("u"), ImportCell::Number(n) if n > 1.8e19));
    assert_eq!(
        row.keys().collect::<Vec<_>>(),
        ["t", "n", "u", "i", "f", "b", "z"]
    );
    // a cell that is an object or array is not a spreadsheet cell
    assert_eq!(
        serde_json::from_str::<ImportCell>("\"typed\"").unwrap(),
        ImportCell::Text("typed".into())
    );
    let refused = serde_json::from_value::<ImportCell>(json!({"a": 1})).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("a string, number, boolean or null"),
        "{refused}"
    );
    let refused = serde_json::from_value::<ccvl::transfer::ImportRow>(json!(5)).unwrap_err();
    assert!(
        refused.to_string().contains("header names to cells"),
        "{refused}"
    );
    assert!(serde_json::from_value::<ccvl::transfer::ImportRow>(json!([1, 2])).is_err());
    // a mapping without isCustom reads as "not custom"
    let m: ImportColumnMapping = serde_json::from_value(
        json!({"fileHeader": "Job", "targetColumnId": "title", "confidence": 1.0}),
    )
    .unwrap();
    assert_eq!(m.is_custom, None);
}

#[test]
fn every_windows_1252_byte_reads_as_its_character() {
    // 0x80..=0x9F differ from Latin-1: a real euro sign, quotes and dashes instead of C1 controls
    let expected = "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8D}Ž\u{8F}\u{90}‘’“”•–—˜™š›œ\u{9D}žŸ";
    let mut bytes = b"Job,Organization\nA,".to_vec();
    bytes.extend(0x80u8..=0x9F);
    bytes.push(b'\n');
    let rows = read_rows(&bytes).unwrap();
    let Some(ccvl::transfer::ImportCell::Text(text)) = rows[0].get("Organization") else {
        panic!("not text")
    };
    assert_eq!(text, expected);
}

#[test]
fn bool_and_error_cells_keep_their_spreadsheet_meaning() {
    let (preview, jobs) = import(b"Job,Remote,Fit\nA,TRUE,#NULL!\nB,FALSE,#DIV/0!\n");
    let rows = serde_json::to_value(&preview.raw_data).unwrap();
    assert_eq!(rows[0]["Remote"], json!(true));
    assert_eq!(rows[0]["Fit"], Value::Null);
    assert!(rows[1].get("Fit").is_none(), "an error cell has no key");
    assert_eq!(jobs[0]["evaluations"]["remote"], json!(true));
    assert_eq!(jobs[1]["evaluations"]["remote"], json!(false));
}

#[test]
fn the_refusals_say_what_the_web_says() {
    use ccvl::transfer::ImportError;
    assert_eq!(ImportError::Empty.to_string(), "File appears to be empty");
    assert_eq!(
        ImportError::UnsupportedFormat.to_string(),
        "The file is not a CSV file"
    );
}
