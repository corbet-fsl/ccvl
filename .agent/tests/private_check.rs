//! Offline private-route guards with real Git archives and fixture runtimes.
//! They drive the real `ccvl private-check` command inside an extracted tree.
#![cfg(all(unix, target_os = "linux", target_arch = "x86_64"))]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ccvl::archive::sha256_hex;
use serde_json::{Value, json};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_ccvl");
const UPSTREAM_URL: &str = "https://forge.example.invalid/corbet-fsl/ccvl.git";
const CREDENTIALS: [&str; 10] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GITHUB_ACTIONS",
    "DOWNSTREAM_PUSH_KEY",
    "CARGO_REGISTRY_TOKEN",
    "CARGO_REGISTRIES_CRATES_IO_TOKEN",
    "NPM_TOKEN",
    "NODE_AUTH_TOKEN",
    "PYPI_TOKEN",
    "JSR_TOKEN",
];

const FAKE_RUNTIME: &str = r#"#!/usr/bin/env bash
command="$1"
printf '%s\n' "$command" >> "$TEST_TRACE"
case "$command" in
  runtime-id)
    if [[ -n ${TEST_WRONG_RUNTIME:-} ]]; then printf '%064d\n' 0; else printf '%s\n' "$TEST_RUNTIME_ID"; fi ;;
  downstream-check)
    [[ "${*:2}" == "--policy ccvl-downstream.json --upstream-ref $UPSTREAM_COMMIT" ]] || exit 98
    if [[ -n ${TEST_OWNERSHIP_FAILURE:-} ]]; then echo PRIVATE_SENTINEL >&2; exit 1; fi ;;
  check)
    if [[ -n ${TEST_DOCUMENT_FAILURE:-} ]]; then echo PRIVATE_SENTINEL >&2; exit 1; fi ;;
  *) exit 99 ;;
esac
"#;

const FAKE_CURL: &str = r#"#!/usr/bin/env bash
last="${*: -1}"
name="${last##*/}"
[[ $1 == -q ]] || exit 98
[[ $last == "https://forge.example.invalid/corbet-fsl/ccvl/releases/download/runtime-$TEST_RUNTIME_ID/$name" ]] || exit 97
printf 'download:%s\n' "$name" >> "$TEST_TRACE"
[[ -z ${TEST_RUNTIME_UNAVAILABLE:-} ]] || exit 22
output=
for ((index = 1; index < $#; index++)); do
  if [[ ${!index} == --output ]]; then next=$((index + 1)); output=${!next}; fi
done
cp "$TEST_ASSETS/$name" "$output"
"#;

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn file_hash(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap())
}

fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Downstream {
    temp: TempDir,
    repo: PathBuf,
    extracted: PathBuf,
    archive: PathBuf,
    assets: PathBuf,
    trace: PathBuf,
    commit: String,
    upstream: String,
    fingerprint: String,
    git_environment: Vec<(&'static str, &'static str)>,
    env: BTreeMap<String, String>,
}

impl Downstream {
    fn git(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .envs(self.git_environment.iter().copied())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("ccvl-private-test-")
            .tempdir()
            .unwrap();
        let base = temp.path().to_path_buf();
        let repo = base.join("repository");
        fs::create_dir_all(&repo).unwrap();
        let git_environment = vec![
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_AUTHOR_NAME", "fixture"),
            ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
            ("GIT_COMMITTER_NAME", "fixture"),
            ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
        ];
        let mut downstream = Self {
            temp,
            repo,
            extracted: base.join("extracted"),
            archive: base.join("source.tar"),
            assets: base.join("published"),
            trace: base.join("trace"),
            commit: String::new(),
            upstream: String::new(),
            fingerprint: String::new(),
            git_environment,
            env: BTreeMap::new(),
        };
        downstream.git(&["init", "--quiet", "--template=", "--initial-branch=main"]);
        for name in [
            "Cargo.toml",
            ".agent/core/Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".agent/build.rs",
            ".agent/src/main.rs",
            ".agent/core/src/lib.rs",
            ".agent/typst/lib.typ",
        ] {
            downstream.write(name, "runtime fixture\n");
        }
        downstream.git(&["add", "."]);
        downstream.git(&["commit", "--quiet", "-m", "public fixture"]);
        downstream.upstream = downstream.rev_parse();
        downstream.write(
            "ccvl-downstream.json",
            &json!({
                "schema_version": 1,
                "upstream": {"remote": "upstream", "url": UPSTREAM_URL, "branch": "main"},
                "allowed_paths": ["ccvl-downstream.json", "inputs/interviews/"],
            })
            .to_string(),
        );
        downstream.write("inputs/interviews/profile.md", "PRIVATE_SENTINEL\n");
        downstream.git(&["add", "."]);
        downstream.git(&["commit", "--quiet", "-m", "private fixture"]);
        downstream.commit = downstream.rev_parse();
        let commit = downstream.commit.clone();
        downstream.git(&["update-ref", "refs/heads/source", &commit]);
        let bundle = base.join("source.bundle");
        downstream.git(&[
            "bundle",
            "create",
            bundle.to_str().unwrap(),
            "refs/heads/source",
        ]);
        fs::write(&downstream.archive, downstream.git(&["archive", &commit])).unwrap();
        fs::create_dir_all(&downstream.extracted).unwrap();
        let status = Command::new("tar")
            .arg("-xf")
            .arg(&downstream.archive)
            .arg("-C")
            .arg(&downstream.extracted)
            .status()
            .unwrap();
        assert!(status.success());
        // The shell algorithm that bootstrap uses is the reference identity.
        let helper = repository().join(".agent/scripts/runtime-id.sh");
        let output = Command::new("bash")
            .args([
                "-c",
                "source \"$1\"; repo_root=\"$2\"; probe() { command -v \"$1\"; }; source_fingerprint",
                "fixture",
            ])
            .arg(&helper)
            .arg(&downstream.repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .clone_into(&mut downstream.fingerprint);
        let bin = base.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&downstream.assets).unwrap();
        fs::write(&downstream.trace, "").unwrap();
        let binary = downstream.assets.join("ccvl-linux-x86_64");
        executable(&binary, FAKE_RUNTIME);
        fs::write(
            downstream.assets.join("ccvl-linux-x86_64.sha256"),
            format!("{}  ccvl-linux-x86_64\n", file_hash(&binary)),
        )
        .unwrap();
        executable(&bin.join("curl"), FAKE_CURL);
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        for (key, value) in [
            ("PATH", path),
            ("CI", "crow".to_owned()),
            ("CCVL_UPSTREAM_URL", UPSTREAM_URL.to_owned()),
            ("CI_REPO", "fixture/private-downstream".to_owned()),
            ("CI_REPO_PRIVATE", "true".to_owned()),
            ("CI_PIPELINE_EVENT", "manual".to_owned()),
            ("CI_COMMIT_SHA", downstream.commit.clone()),
            ("UPSTREAM_COMMIT", downstream.upstream.clone()),
            (
                "CARGO_TARGET_DIR",
                base.join("target").display().to_string(),
            ),
            (
                "CCID_TARGET_LOCK_HELD",
                base.join("target").display().to_string(),
            ),
            ("SOURCE_ARCHIVE", downstream.archive.display().to_string()),
            ("SOURCE_SHA256", file_hash(&downstream.archive)),
            ("DOWNSTREAM_SOURCE_BUNDLE", bundle.display().to_string()),
            ("DOWNSTREAM_SOURCE_SHA256", file_hash(&bundle)),
            ("TEST_RUNTIME_ID", downstream.fingerprint.clone()),
            ("TEST_TRACE", downstream.trace.display().to_string()),
            ("TEST_ASSETS", downstream.assets.display().to_string()),
        ] {
            downstream.env.insert(key.to_owned(), value);
        }
        downstream
    }

    fn write(&self, name: &str, content: &str) {
        let path = self.repo.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn rev_parse(&self) -> String {
        String::from_utf8(self.git(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn invoke(&self, success: bool, changes: &[(&str, &str)]) -> (Output, Vec<String>) {
        fs::write(&self.trace, "").unwrap();
        let mut env = self.env.clone();
        for (key, value) in changes {
            env.insert((*key).to_owned(), (*value).to_owned());
        }
        let output = Command::new(BIN)
            .arg("private-check")
            .env_clear()
            .envs(env)
            .current_dir(&self.extracted)
            .output()
            .unwrap();
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.success(), success, "{combined}");
        assert!(!combined.contains("PRIVATE_SENTINEL"), "{combined}");
        assert_eq!(self.rev_parse(), self.commit);
        assert_eq!(self.git(&["status", "--porcelain"]), [] as [u8; 0]);
        let trace = fs::read_to_string(&self.trace)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        (output, trace)
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn exact_private_commit_runs_both_gates_without_changing_source() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(true, &[]);
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["source_commit"], fixture.commit.as_str());
    assert_eq!(receipt["upstream_commit"], fixture.upstream.as_str());
    assert_eq!(receipt["checks"], json!(["downstream-check", "check"]));
    assert_eq!(receipt["merged"], false);
    assert_eq!(receipt["pushed"], false);
    assert_eq!(receipt["runtime_id"], fixture.fingerprint.as_str());
    assert_eq!(receipt["repository"], "fixture/private-downstream");
    assert_eq!(
        trace[trace.len() - 3..],
        ["runtime-id", "downstream-check", "check"]
    );
    // Receipts keep the established sorted, single line form.
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.starts_with("{\"binary_sha256\": \""), "{printed}");
}

#[test]
fn public_hosted_and_nonmanual_execution_are_refused_before_download() {
    let fixture = Downstream::new();
    for environment in [
        ("CI", "true"),
        ("CI_REPO_PRIVATE", "false"),
        ("CI_REPO", "corbet-fsl/ccvl"),
        ("GITHUB_ACTIONS", "true"),
        ("CI_PIPELINE_EVENT", "push"),
    ] {
        let (_, trace) = fixture.invoke(false, &[environment]);
        assert!(trace.is_empty(), "{environment:?}");
    }
}

#[test]
fn publication_credentials_are_refused_without_disclosure() {
    let fixture = Downstream::new();
    for key in CREDENTIALS
        .into_iter()
        .filter(|key| *key != "GITHUB_ACTIONS")
    {
        let (output, trace) = fixture.invoke(false, &[(key, "PRIVATE_SENTINEL")]);
        assert!(trace.is_empty(), "{key}");
        assert!(
            stderr(&output).contains("Publication credentials are not accepted"),
            "{key}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn archive_checksum_and_extracted_source_changes_are_refused() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("SOURCE_SHA256", &"0".repeat(64))]);
    assert_eq!(trace, [] as [std::string::String; 0]);
    assert!(stderr(&output).contains("Source archive checksum mismatch."));
    fs::write(
        fixture.extracted.join("inputs/interviews/profile.md"),
        "changed private source",
    )
    .unwrap();
    let (output, trace) = fixture.invoke(false, &[]);
    assert_eq!(trace, [] as [std::string::String; 0]);
    assert!(stderr(&output).contains("Extracted source differs from its archive."));
}

#[test]
fn archive_and_bundle_tree_disagreement_is_refused() {
    let fixture = Downstream::new();
    let mut bytes = fs::read(&fixture.archive).unwrap();
    let original = b"PRIVATE_SENTINEL\n";
    let position = bytes
        .windows(original.len())
        .position(|window| window == original)
        .unwrap();
    bytes[position..position + original.len()].copy_from_slice(b"ALTERED_TRANSPORT");
    fs::write(&fixture.archive, &bytes).unwrap();
    fs::write(
        fixture.extracted.join("inputs/interviews/profile.md"),
        "ALTERED_TRANSPORT",
    )
    .unwrap();
    let (output, trace) = fixture.invoke(false, &[("SOURCE_SHA256", &sha256_hex(&bytes))]);
    assert_eq!(trace, [] as [std::string::String; 0]);
    assert!(stderr(&output).contains("Source archive differs from the committed Git bundle tree."));
}

#[test]
fn wrong_bundle_and_nonancestor_upstream_are_refused() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("DOWNSTREAM_SOURCE_SHA256", &"0".repeat(64))]);
    assert_eq!(trace, [] as [std::string::String; 0]);
    assert!(stderr(&output).contains("Git bundle checksum mismatch."));
    let (output, trace) = fixture.invoke(false, &[("UPSTREAM_COMMIT", &"0".repeat(40))]);
    assert_eq!(trace, [] as [std::string::String; 0]);
    assert!(stderr(&output).contains("Upstream ancestry failed"));
}

#[test]
fn missing_published_runtime_stops_before_private_gates() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("TEST_RUNTIME_UNAVAILABLE", "1")]);
    assert!(
        stderr(&output).contains("Matching published runtime checksum unavailable"),
        "{}",
        stderr(&output)
    );
    assert!(!trace.iter().any(|line| line == "downstream-check"));
}

#[test]
fn wrong_runtime_checksum_and_identity_stop_before_private_gates() {
    let fixture = Downstream::new();
    let (_, trace) = fixture.invoke(false, &[("TEST_WRONG_RUNTIME", "1")]);
    assert!(!trace.iter().any(|line| line == "downstream-check"));
    fs::write(
        fixture.assets.join("ccvl-linux-x86_64.sha256"),
        format!("{}  ccvl-linux-x86_64\n", "0".repeat(64)),
    )
    .unwrap();
    let (_, trace) = fixture.invoke(false, &[]);
    assert!(!trace.iter().any(|line| line == "runtime-id"));
}

#[test]
fn existing_runtime_requires_published_checksum_and_is_not_reinstalled() {
    let fixture = Downstream::new();
    let existing = fixture.assets.join("ccvl-linux-x86_64");
    let (_, trace) = fixture.invoke(true, &[("CCVL_CHECK_RUNTIME", existing.to_str().unwrap())]);
    let downloads: Vec<_> = trace
        .iter()
        .filter(|line| line.starts_with("download:"))
        .collect();
    assert_eq!(downloads, ["download:ccvl-linux-x86_64.sha256"]);
    assert!(!fixture.extracted.join(".agent/cache").exists());
}

#[test]
fn ownership_and_document_failures_do_not_report_success_or_private_text() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("TEST_OWNERSHIP_FAILURE", "1")]);
    assert!(stderr(&output).contains("Downstream ownership gate failed"));
    assert!(!trace.iter().any(|line| line == "check"));
    let (output, trace) = fixture.invoke(false, &[("TEST_DOCUMENT_FAILURE", "1")]);
    assert!(stderr(&output).contains("Full private document gate failed"));
    assert_eq!(trace[trace.len() - 2..], ["downstream-check", "check"]);
}

#[test]
fn malformed_policy_reports_only_a_fixed_message() {
    let fixture = Downstream::new();
    // A policy that is not JSON can never be echoed into a log: build a second
    // private commit whose policy carries a sentinel and bundle it.
    fixture.write("ccvl-downstream.json", "PRIVATE_SENTINEL is not json");
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "--quiet", "-m", "broken policy"]);
    let commit = fixture.rev_parse();
    fixture.git(&["update-ref", "refs/heads/source", &commit]);
    let bundle = fixture.temp.path().join("broken.bundle");
    fixture.git(&[
        "bundle",
        "create",
        bundle.to_str().unwrap(),
        "refs/heads/source",
    ]);
    let archive = fixture.git(&["archive", &commit]);
    let archive_path = fixture.temp.path().join("broken.tar");
    fs::write(&archive_path, &archive).unwrap();
    fs::write(
        fixture.extracted.join("ccvl-downstream.json"),
        "PRIVATE_SENTINEL is not json",
    )
    .unwrap();
    let broken = [
        ("CI_COMMIT_SHA", commit.as_str()),
        ("SOURCE_ARCHIVE", archive_path.to_str().unwrap()),
        ("SOURCE_SHA256", &sha256_hex(&archive)),
        ("DOWNSTREAM_SOURCE_BUNDLE", bundle.to_str().unwrap()),
        ("DOWNSTREAM_SOURCE_SHA256", &file_hash(&bundle)),
    ];
    let output = Command::new(BIN)
        .arg("private-check")
        .env_clear()
        .envs(fixture.env.clone())
        .envs(broken.iter().copied())
        .current_dir(&fixture.extracted)
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        stderr(&output)
    );
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(!combined.contains("PRIVATE_SENTINEL"), "{combined}");
    assert!(
        combined.contains("Private check inputs are unavailable or invalid."),
        "{combined}"
    );
}
