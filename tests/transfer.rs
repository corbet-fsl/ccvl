//! Jobs CSV transfer: parity with the TypeScript reference. The fixtures under
//! `tests/fixtures/transfer` are produced by `tests/transfer-ts/gen.ts` from the `CareerVector`
//! monorepo's `export.ts` / `import.ts` running on `SheetJS`; every output here must match them.

use std::fs;
use std::path::{Path, PathBuf};

use ccvl::transfer::jsnum::{general_number, number_to_string};
use ccvl::transfer::{
    DEFAULT_COLUMNS, ImportColumnMapping, ImportContext, ImportError, ImportPreview, Layout,
    OpContext, Settings, default_column, export_csv, job_create_batches, parse_import_csv,
    prepare_export_data, process_import_data, read_rows_text,
};
use serde_json::{Number, Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transfer")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap()
}

fn case_names(dir: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures().join(dir))
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension()? == "json")
                .then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no fixtures in {dir}");
    names
}

/// Numbers compare by value: `1` and `1.0` are the same JSON number to JavaScript.
fn canon(v: &Value) -> Value {
    match v {
        Value::Number(n) => {
            let f = n.as_f64().unwrap();
            #[allow(clippy::cast_possible_truncation)]
            if f.fract() == 0.0 && f.abs() < 9.0e15 {
                Value::Number(Number::from(f as i64))
            } else {
                Value::Number(Number::from_f64(f).unwrap())
            }
        }
        Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), canon(v))).collect()),
        other => other.clone(),
    }
}

/// The fixture writer renders non-finite numbers as `{"nonFinite": ...}`; the engine has `null`.
fn denonfinite(v: &Value) -> Value {
    match v {
        Value::Object(o) if o.len() == 1 && o.contains_key("nonFinite") => Value::Null,
        Value::Array(a) => Value::Array(a.iter().map(denonfinite).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), denonfinite(v))).collect())
        }
        other => other.clone(),
    }
}

/// Assert equality and name the first differing path instead of dumping both documents.
fn assert_same(ours: &Value, theirs: &Value, context: &str) {
    fn first_diff(ours: &Value, theirs: &Value, path: &str) -> Option<String> {
        match (ours, theirs) {
            (Value::Object(x), Value::Object(y)) => {
                for (k, v) in x {
                    match y.get(k) {
                        Some(w) => {
                            if let Some(d) = first_diff(v, w, &format!("{path}.{k}")) {
                                return Some(d);
                            }
                        }
                        None => return Some(format!("{path}.{k}: only in ours: {v}")),
                    }
                }
                y.keys()
                    .find(|k| !x.contains_key(*k))
                    .map(|k| format!("{path}.{k}: only in reference: {}", y[k]))
            }
            (Value::Array(x), Value::Array(y)) => {
                for (i, (v, w)) in x.iter().zip(y).enumerate() {
                    if let Some(d) = first_diff(v, w, &format!("{path}[{i}]")) {
                        return Some(d);
                    }
                }
                (x.len() != y.len())
                    .then(|| format!("{path}: length {} vs reference {}", x.len(), y.len()))
            }
            _ => (ours != theirs).then(|| format!("{path}: ours {ours} vs reference {theirs}")),
        }
    }
    if let Some(diff) = first_diff(ours, theirs, "$") {
        panic!("{context}: {diff}");
    }
}

#[test]
fn default_columns_match_the_reference() {
    let expected = read_json(&fixtures().join("default-columns.json"));
    let ours: Vec<Value> = DEFAULT_COLUMNS
        .iter()
        .map(|(id, label, sub, sup)| json!([id, label, sub, sup]))
        .collect();
    assert_eq!(Value::Array(ours), expected);
    assert!(default_column("title").is_some() && default_column("nope").is_none());
}

#[test]
fn number_text_matches_javascript() {
    let table = read_json(&fixtures().join("numbers.json"));
    let mut checked = 0;
    for row in table.as_array().unwrap() {
        let bits = row["bits"].as_str().unwrap();
        let v: f64 = if bits == "-0" {
            -0.0
        } else {
            bits.parse().unwrap()
        };
        assert_eq!(
            number_to_string(v),
            row["string"].as_str().unwrap(),
            "String({bits})"
        );
        assert_eq!(
            general_number(v),
            row["general"].as_str().unwrap(),
            "General({bits})"
        );
        checked += 1;
    }
    assert!(checked > 500);
}

#[test]
fn cell_typing_matches_sheetjs() {
    let table = read_json(&fixtures().join("cells.json"));
    let mut checked = 0;
    for case in table["typed"].as_array().unwrap() {
        let csv = case["csv"].as_str().unwrap();
        let ours = serde_json::to_value(read_rows_text(csv)).unwrap();
        assert_same(
            &canon(&ours),
            &canon(&denonfinite(&case["rows"])),
            &format!("csv {csv:?}"),
        );
        checked += 1;
    }
    assert!(checked > 100);
}

fn settings_of(v: &Value) -> Settings {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn export_matches_the_reference_byte_for_byte() {
    for name in case_names("export") {
        let case = read_json(&fixtures().join(format!("export/{name}.json")));
        let jobs = case["jobs"].as_array().unwrap();
        let ids: Vec<String> = case["orderedColumnIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let settings = settings_of(&case["settings"]);
        let layout: Layout = serde_json::from_value(case["layout"].clone()).unwrap();
        let expected = fs::read(fixtures().join(format!("export/{name}.csv"))).unwrap();
        let ours = export_csv(jobs, &ids, &settings, &layout);
        assert_eq!(
            String::from_utf8_lossy(&ours),
            String::from_utf8_lossy(&expected),
            "export case {name}"
        );
        assert_eq!(ours, expected, "export case {name} (bytes)");
        let columns =
            serde_json::to_value(prepare_export_data(jobs, &ids, &settings, &layout).columns)
                .unwrap();
        assert_eq!(
            columns, case["expectedColumns"],
            "export case {name} (columns)"
        );
    }
}

fn apply_overrides(mappings: &mut [ImportColumnMapping], overrides: &Value) {
    for m in mappings {
        if let Some(o) = overrides.get(&m.file_header) {
            m.target_column_id = o["targetColumnId"].as_str().unwrap().to_owned();
            if let Some(c) = o.get("isCustom").and_then(Value::as_bool) {
                m.is_custom = Some(c);
            }
        }
    }
}

#[test]
fn import_matches_the_reference() {
    for name in case_names("import") {
        let case = read_json(&fixtures().join(format!("import/{name}.json")));
        let bytes = fs::read(fixtures().join(format!("import/{name}.csv"))).unwrap();
        let settings = settings_of(&case["settings"]);
        let preview = parse_import_csv(&bytes, &settings);
        if let Some(error) = case.get("error") {
            assert_eq!(error, "File appears to be empty", "{name}");
            assert_eq!(preview.unwrap_err(), ImportError::Empty, "{name}");
            continue;
        }
        let preview: ImportPreview = preview.unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = &case["expected"];
        assert_same(
            &canon(&serde_json::to_value(&preview).unwrap()),
            &canon(&denonfinite(&expected["preview"])),
            &format!("import case {name}: preview"),
        );

        let mut mappings = preview.columns.clone();
        apply_overrides(&mut mappings, &case["mapping"]);
        assert_same(
            &canon(&serde_json::to_value(&mappings).unwrap()),
            &canon(&expected["mappings"]),
            &format!("import case {name}: mappings"),
        );

        let mut next = 0;
        let jobs = process_import_data(
            &preview.raw_data,
            &mappings,
            &settings,
            &ImportContext { today: "<today>" },
            || {
                next += 1;
                (next - 1).to_string()
            },
        );
        let jobs_value = Value::Array(jobs.iter().cloned().map(Value::Object).collect());
        assert_same(
            &canon(&jobs_value),
            &canon(&denonfinite(&expected["jobs"])),
            &format!("import case {name}: jobs"),
        );

        let ctx = OpContext {
            workspace_id: case["workspaceId"].as_str().unwrap(),
            today: "<today>",
            now: "2030-01-01T10:00:00.000Z",
            initial_follow_up: Some("2026-11-24"),
        };
        let mut local = 0;
        let batches = job_create_batches(&jobs, &ctx, || {
            local += 1;
            format!("L{}", local - 1)
        });
        assert_same(
            &canon(&Value::Array(batches)),
            &canon(&denonfinite(&expected["batches"])),
            &format!("import case {name}: ops"),
        );
    }
}

/// Known divergence (see the user story in the lane report): `SheetJS` turns date-looking text into
/// spreadsheet serial numbers, the engine keeps the text the file contains.
#[test]
fn dates_are_kept_as_text() {
    let case = read_json(&fixtures().join("dates.json"));
    let ours = serde_json::to_value(read_rows_text(case["csv"].as_str().unwrap())).unwrap();
    let theirs = &case["sheetjsRows"];
    assert_ne!(
        ours, *theirs,
        "`SheetJS` no longer converts these dates: remove the divergence"
    );
    assert_eq!(ours[0]["Last Action"], "2026-01-05");
    assert_eq!(ours[0]["Next Action"], "01/02/2026");
    assert!(theirs[0]["Last Action"].is_number());
}

#[test]
fn unsupported_files_are_refused() {
    let settings = Settings::default();
    for bytes in [
        &b"<html><body></body></html>"[..],
        b"PK\x03\x04rest",
        b"ID;PWXL",
        b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1x",
        b"\xFF\xFEJ\x00",
    ] {
        assert_eq!(
            parse_import_csv(bytes, &settings).unwrap_err(),
            ImportError::UnsupportedFormat,
            "{bytes:?}"
        );
    }
    assert_eq!(
        parse_import_csv(b"Job\n", &settings).unwrap_err(),
        ImportError::Empty
    );
    assert_eq!(
        parse_import_csv(b"", &settings).unwrap_err(),
        ImportError::Empty
    );
}

#[test]
fn export_then_import_round_trips_the_columns() {
    let case = read_json(&fixtures().join("export/basic.json"));
    let jobs = case["jobs"].as_array().unwrap();
    let ids: Vec<String> = case["orderedColumnIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let settings = settings_of(&case["settings"]);
    let bytes = export_csv(jobs, &ids, &settings, &Layout::default());
    let preview = parse_import_csv(&bytes, &settings).unwrap();
    assert_eq!(preview.total_rows, 3);
    let mappings = preview.columns.clone();
    let mapped: Vec<(&str, &str)> = mappings
        .iter()
        .map(|m| (m.file_header.as_str(), m.target_column_id.as_str()))
        .collect();
    assert_eq!(
        mapped,
        [
            ("Source", "source"),
            ("Job", "title"),
            ("Organization", "organization"),
            ("Industry", "industry"),
            ("Location", "location"),
            ("Workload", "workload"),
            ("Salary", "salary"),
            ("Commute", "commute"),
            ("Status", "status"),
            ("Last Action", "applied_at"),
            ("Next Action", "follow_up_at"),
        ]
        .iter()
        .map(|(h, t)| (*h, *t))
        .collect::<Vec<_>>()
        .into_iter()
        .filter(|(h, _)| mapped.iter().any(|(m, _)| m == h))
        .collect::<Vec<_>>()
    );
    let imported = process_import_data(
        &preview.raw_data,
        &mappings,
        &settings,
        &ImportContext {
            today: "2026-11-17",
        },
        || "x".into(),
    );
    assert_eq!(imported.len(), 3);
    for (job, original) in imported.iter().zip(jobs) {
        for field in [
            "title",
            "organization",
            "location",
            "source",
            "applied_at",
            "follow_up_at",
        ] {
            assert_eq!(job[field], original[field], "{field}");
        }
        assert_eq!(
            job["status_id"],
            original["status_id"]
                .as_i64()
                .filter(|n| *n != 99)
                .map_or(json!(1), |n| json!(n))
        );
    }
}

/// Known divergence (user story in the lane report): the reference stores `Infinity` (which no
/// JSON op can carry) for a custom-column cell reading "Infinity"; the engine leaves it unset.
#[test]
fn non_finite_evaluations_are_skipped() {
    let settings =
        settings_of(&json!({"customColumns": [{"id": "growth", "label": "Growth"}], "kanban": {}}));
    let preview = parse_import_csv(
        b"Job,Growth\nA,Infinity\nB,-Infinity\nC,1e999x\n",
        &settings,
    )
    .unwrap();
    assert_eq!(preview.columns[1].is_custom, Some(true));
    let jobs = process_import_data(
        &preview.raw_data,
        &preview.columns,
        &settings,
        &ImportContext { today: "t" },
        || "x".into(),
    );
    for job in jobs {
        assert_eq!(job["evaluations"], json!({}));
    }
}
