//! Offline private-route guards with real Git repositories and fixture runtimes.
//! They drive the real `ccvl private-check` command inside a shallow checkout,
//! the way a Forgejo job leaves it.
#![cfg(all(unix, target_os = "linux", target_arch = "x86_64"))]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_ccvl");
const UPSTREAM_URL: &str = "https://forge.example.invalid/corbet-fsl/ccvl.git";
const TOKEN: &str = "fixture-job-token";
/// Publication credentials: each one alone must stop the stage.
const CREDENTIALS: [&str; 7] = [
    "GH_TOKEN",
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
[[ -z ${GITHUB_TOKEN:-}${CCID_FORGE_TOKEN:-}${FORGEJO_TOKEN:-} ]] || exit 97
case "$command" in
  runtime-id)
    if [[ -n ${TEST_WRONG_RUNTIME:-} ]]; then printf '%064d\n' 0; else printf '%s\n' "$TEST_RUNTIME_ID"; fi ;;
  downstream-check)
    [[ "${*:2}" == "--policy ccvl-downstream.json --upstream-ref $EXPECTED_UPSTREAM" ]] || exit 98
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
[[ "$*" != *"$TEST_TOKEN"* ]] || exit 96
output=
for ((index = 1; index < $#; index++)); do
  if [[ ${!index} == --output ]]; then next=$((index + 1)); output=${!next}; fi
done
if [[ $last == "https://forge.example.invalid/api/v1/repos/fixture/private-downstream" ]]; then
  config="$(cat)"
  [[ $config == "header = \"Authorization: token $TEST_TOKEN\""* ]] || exit 95
  printf 'api\n' >> "$TEST_TRACE"
  private=true
  [[ -z ${TEST_PUBLIC:-} ]] || private=false
  printf '{"private": %s, "full_name": "fixture/private-downstream"}' "$private" > "$output"
  exit 0
fi
if [[ $last == "https://forge.example.invalid/api/v1/repos/corbet-fsl/ccvl/actions/runs?head_sha="*"&workflow_id=ci.yml&status=success&limit=1" ]]; then
  sha="${last#*head_sha=}"; sha="${sha%%&*}"
  printf 'ci:%s\n' "$sha" >> "$TEST_TRACE"
  [[ -z ${TEST_CI_UNAVAILABLE:-} ]] || exit 22
  if [[ -n ${TEST_CI_RED:-} ]]; then printf '{"workflow_runs": [], "total_count": 0}' > "$output"
  else printf '{"workflow_runs": [{"id": 7, "status": "success"}], "total_count": 1}' > "$output"; fi
  exit 0
fi
exit 97
"#;

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Downstream {
    _temp: TempDir,
    repo: PathBuf,
    /// The shallow checkout a Forgejo job leaves behind.
    extracted: PathBuf,
    assets: PathBuf,
    trace: PathBuf,
    commit: String,
    upstream: String,
    fingerprint: String,
    git_environment: Vec<(&'static str, &'static str)>,
    env: BTreeMap<String, String>,
}

impl Downstream {
    fn git_in(&self, directory: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
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

    fn git(&self, args: &[&str]) -> Vec<u8> {
        self.git_in(&self.repo, args)
    }

    fn new() -> Self {
        Self::with_policy(None)
    }

    fn with_policy(policy: Option<&str>) -> Self {
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
            _temp: temp,
            repo,
            extracted: base.join("extracted"),
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
            "ccvl.json",
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
            &policy.map_or_else(
                || {
                    json!({
                        "schema_version": 1,
                        "upstream": {"remote": "upstream", "url": UPSTREAM_URL, "branch": "main"},
                        "allowed_paths": ["ccvl-downstream.json", "inputs/interviews/"],
                    })
                    .to_string()
                },
                str::to_owned,
            ),
        );
        downstream.write("inputs/interviews/profile.md", "PRIVATE_SENTINEL\n");
        downstream.git(&["add", "."]);
        downstream.git(&["commit", "--quiet", "-m", "private fixture"]);
        downstream.commit = downstream.rev_parse();
        // The public repository as the forge serves it: only the public commit.
        let public = base.join("public.git");
        fs::create_dir_all(&public).unwrap();
        downstream.git_in(
            &public,
            &["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        let upstream = downstream.upstream.clone();
        downstream.git(&[
            "push",
            "--quiet",
            public.to_str().unwrap(),
            &format!("{upstream}:refs/heads/main"),
        ]);
        // A Forgejo job fetches the exact commit with depth 1.
        let url = format!("file://{}", downstream.repo.display());
        let status = Command::new("git")
            .args(["clone", "--quiet", "--depth", "1", &url])
            .arg(&downstream.extracted)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .envs(downstream.git_environment.iter().copied())
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
        executable(&bin.join("curl"), FAKE_CURL);
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        for (key, value) in [
            ("PATH", path),
            // What a Forgejo runner exports; FORGEJO_ACTIONS is the discriminator (present in forgejo-runner 13.1.0).
            ("CI", "true".to_owned()),
            ("GITHUB_ACTIONS", "true".to_owned()),
            ("FORGEJO_ACTIONS", "true".to_owned()),
            ("GITHUB_EVENT_NAME", "workflow_dispatch".to_owned()),
            ("GITHUB_REPOSITORY", "fixture/private-downstream".to_owned()),
            (
                "GITHUB_SERVER_URL",
                "https://forge.example.invalid".to_owned(),
            ),
            (
                "GITHUB_API_URL",
                "https://forge.example.invalid/api/v1".to_owned(),
            ),
            ("CCID_FORGE_TOKEN", TOKEN.to_owned()),
            ("GITHUB_TOKEN", TOKEN.to_owned()),
            ("FORGEJO_TOKEN", TOKEN.to_owned()),
            ("ACTIONS_RUNTIME_TOKEN", "runtime-token".to_owned()),
            ("CI_COMMIT_SHA", downstream.commit.clone()),
            ("CCVL_UPSTREAM_FETCH_URL", public.display().to_string()),
            (
                "CARGO_TARGET_DIR",
                base.join("target").display().to_string(),
            ),
            (
                "CCID_TARGET_LOCK_HELD",
                base.join("target").display().to_string(),
            ),
            ("TEST_TOKEN", TOKEN.to_owned()),
            ("TEST_RUNTIME_ID", downstream.fingerprint.clone()),
            (
                "CCVL_CHECK_RUNTIME",
                downstream
                    .assets
                    .join("ccvl-linux-x86_64")
                    .display()
                    .to_string(),
            ),
            ("TEST_TRACE", downstream.trace.display().to_string()),
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
        // The fake runtime checks the arguments it is given against this value.
        env.insert("EXPECTED_UPSTREAM".to_owned(), self.upstream.clone());
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
        assert!(!combined.contains(TOKEN), "{combined}");
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
    // A Forgejo job leaves a depth-1 checkout; the check completes it itself
    // and derives the upstream commit it already contains.
    assert_eq!(
        String::from_utf8(fixture.git_in(
            &fixture.extracted,
            &["rev-parse", "--is-shallow-repository"]
        ))
        .unwrap()
        .trim(),
        "true"
    );
    let (output, trace) = fixture.invoke(true, &[]);
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["source_commit"], fixture.commit.as_str());
    assert_eq!(receipt["upstream_commit"], fixture.upstream.as_str());
    assert_eq!(receipt["checks"], json!(["downstream-check", "check"]));
    assert_eq!(receipt["merged"], false);
    assert_eq!(receipt["pushed"], false);
    assert_eq!(receipt["private"], true);
    assert_eq!(receipt["runtime_id"], fixture.fingerprint.as_str());
    assert_eq!(receipt["repository"], "fixture/private-downstream");
    // The privacy answer comes before anything else leaves the machine.
    assert_eq!(trace[0], "api");
    assert_eq!(trace[1], format!("ci:{}", fixture.upstream));
    assert_eq!(
        trace[trace.len() - 3..],
        ["runtime-id", "downstream-check", "check"]
    );
    // Receipts keep the established sorted, single line form.
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.starts_with("{\"binary_sha256\": \""), "{printed}");
}

#[test]
fn an_explicit_upstream_commit_needs_no_public_fetch() {
    let fixture = Downstream::new();
    let upstream = fixture.upstream.clone();
    let (output, _) = fixture.invoke(
        true,
        &[
            ("UPSTREAM_COMMIT", &upstream),
            ("CCVL_UPSTREAM_FETCH_URL", "/nonexistent/public.git"),
        ],
    );
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["upstream_commit"], upstream.as_str());
}

#[test]
fn public_hosted_manual_and_nonprivate_execution_are_refused_before_the_upstream_ci_is_asked() {
    let fixture = Downstream::new();
    for environment in [
        ("FORGEJO_ACTIONS", ""),
        ("RUNNER_ENVIRONMENT", "github-hosted"),
        ("GITHUB_REPOSITORY", "corbet-fsl/ccvl"),
        ("GITHUB_REPOSITORY", ""),
        ("GITHUB_EVENT_NAME", "push"),
        ("GITHUB_EVENT_NAME", "pull_request"),
    ] {
        let (_, trace) = fixture.invoke(false, &[environment]);
        assert!(trace.is_empty(), "{environment:?}");
    }
    // The forge says the repository is public: refused, after the one question.
    let (output, trace) = fixture.invoke(false, &[("TEST_PUBLIC", "1")]);
    assert_eq!(trace, ["api"]);
    assert!(stderr(&output).contains("require a private repository on a Forgejo runner"));
}

#[test]
fn publication_credentials_and_foreign_tokens_are_refused_without_disclosure() {
    let fixture = Downstream::new();
    for key in CREDENTIALS {
        let (output, trace) = fixture.invoke(false, &[(key, "PRIVATE_SENTINEL")]);
        assert!(trace.is_empty(), "{key}");
        assert!(
            stderr(&output).contains("Publication credentials are not accepted"),
            "{key}: {}",
            stderr(&output)
        );
    }
    // The job token is welcome; any other value under its names is a credential.
    for key in ["GITHUB_TOKEN", "FORGEJO_TOKEN", "CCID_FORGE_TOKEN"] {
        let (output, trace) = fixture.invoke(false, &[(key, "another-credential-value")]);
        assert!(trace.is_empty(), "{key}");
        assert!(
            stderr(&output).contains("must be the only token present"),
            "{key}: {}",
            stderr(&output)
        );
    }
    let (_, trace) = fixture.invoke(
        false,
        &[
            ("CCID_FORGE_TOKEN", ""),
            ("FORGEJO_TOKEN", ""),
            ("GITHUB_TOKEN", ""),
        ],
    );
    assert_eq!(trace, Vec::<String>::new());
}

#[test]
fn another_commit_and_changed_checkout_are_refused() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("CI_COMMIT_SHA", &fixture.upstream)]);
    assert_eq!(trace, [] as [std::string::String; 0]);
    assert!(stderr(&output).contains("not the requested private commit"));
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
fn nonancestor_upstream_is_refused() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("UPSTREAM_COMMIT", &"0".repeat(40))]);
    assert_eq!(trace, ["api"]);
    assert!(stderr(&output).contains("Upstream ancestry failed"));
}

#[test]
fn an_upstream_commit_without_a_green_ci_run_stops_before_private_gates() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("TEST_CI_RED", "1")]);
    assert!(
        stderr(&output).contains("The upstream commit has no successful ci run."),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        trace,
        vec!["api".to_owned(), format!("ci:{}", fixture.upstream)]
    );
    // An unreachable forge is a failed prerequisite, never a pass.
    let (output, trace) = fixture.invoke(false, &[("TEST_CI_UNAVAILABLE", "1")]);
    assert!(
        stderr(&output).contains("Upstream CI run unavailable"),
        "{}",
        stderr(&output)
    );
    assert!(!trace.iter().any(|line| line == "downstream-check"));
}

#[test]
fn a_binary_of_another_source_stops_before_private_gates() {
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("TEST_WRONG_RUNTIME", "1")]);
    assert!(
        stderr(&output).contains("was not built from this source"),
        "{}",
        stderr(&output)
    );
    assert!(!trace.iter().any(|line| line == "downstream-check"));
}

#[test]
fn without_a_named_binary_the_running_one_must_report_the_identity_of_the_source() {
    // No published runtime exists between releases: the gates run with the binary that is running. The test binary's own
    // identity is not the fixture source's, so the stage refuses before it runs any gate.
    let fixture = Downstream::new();
    let (output, trace) = fixture.invoke(false, &[("CCVL_CHECK_RUNTIME", "")]);
    assert!(
        stderr(&output).contains("was not built from this source"),
        "{}",
        stderr(&output)
    );
    assert!(
        !trace
            .iter()
            .any(|line| line == "downstream-check" || line == "check")
    );
    // The release of the source is never consulted any more.
    assert!(!trace.iter().any(|line| line.starts_with("download:")));
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
    // A policy that is not JSON can never be echoed into a log.
    let fixture = Downstream::with_policy(Some("PRIVATE_SENTINEL is not json"));
    let output = Command::new(BIN)
        .arg("private-check")
        .env_clear()
        .envs(fixture.env.clone())
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
