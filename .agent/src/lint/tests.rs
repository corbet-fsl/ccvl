use std::fs;

use serde_json::json;

use super::*;

fn plan(source: &str) -> Value {
    serde_json::to_value(toml::from_str::<toml::Value>(source).unwrap()).unwrap()
}

const CV: &str = r#"
  #cv-compact-heading[#cv-strings.experience]
  // ccvl-station: alpha
  #cv-h[Alpha: #brand[ACME]]
  #cv-gap("heading_after_pt")
  #cv-s[Analyst | Team · 2024 · Zurich]
  #cv-b[Built *three* models with #brand[Typst] | \>15% faster]
  #cv-b[Cut review time by 80%]
  #cv-entry-gap()
  // ccvl-station: beta
  #cv-h[Beta]
  #cv-s(min-fill: 25, target-fill: 35)[First aid | de-escalation]
  #cv-b[Helped people]
  #cv-hu[Compact line that belongs to no entry]
  #cv-b[Stray bullet]
  #cv-pagebreak()
  // ccvl-project: gamma
  #cv-h[Gamma]
  #cv-s[Product release 2026]
  #cv-b[One]
  #cv-b[Two]
"#;

const PLAN_SOURCE: &str = r#"schema_version = 1
updated = "2026-11-01"

[[stations]]
id = "alpha"
facts = [
  { id = "alpha-scope", text = "Not compared" },
  { id = "alpha-role", text = "Analyst | Team · 2024 · Zurich" },
  { id = "alpha-1", text = "Built three models with Typst | >15% faster" },
  { id = "alpha-2", text = "Cut review time by 70%" },
  { id = "alpha-3", text = "A fact the CV dropped" },
]

[[stations]]
id = "beta"
facts = [
  { id = "beta-1", text = "Helped people" },
]

[[projects]]
id = "gamma"
anchor = "Product release 2025"
facts = [
  { id = "gamma-1", text = "One" },
  { id = "gamma-2", text = "Two" },
]
"#;

fn drift(plan_source: &str) -> Vec<String> {
    drift_against(
        &plan(plan_source),
        plan_source,
        "interview/stations.toml",
        CV,
        "cv.typ",
    )
}

#[test]
fn typst_markup_is_compared_as_plain_text() {
    assert_eq!(
        plain_text(r"Introduced #brand[Claude] for *bold* work | \>90% lower~cost"),
        "Introduced Claude for bold work | >90% lower cost"
    );
    assert_eq!(
        plain_text("#cv-strings.projects and [brackets] stay"),
        "and [brackets] stay"
    );
    assert_eq!(plain_text("team’s  data"), "team's data");
}

#[test]
fn cv_entries_read_subheadings_and_bullets_until_compact_lines() {
    let entries = cv_entries(CV);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta", "gamma"]
    );
    let alpha = &entries[0];
    assert_eq!(alpha.kind, "station");
    assert_eq!(alpha.line, 3);
    assert_eq!(
        alpha.subheading.as_ref().unwrap().text,
        "Analyst | Team · 2024 · Zurich"
    );
    assert_eq!(
        alpha.bullets[0].text,
        "Built three models with Typst | >15% faster"
    );
    assert_eq!(alpha.bullets[1].line, 8);
    let beta = &entries[1];
    assert_eq!(
        beta.subheading.as_ref().unwrap().text,
        "First aid | de-escalation"
    );
    assert_eq!(beta.bullets.len(), 1, "the compact line ends the entry");
    assert_eq!(entries[2].kind, "project");
}

#[test]
fn drift_reports_differing_missing_and_extra_facts_with_locations() {
    let hints = drift(PLAN_SOURCE);
    assert_eq!(hints.len(), 4, "{hints:#?}");
    assert!(hints[0].starts_with(
        "interview/stations.toml:10: interview fact alpha-2 differs from CV bullet 2 at cv.typ:8;"
    ));
    assert!(
        hints[0].contains(r#"interview: "Cut review time by 70%"; CV: "Cut review time by 80%""#)
    );
    assert!(hints[1].starts_with(
        "interview/stations.toml:11: interview fact alpha-3 has no CV bullet 3 in station alpha at cv.typ:3"
    ));
    assert_eq!(
        hints[2],
        r#"cv.typ:12: CV subheading of station beta has no interview fact beta-role: "First aid | de-escalation""#
    );
    assert!(hints[3].starts_with(
        "interview/stations.toml:21: anchor of project gamma differs from CV subheading at cv.typ:19;"
    ));
}

#[test]
fn drift_reports_cv_entries_missing_from_the_plan() {
    let source = PLAN_SOURCE.replace("id = \"beta\"", "id = \"delta\"");
    let hints = drift(&source);
    assert!(
        hints.contains(
            &"cv.typ:10: CV station beta has no interview entry in interview/stations.toml"
                .to_owned()
        ),
        "{hints:#?}"
    );
}

#[test]
fn allow_drift_acknowledges_a_fact_or_a_whole_entry() {
    let source = PLAN_SOURCE
        .replace(
            r#"{ id = "alpha-2", text = "Cut review time by 70%" }"#,
            r#"{ id = "alpha-2", text = "Cut review time by 70%", allow_drift = "the CV rounds the measured value" }"#,
        )
        .replace(
            "id = \"beta\"\n",
            "id = \"beta\"\nallow_drift = \"the CV keeps a shorter subheading\"\n",
        );
    let hints = drift(&source);
    assert_eq!(hints.len(), 2, "{hints:#?}");
    assert!(hints.iter().all(|hint| !hint.contains("alpha-2")));
    assert!(hints.iter().all(|hint| !hint.contains("beta")));

    let empty = plan(&format!("{PLAN_SOURCE}allow_drift = \"\"\n"));
    let error = validate_plan(&empty, "plan").unwrap_err().to_string();
    assert!(error.contains("needs a short written reason"), "{error}");
}

#[test]
fn drift_is_silent_without_stations() {
    let workspace = crate::test_support::fixture_view();
    let hints = drift_hints(
        &workspace,
        &json!({"schema_version": 1, "stations": []}),
        "",
        "plan",
    )
    .unwrap();
    assert!(hints.is_empty());
}

const RULES: &str = r#"
[[not_true]]
term = "SAP"
note = "Never used SAP professionally"

[[not_true]]
term = "ERP"
note = "Only the own mini ERP"
except = ["mini-ERP", "mini ERP"]
"#;

#[test]
fn not_true_terms_match_whole_words_case_insensitively_outside_exceptions() {
    let rules = rules(&plan(RULES), "plan").unwrap();
    assert_eq!(
        rules[0].find("Led SAP S/4HANA and sap rollout"),
        ["SAP", "sap"]
    );
    assert!(rules[0].find("Sapporo, ASAP, SAPling").is_empty());
    assert!(
        rules[1]
            .find("through own mini-ERP and a Mini ERP")
            .is_empty()
    );
    assert_eq!(rules[1].find("ERP-Systeme and ERP"), ["ERP", "ERP"]);
    let sensitive =
        rules_from("[[not_true]]\nterm = \"Go\"\nnote = \"n\"\ncase_sensitive = true\n");
    assert_eq!(sensitive[0].find("Go and go"), ["Go"]);
}

fn rules_from(source: &str) -> Vec<Rule> {
    rules(&plan(source), "plan").unwrap()
}

#[test]
fn not_true_schema_errors_are_structural() {
    for (source, expected) in [
        (
            "[[not_true]]\nterm = \"SAP\"\nnote = \"a\"\n[[not_true]]\nterm = \"sap\"\nnote = \"b\"\n",
            "duplicate not_true term",
        ),
        (
            "[[not_true]]\nterm = \"SAP\"\n",
            "note: non-empty text is required",
        ),
        (
            "[[not_true]]\nterm = \"SAP\"\nnote = \"a\"\nexcept = [\"\"]\n",
            "non-empty phrases",
        ),
        (
            "[[not_true]]\nterm = \"SAP\"\nnote = \"a\"\nexceptions = []\n",
            "unknown field exceptions",
        ),
    ] {
        let error = rules(&plan(source), "plan").unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

fn write(workspace: &Workspace, relative: &str, content: &str) {
    let path = workspace.path(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

const RECORD: &str = r#"schema_version = 4

[job]
title = "SAP Finance Lead"

[cv]
summary = "Brings SAP and ERP rollout experience through own mini-ERP work"

[cl]
paragraphs = [
  [
    "I know the ERP landscape",
  ],
]
"#;

#[test]
fn not_true_hints_cover_records_cvl_sources_and_the_plan() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    let plan_source = format!(
        "{RULES}\n[[stations]]\nid = \"alpha\"\nfacts = [\n  {{ id = \"alpha-1\", text = \"Configured SAP modules\" }},\n]\n"
    );
    write(
        &workspace,
        "opportunities/acme/lead/application.toml",
        RECORD,
    );
    write(
        &workspace,
        "opportunities/acme/lead/variants/harvard/standard/old/application.toml",
        RECORD,
    );
    write(
        &workspace,
        "opportunities/acme/lead/review/run-1/application.toml",
        RECORD,
    );
    write(
        &workspace,
        "cvl/cv/ledger/src/entries-en.typ",
        "// SAP in a comment is not wording\n#cv-b[Automated a five-figure eBay operation through own mini-ERP]\n#cv-b[Ran SAP reports]\n",
    );
    let hints = not_true_hints(
        &workspace,
        &plan(&plan_source),
        &plan_source,
        "interview/stations.toml",
    )
    .unwrap();
    assert_eq!(hints.len(), 5, "{hints:#?}");
    assert!(hints[0].starts_with(
        r#"interview/stations.toml:14: fact alpha-1 says "SAP", which matches not_true term "SAP""#
    ));
    assert_eq!(
        hints[1],
        r#"cvl/cv/ledger/src/entries-en.typ:3: "SAP" matches not_true term "SAP": Never used SAP professionally"#
    );
    assert!(hints[2].starts_with(r#"opportunities/acme/lead/application.toml:7: "SAP" matches"#));
    assert!(hints[3].starts_with(r#"opportunities/acme/lead/application.toml:7: "ERP" matches"#));
    assert!(hints[4].starts_with(
        r#"opportunities/acme/lead/application.toml:12: "ERP" matches not_true term "ERP": Only the own mini ERP"#
    ));
    assert!(
        hints
            .iter()
            .all(|hint| !hint.contains("variants") && !hint.contains("review/")),
        "variants and review folders are skipped"
    );
}

#[test]
fn record_acknowledgements_silence_terms_and_flag_stale_ones() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    let record = format!(
        "{}\n[lint]\nallow = [\n  {{ term = \"SAP\", reason = \"names the employer's SAP environment, not my experience\" }},\n  {{ term = \"Oracle\", reason = \"left over\" }},\n]\n",
        RECORD.replace("SAP and ERP rollout", "SAP rollout")
    );
    write(
        &workspace,
        "opportunities/acme/lead/application.toml",
        &record,
    );
    let rules = rules_from(RULES);
    let hints = file_term_hints(
        &workspace,
        &rules,
        &workspace.path("opportunities/acme/lead/application.toml"),
    )
    .unwrap();
    assert_eq!(hints.len(), 2, "{hints:#?}");
    assert!(hints[0].contains(r#""ERP" matches not_true term "ERP""#));
    assert_eq!(
        hints[1],
        r#"opportunities/acme/lead/application.toml:19: [lint] allow for term "Oracle" matches nothing here; remove the stale acknowledgement"#
    );
}

#[test]
fn record_lint_table_is_validated_and_accepted_by_records() {
    let ok = json!({"lint": {"allow": [
        {"term": "SAP", "reason": "names the employer's system"},
        {"figure": "40%", "reason": "the employer's own figure"}
    ]}});
    validate_record_lint(&ok, "record").unwrap();
    for (lint, expected) in [
        (
            json!({"allow": [{"term": "SAP"}]}),
            "needs a short written reason",
        ),
        (
            json!({"allow": [{"term": "SAP", "figure": "1", "reason": "r"}]}),
            "exactly one of term or figure",
        ),
        (
            json!({"allow": [{"reason": "r"}]}),
            "exactly one of term or figure",
        ),
        (
            json!({"allow": [{"term": "SAP", "reason": "r", "until": "x"}]}),
            "unknown field until",
        ),
        (json!({"ignore": []}), "unknown field ignore"),
    ] {
        let error = validate_record_lint(&json!({"lint": lint}), "record")
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }

    let workspace = crate::test_support::fixture_view();
    let leaf = crate::application::cv_leaves(&workspace).unwrap().remove(0);
    let mut record = crate::content::read_record(&workspace, leaf.content()).unwrap();
    record["lint"] = ok["lint"].clone();
    crate::application::validate_document_record(&workspace, &record, "fixture", true, "cv")
        .unwrap();
    record["lint"] = json!({"allow": [{"term": "SAP"}]});
    assert!(
        crate::application::validate_document_record(&workspace, &record, "fixture", true, "cv")
            .is_err()
    );
}

#[test]
fn third_party_confirmation_is_worth_a_second_look() {
    let source = r#"schema_version = 1

[[stations]]
id = "alpha"
facts = [
  { id = "alpha-1", text = "Used SAP", source_refs = ["confirmed-by:Jane Doe:2026-10-01"] },
  { id = "alpha-2", text = "Own work" },
]
source_refs = ["user-confirmed:2026-09-03"]

[[stations]]
id = "beta"
facts = [{ id = "beta-1", text = "Fact" }]
source_refs = ["confirmed-by:Jane Doe:2026-10-01", "confirmed-by:Max Muster:2026-10-02"]

[[projects]]
id = "gamma"
facts = [{ id = "gamma-1", text = "Fact" }]
source_refs = ["confirmed-by:Jane Doe:2026-10-01", "cv:cvl/cv/harvard/src/entries-en.typ"]
"#;
    let document = plan(source);
    validate_plan(&document, "plan").unwrap();
    let hints = second_look_hints(&document, source, "interview/stations.toml");
    assert_eq!(
        hints,
        [
            "interview/stations.toml:6: fact alpha-1 is confirmed only by Jane Doe (2026-10-01), not by the candidate; worth a second look",
            "interview/stations.toml:12: station beta is confirmed only by Jane Doe (2026-10-01), Max Muster (2026-10-02), not by the candidate; worth a second look",
        ]
    );
    assert!(candidate_confirmation("user-confirmed:2026-09-03"));
    assert!(candidate_confirmation("author-confirmed:2026-09-03"));
    assert!(!candidate_confirmation("confirmed-by:Jane Doe:2026-10-01"));

    for bad in [
        "confirmed-by:2026-10-01",
        "confirmed-by:Jane:2026-13-01",
        "confirmed-by::2026-10-01",
    ] {
        let document = json!({"stations": [{"id": "a", "source_refs": [bad]}]});
        let error = validate_plan(&document, "plan").unwrap_err().to_string();
        assert!(
            error.contains("confirmed-by:<person>:<YYYY-MM-DD>"),
            "{bad}: {error}"
        );
    }
}

#[test]
fn interview_listing_is_plain_stable_markdown() {
    let source = format!(
        r#"schema_version = 1
updated = "2026-11-01"
{RULES}
[[stations]]
id = "alpha"
label = "Alpha work"
anchor = "ACME | 2024"
kind = "employment"
status = "verified"
page = 1
section = "experience"
facts = [
  {{ id = "alpha-role", text = "Analyst · 2024" }},
  {{ id = "alpha-1", text = "Built models" }},
]
source_refs = ["user-confirmed:2026-09-03"]

[[stations]]
id = "spare"
label = "Spare"
anchor = "Later"
status = "unverified"
section = ""
facts = []
source_refs = []

[[competencies]]
id = "skills"
label = "Skills"
facts = [{{ id = "skills-1", text = "Rust | Typst" }}]
"#
    );
    let listing = interview_markdown(&plan(&source));
    assert_eq!(
        listing,
        "# Interview record

Updated: 2026-11-01

## Stations

### Alpha work (alpha)

ACME | 2024 · employment · verified · page 1, experience

Role: Analyst · 2024

- alpha-1: Built models

Sources: user-confirmed:2026-09-03

### Spare (spare)

Later · unverified · unassigned

## Projects

None recorded.

## Competencies

### Skills (skills)

- skills-1: Rust | Typst

## Not true

- SAP: Never used SAP professionally
- ERP: Only the own mini ERP (except: mini-ERP, mini ERP)"
    );
}

#[test]
fn figures_are_quantified_claims_only() {
    let keys = |text: &str| {
        figures(text)
            .into_iter()
            .map(|figure| (figure.text, figure.key))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        keys(
            "Supported CHF 10m+ talks, cut 80% of time, 1'000+ calls, 16 years, eight-figure deals"
        ),
        [
            ("CHF 10m+".to_owned(), "10".to_owned()),
            ("80%".to_owned(), "80".to_owned()),
            ("1'000+".to_owned(), "1000".to_owned()),
            ("16 years".to_owned(), "16".to_owned()),
            ("eight-figure".to_owned(), "8-figure".to_owned()),
        ]
    );
    assert!(
        figures("In 2024 at Tier-1 banks with T+1 settlement, grade 1.0, S/4HANA, 3 teams")
            .is_empty()
    );
    assert_eq!(keys("achtstellige Investitionen")[0].1, "8-figure");
    assert_eq!(keys("1,000+ calls")[0].1, "1000");
    assert_eq!(keys("93.4% less")[0].1, "93.4");
}

#[test]
fn unbacked_figures_are_hinted_unless_acknowledged() {
    let (_fixture, workspace) = crate::test_support::fixture_workspace();
    write(
        &workspace,
        PLAN,
        "schema_version = 1\nupdated = \"2026-11-01\"\n\n[[stations]]\nid = \"alpha\"\nfacts = [\n  { id = \"alpha-1\", text = \"Supported CHF 10m+ negotiations over 16 years; eight-figure investment\" },\n]\n",
    );
    write(
        &workspace,
        "opportunities/acme/lead/application.toml",
        "schema_version = 4\n\n[job]\ntitle = \"Lead with 40% travel\"\n\n[cv]\nsummary = \"Negotiated CHF 10 million deals over 16 years and an achtstellige investment\"\n\n[cl]\nparagraphs = [\n  [\"Cut costs by 35% and grew 2× in 2024\"],\n]\n\n[lint]\nallow = [{ figure = \"2×\", reason = \"the posting's own growth target\" }]\n",
    );
    let hints = figure_hints(&workspace).unwrap();
    assert_eq!(
        hints,
        [format!(
            r#"opportunities/acme/lead/application.toml:11: figure "35%" occurs in no interview fact; add the fact to {PLAN} or acknowledge it under [lint] allow"#
        )]
    );
}
