//! Guard fixtures for the release route: release identity, complete native
//! evidence and immutable uploads. They drive the real `ccvl release-evidence`
//! command and the real release scripts against a synthetic source tree.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ccvl::archive::{sha256_hex, write_with_comment};
use ccvl::release_evidence::{
    GATES, NATIVE_RESULT_CHECKS, PLATFORMS, ReleaseKind, parse_release_version,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_ccvl");
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const LFS_FILE: &str = "inputs/assets/lfs.bin";
/// The one source file the fixture commits as executable.
const EXECUTABLE: &str = ".agent/scripts/publish-release.sh";

fn source_mode(name: &str) -> u32 {
    if name == EXECUTABLE { 0o755 } else { 0o644 }
}

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn host_of(platform: &str) -> &'static str {
    PLATFORMS
        .iter()
        .find(|(name, _, _)| *name == platform)
        .unwrap()
        .1
}

fn file_hash(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap())
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn release_platforms() -> Vec<String> {
    fs::read_to_string(repository().join(".agent/release-platforms.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The platforms a release of that version holds: all of the declared set for `0.y.0`,
/// Linux `x86_64` alone for `0.y.z` with `z > 0`.
fn platforms_of(version: &str) -> Vec<String> {
    if version.ends_with(".0") {
        release_platforms()
    } else {
        vec!["linux-x86_64".to_owned()]
    }
}

struct Release {
    version: String,
    temp: TempDir,
    root: PathBuf,
    output: PathBuf,
    archive: PathBuf,
    env: Vec<(String, String)>,
    identity: Value,
}

impl Release {
    fn new() -> Self {
        Self::with_version("0.1.0")
    }

    fn with_version(version: &str) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("ccvl-release-test-")
            .tempdir()
            .unwrap();
        let root = temp.path().join("source");
        let output = temp.path().join("dist");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&output).unwrap();
        let mut files: Vec<(String, Vec<u8>)> = vec![
            (
                "Cargo.toml".into(),
                format!("[package]\nname=\"ccvl\"\nversion=\"{version}\"\nrust-version=\"1.94\"\n")
                    .into_bytes(),
            ),
            ("Cargo.lock".into(), b"fixture lock\n".to_vec()),
            (
                "rust-toolchain.toml".into(),
                b"[toolchain]\nchannel=\"stable\"\n".to_vec(),
            ),
            (".agent/build.rs".into(), b"fn main() {}\n".to_vec()),
            ("ccvl.json".into(), b"{}\n".to_vec()),
            (".agent/src/main.rs".into(), b"fn main() {}\n".to_vec()),
            (
                ".agent/core/Cargo.toml".into(),
                b"[package]\nname=\"ccvl-core\"\n".to_vec(),
            ),
            (
                ".agent/core/src/lib.rs".into(),
                b"pub fn fixture() {}\n".to_vec(),
            ),
            (
                ".agent/release-platforms.txt".into(),
                fs::read(repository().join(".agent/release-platforms.txt")).unwrap(),
            ),
        ];
        for name in [
            "release-evidence.sh",
            "publish-release.sh",
            "dispatch-leg.sh",
            "forge-draft.sh",
            "windows-attest.sh",
        ] {
            files.push((
                format!(".agent/scripts/{name}"),
                fs::read(repository().join(".agent/scripts").join(name)).unwrap(),
            ));
        }
        // The archive holds the committed LFS pointer; the checkout holds its object.
        let object = b"LFS OBJECT BYTES\n".to_vec();
        files.push((
            LFS_FILE.into(),
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\n",
                sha256_hex(&object),
                object.len()
            )
            .into_bytes(),
        ));
        for (name, payload) in &files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, if name == LFS_FILE { &object } else { payload }).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(source_mode(name))).unwrap();
        }
        let archive = temp.path().join("source.tar");
        fs::write(
            &archive,
            write_with_comment(
                Some(COMMIT),
                files
                    .iter()
                    .map(|(name, payload)| (name.as_str(), source_mode(name), payload.as_slice())),
            ),
        )
        .unwrap();
        let env = vec![
            ("FORGEJO_ACTIONS".to_owned(), "true".to_owned()),
            ("GITHUB_REPOSITORY".to_owned(), "corbet-fsl/ccvl".to_owned()),
            ("CI_COMMIT_SHA".to_owned(), COMMIT.to_owned()),
            ("GITHUB_RUN_ID".to_owned(), "123".to_owned()),
            ("SOURCE_ARCHIVE".to_owned(), archive.display().to_string()),
            ("SOURCE_SHA256".to_owned(), file_hash(&archive)),
        ];
        let mut release = Self {
            version: version.to_owned(),
            temp,
            root,
            output,
            archive,
            env,
            identity: Value::Null,
        };
        let result = release.run(&["source"], &[]);
        assert!(result.status.success(), "{}", text(&result));
        release.identity = serde_json::from_slice(&result.stdout).unwrap();
        release.populate();
        release
    }

    fn command(&self, extra: &[(&str, &str)]) -> Command {
        let mut command = Command::new(BIN);
        command
            .env_clear()
            .envs(
                std::env::vars()
                    .filter(|(key, _)| matches!(key.as_str(), "PATH" | "HOME" | "TMPDIR")),
            )
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .envs(extra.iter().copied())
            .current_dir(&self.root);
        command
    }

    fn run(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        self.command(extra)
            .arg("release-evidence")
            .args(args)
            .output()
            .unwrap()
    }

    fn check(&self) -> Output {
        self.run(&["check", "--output", self.output.to_str().unwrap()], &[])
    }

    /// The check a promotion makes: every leg has left its final evidence.
    fn check_complete(&self) -> Output {
        self.run(
            &[
                "check",
                "--complete",
                "--output",
                self.output.to_str().unwrap(),
            ],
            &[],
        )
    }

    fn identity_field(&self, field: &str) -> String {
        self.identity[field].as_str().unwrap().to_owned()
    }

    fn write(&self, name: &str, value: &Value) {
        fs::write(
            self.output.join(name),
            format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
        )
        .unwrap();
    }

    fn change(&self, name: &str, field: &str, value: &str) {
        let mut data = read_json(&self.output.join(name));
        data[field] = json!(value);
        self.write(name, &data);
    }

    /// What the Windows clone reports about the shipped `x86_64` executable.
    fn clone_result(&self) -> Value {
        let checks: Vec<Value> = NATIVE_RESULT_CHECKS
            .iter()
            .map(|id| json!({"id": id, "name": id, "ok": true, "detail": "fixture"}))
            .collect();
        json!({
            "schema": 1,
            "platform": "windows-x86_64",
            "rust_host": host_of("windows-x86_64"),
            "source_commit": COMMIT,
            "executable_sha256": file_hash(&self.output.join("ccvl-windows-x86_64.exe")),
            "runtime_id": self.identity_field("runtime_id"),
            "tests": {"executables": 5, "passed": 227, "failed": 0, "ignored": 0},
            "checks": checks,
        })
    }

    /// `attest` the Windows platform with `result` as the clone's report, in run `run`.
    fn attest_as(&self, platform: &str, result: &Value, run: &str) -> Output {
        let path = self.temp.path().join("clone-result.json");
        fs::write(&path, serde_json::to_vec_pretty(result).unwrap()).unwrap();
        self.run(
            &[
                "attest",
                "--platform",
                platform,
                "--result",
                path.to_str().unwrap(),
                "--output",
                self.output.to_str().unwrap(),
            ],
            &[("GITHUB_RUN_ID", run)],
        )
    }

    fn attest(&self, result: &Value) -> Output {
        self.attest_as("windows-x86_64", result, "456")
    }

    fn populate(&self) {
        for platform in platforms_of(&self.version) {
            // The macOS leg has not run when the draft is made: those platforms stay pending.
            if platform.starts_with("macos-") {
                continue;
            }
            self.write_platform(&platform);
        }
        self.write_gates();
    }

    /// The manifests are derived from the receipts: they are written again after a leg.
    fn forget_manifests(&self) {
        for name in ["manifest.json", "runtime-manifest.json"] {
            let _ = fs::remove_file(self.output.join(name));
        }
    }

    /// What the `release-windows` leg leaves in the draft: the clone's report and the native
    /// receipt that stands on it.
    fn finish_windows(&self) {
        let result = self.attest(&self.clone_result());
        assert!(result.status.success(), "{}", text(&result));
        self.forget_manifests();
    }

    /// What the `release-macos` leg leaves in the draft: the native receipt of `x86_64`
    /// (built and run in the macOS clone) and the cross receipt of ARM64.
    fn finish_macos(&self) {
        for platform in ["macos-x86_64", "macos-arm64"] {
            self.write_platform(platform);
        }
        self.forget_manifests();
    }

    fn finish_legs(&self) {
        self.finish_windows();
        self.finish_macos();
    }

    fn write_platform(&self, platform: &str) {
        let host = host_of(platform);
        let native = platform == "linux-x86_64" || platform == "macos-x86_64";
        let name = format!(
            "ccvl-{platform}{}",
            if platform.starts_with("windows-") {
                ".exe"
            } else {
                ""
            }
        );
        let mut assets = serde_json::Map::new();
        for filename in [name.clone(), format!("ccvl-{platform}.tar.gz")] {
            let payload = format!("fixture {filename}");
            fs::write(self.output.join(&filename), &payload).unwrap();
            let checksum = sha256_hex(payload.as_bytes());
            fs::write(
                self.output.join(format!("{filename}.sha256")),
                format!("{checksum}  {filename}\n"),
            )
            .unwrap();
            assets.insert(filename, json!(checksum));
        }
        fs::write(
            self.output.join(format!("{name}.runtime-id")),
            format!("{}\n", self.identity_field("runtime_id")),
        )
        .unwrap();
        let mut receipt = self.identity.clone();
        for (key, value) in [
            ("kind", json!(if native { "native" } else { "cross" })),
            ("platform", json!(platform)),
            ("rust_host", json!(host)),
            ("rustc", json!("rustc 1.97.0 fixture")),
            ("cargo", json!("cargo 1.97.0 fixture")),
            (
                "checks",
                if native {
                    json!(["public-check", "native-runtime"])
                } else {
                    json!(["cross-build"])
                },
            ),
            ("assets", Value::Object(assets)),
            ("provider", json!("forgejo")),
            ("run", json!("123")),
        ] {
            receipt[key] = value;
        }
        if !native {
            receipt["tested"] = json!(false);
            receipt["tool"] = json!("cargo-zigbuild fixture");
        }
        self.write(&format!("ccvl-{platform}.receipt.json"), &receipt);
    }

    fn write_gates(&self) {
        for gate in GATES {
            let mut receipt = self.identity.clone();
            for (key, value) in [
                ("kind", json!("gate")),
                ("gate", json!(gate)),
                ("provider", json!("forgejo")),
                ("run", json!("123")),
            ] {
                receipt[key] = value;
            }
            if gate == "linux-deep" || gate == "archive" {
                receipt["native_receipt_sha256"] = json!(file_hash(
                    &self.output.join("ccvl-linux-x86_64.receipt.json")
                ));
            }
            self.write(&format!("gate-{gate}.json"), &receipt);
        }
    }
}

fn assert_refused(output: &Output, message: &str) {
    let combined = text(output);
    assert!(!output.status.success(), "unexpected success: {combined}");
    assert!(
        combined.contains(message),
        "expected {message:?} in {combined:?}"
    );
}

#[test]
fn complete_explicit_native_results_and_gates() {
    let release = Release::new();
    let result = release.check();
    assert!(result.status.success(), "{}", text(&result));
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        format!("{COMMIT} {}\n", release.identity_field("runtime_id"))
    );
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(manifest["platforms"], json!(release_platforms()));
    // Tested where it ran, built (never run) where it could not, pending until
    // the macOS build clone exists.
    assert_eq!(
        manifest["platform_states"],
        json!({
            "linux-x86_64": "tested",
            "linux-aarch64": "built",
            "windows-x86_64": "built",
            "windows-arm64": "built",
            "macos-x86_64": "pending",
            "macos-arm64": "pending",
        })
    );
    assert_eq!(manifest["receipts"].as_object().unwrap().len(), 8);
    let runtime = read_json(&release.output.join("runtime-manifest.json"));
    assert_eq!(
        runtime["platforms"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        [
            "linux-aarch64",
            "linux-x86_64",
            "windows-arm64",
            "windows-x86_64"
        ]
    );
    assert_eq!(runtime["platform_states"]["macos-arm64"], "pending");
    assert_eq!(runtime["platforms"]["linux-aarch64"]["state"], "built");
    // The evidence files are byte stable: a repeat verifies and rewrites nothing.
    assert!(release.check().status.success());
}

#[test]
fn one_platform_absent_never_passes() {
    let release = Release::new();
    // Only a platform without an executor yet may be absent (pending).
    for platform in [
        "linux-x86_64",
        "linux-aarch64",
        "windows-x86_64",
        "windows-arm64",
    ] {
        release.populate();
        let receipt = release.output.join(format!("ccvl-{platform}.receipt.json"));
        let kept = fs::read(&receipt).unwrap();
        fs::remove_file(&receipt).unwrap();
        let result = release.check();
        assert!(!result.status.success(), "{platform}: {}", text(&result));
        fs::write(&receipt, kept).unwrap();
    }
}

#[test]
fn built_platforms_never_claim_tests_and_native_never_hides_a_cross_build() {
    let release = Release::new();
    // An ARM64 platform is only ever built: a native receipt for it is refused.
    release.change("ccvl-linux-aarch64.receipt.json", "kind", "native");
    assert_refused(&release.check(), "native platform");
    release.populate();
    // Linux x86_64 runs here: a cross receipt can not stand in for it.
    release.change("ccvl-linux-x86_64.receipt.json", "kind", "cross");
    assert_refused(&release.check(), "native platform");
    release.populate();
    // A cross receipt that claims its executable was tested is incomplete.
    let mut data = read_json(&release.output.join("ccvl-windows-arm64.receipt.json"));
    data["tested"] = json!(true);
    release.write("ccvl-windows-arm64.receipt.json", &data);
    assert_refused(&release.check(), "Incomplete cross-build evidence");
    release.populate();
    // x86_64 Windows is tested only on the report of the executor that ran it: a receipt
    // edited by hand into a native one holds no such report.
    let mut data = read_json(&release.output.join("ccvl-windows-x86_64.receipt.json"));
    data["kind"] = json!("native");
    data["checks"] = json!(["public-check", "native-runtime"]);
    data.as_object_mut().unwrap().remove("tested");
    data.as_object_mut().unwrap().remove("tool");
    release.write("ccvl-windows-x86_64.receipt.json", &data);
    assert_refused(&release.check(), "Missing native executor report");
}

#[test]
fn a_windows_executable_that_passed_in_its_clone_is_tested() {
    let release = Release::new();
    let cross = read_json(&release.output.join("ccvl-windows-x86_64.receipt.json"));
    let result = release.attest(&release.clone_result());
    assert!(result.status.success(), "{}", text(&result));
    let report = release
        .output
        .join("ccvl-windows-x86_64.native-result.json");
    let receipt = read_json(&release.output.join("ccvl-windows-x86_64.receipt.json"));
    assert_eq!(receipt["kind"], "native");
    assert_eq!(receipt["checks"], json!(["public-check", "native-runtime"]));
    assert!(receipt.get("tested").is_none());
    // The run that tested is the provenance; the run that built and its receipt stay on record.
    assert_eq!(
        (receipt["provider"].as_str(), receipt["run"].as_str()),
        (Some("forgejo"), Some("456"))
    );
    assert_eq!(receipt["built_by_run"], "123");
    assert_eq!(receipt["tool"], cross["tool"]);
    assert_eq!(receipt["assets"], cross["assets"]);
    assert_eq!(
        receipt["native_result"],
        "ccvl-windows-x86_64.native-result.json"
    );
    assert_eq!(receipt["native_result_sha256"], file_hash(&report));
    assert_eq!(read_json(&report)["tests"]["passed"], 227);
    // The manifests say so; nothing else changes state.
    let check = release.check();
    assert!(check.status.success(), "{}", text(&check));
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(
        manifest["platform_states"],
        json!({
            "linux-x86_64": "tested",
            "linux-aarch64": "built",
            "windows-x86_64": "tested",
            "windows-arm64": "built",
            "macos-x86_64": "pending",
            "macos-arm64": "pending",
        })
    );
    let runtime = read_json(&release.output.join("runtime-manifest.json"));
    assert_eq!(runtime["platforms"]["windows-x86_64"]["state"], "tested");
    assert_eq!(runtime["platform_states"]["windows-x86_64"], "tested");
    // Byte stable: a repeat verifies and rewrites nothing.
    assert!(release.check().status.success());
}

#[test]
fn the_executors_report_must_vouch_for_this_executable_and_pass_everything() {
    let release = Release::new();
    let refused = |result: &Value, message: &str| {
        assert_refused(&release.attest(result), message);
        // A refusal leaves the cross receipt and the release as they were.
        let cross = read_json(&release.output.join("ccvl-windows-x86_64.receipt.json"));
        assert_eq!(cross["kind"], "cross");
        assert!(
            !release
                .output
                .join("ccvl-windows-x86_64.native-result.json")
                .exists()
        );
    };
    let other = "b".repeat(64);
    for (field, value, message) in [
        ("executable_sha256", json!(other), "different executable"),
        ("runtime_id", json!(other), "different runtime identity"),
        (
            "source_commit",
            json!(other),
            "not about this platform and source",
        ),
        (
            "platform",
            json!("windows-arm64"),
            "not about this platform and source",
        ),
        (
            "rust_host",
            json!("x86_64-pc-windows-msvc"),
            "not about this platform and source",
        ),
        ("schema", json!(2), "not about this platform and source"),
    ] {
        let mut result = release.clone_result();
        result[field] = value;
        refused(&result, message);
    }
    for id in NATIVE_RESULT_CHECKS {
        let failing = |mutate: &dyn Fn(&mut Vec<Value>, usize)| {
            let mut result = release.clone_result();
            let checks = result["checks"].as_array_mut().unwrap();
            let index = checks.iter().position(|entry| entry["id"] == id).unwrap();
            mutate(checks, index);
            result
        };
        let message = format!("did not pass {id}");
        refused(
            &failing(&|checks, index| checks[index]["ok"] = json!(false)),
            &message,
        );
        refused(
            &failing(&|checks, index| {
                checks.remove(index);
            }),
            &message,
        );
        refused(
            &failing(&|checks, index| {
                let copy = checks[index].clone();
                checks.push(copy);
            }),
            &message,
        );
    }
    for tests in [
        json!({"passed": 227, "failed": 1}),
        json!({"passed": 0, "failed": 0}),
        json!({"failed": 0}),
        json!({}),
    ] {
        let mut result = release.clone_result();
        result["tests"] = tests;
        refused(&result, "did not pass completely");
    }
    let mut result = release.clone_result();
    result.as_object_mut().unwrap().remove("tests");
    refused(&result, "did not pass completely");
    // Where no executor exists or can not be attested, nothing is re-issued.
    for platform in ["linux-aarch64", "windows-arm64", "linux-x86_64"] {
        let message = "no native executor to attest";
        assert_refused(
            &release.attest_as(platform, &release.clone_result(), "456"),
            message,
        );
    }
}

#[test]
fn only_the_cross_receipt_of_the_executable_in_the_package_is_attested() {
    let release = Release::new();
    // An executable other than the one the receipt names.
    let exe = release.output.join("ccvl-windows-x86_64.exe");
    let kept = fs::read(&exe).unwrap();
    fs::write(&exe, "another executable").unwrap();
    assert_refused(
        &release.attest(&release.clone_result()),
        "Artifact checksum mismatch",
    );
    fs::write(&exe, kept).unwrap();
    // A receipt that already claims a test, or is not the cross receipt of the platform.
    let mut tampered = read_json(&release.output.join("ccvl-windows-x86_64.receipt.json"));
    tampered["tested"] = json!(true);
    release.write("ccvl-windows-x86_64.receipt.json", &tampered);
    assert_refused(
        &release.attest(&release.clone_result()),
        "Only the cross-build receipt",
    );
    release.populate();
    release.change(
        "ccvl-windows-x86_64.receipt.json",
        "source_commit",
        &"b".repeat(40),
    );
    assert_refused(&release.attest(&release.clone_result()), "source_commit");
    release.populate();
    // Attested once, a native receipt is not attested again.
    assert!(release.attest(&release.clone_result()).status.success());
    assert_refused(
        &release.attest(&release.clone_result()),
        "Only the cross-build receipt",
    );
}

#[test]
fn a_native_windows_receipt_stands_on_its_report_as_recorded() {
    let release = Release::new();
    assert!(release.attest(&release.clone_result()).status.success());
    assert!(release.check().status.success());
    let report = release
        .output
        .join("ccvl-windows-x86_64.native-result.json");
    let receipt_name = "ccvl-windows-x86_64.receipt.json";
    let kept = fs::read(&report).unwrap();
    // The report is altered, or missing: the receipt records its checksum.
    let mut edited = read_json(&report);
    edited["tests"]["failed"] = json!(3);
    release.write("ccvl-windows-x86_64.native-result.json", &edited);
    assert_refused(&release.check(), "report checksum mismatch");
    fs::remove_file(&report).unwrap();
    assert_refused(&release.check(), "Missing native executor report");
    // Report and receipt both edited to agree: the report still has to pass.
    release.write("ccvl-windows-x86_64.native-result.json", &edited);
    release.change(receipt_name, "native_result_sha256", &file_hash(&report));
    assert_refused(&release.check(), "did not pass completely");
    // A report of another executable, with a matching checksum.
    let mut other = release.clone_result();
    other["executable_sha256"] = json!("b".repeat(64));
    release.write("ccvl-windows-x86_64.native-result.json", &other);
    release.change(receipt_name, "native_result_sha256", &file_hash(&report));
    assert_refused(&release.check(), "different executable");
    // Restored, it passes again; a receipt that names another report does not.
    fs::write(&report, kept).unwrap();
    release.change(receipt_name, "native_result_sha256", &file_hash(&report));
    assert!(release.check().status.success());
    release.change(receipt_name, "native_result", "elsewhere.json");
    assert_refused(&release.check(), "Missing native executor report");
}

#[test]
fn macos_without_an_executor_is_pending_and_with_one_is_checked() {
    let release = Release::new();
    assert!(release.check().status.success());
    // A macOS receipt that exists is validated like any other.
    release.write("ccvl-macos-arm64.receipt.json", &json!({"kind": "cross"}));
    assert!(!release.check().status.success());
}

#[test]
fn a_complete_check_needs_the_final_evidence_of_every_leg() {
    let release = Release::new();
    // A fresh draft awaits its legs: macOS has no receipt yet, Windows is only built.
    assert!(release.check().status.success());
    release.forget_manifests();
    assert_refused(
        &release.check_complete(),
        "Missing platform evidence: macos-x86_64 (its leg, release-macos, has not left a receipt)",
    );
    // The macOS leg alone is not enough: the shipped Windows executable was never run.
    release.finish_macos();
    assert!(release.check().status.success());
    release.forget_manifests();
    assert_refused(
        &release.check_complete(),
        "windows-x86_64 is only built: its leg, release-windows, has not tested it",
    );
    release.finish_windows();
    let result = release.check_complete();
    assert!(result.status.success(), "{}", text(&result));
    // Tested where an executor ran the platform, built where nothing can run it.
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(
        manifest["platform_states"],
        json!({
            "linux-x86_64": "tested",
            "linux-aarch64": "built",
            "windows-x86_64": "tested",
            "windows-arm64": "built",
            "macos-x86_64": "tested",
            "macos-arm64": "built",
        })
    );
    // The manifests of a complete set are the ones the interim check writes for it: a repeat
    // of either verifies and rewrites nothing.
    assert!(release.check().status.success());
    assert!(release.check_complete().status.success());
}

#[test]
fn macos_x86_64_is_complete_only_when_it_ran_in_the_clone() {
    let release = Release::new();
    release.finish_legs();
    let receipt = "ccvl-macos-x86_64.receipt.json";
    let mut data = read_json(&release.output.join(receipt));
    data["kind"] = json!("cross");
    data["checks"] = json!(["cross-build"]);
    data["tested"] = json!(false);
    data["tool"] = json!("cross fixture");
    release.write(receipt, &data);
    // Built is a state of its own, fine for a draft and refused for a promotion.
    assert!(release.check().status.success());
    release.forget_manifests();
    assert_refused(
        &release.check_complete(),
        "macos-x86_64 is only built: its leg, release-macos, has not tested it",
    );
    // One ARM64 platform missing is incomplete too.
    release.finish_macos();
    fs::remove_file(release.output.join("ccvl-macos-arm64.receipt.json")).unwrap();
    assert_refused(
        &release.check_complete(),
        "Missing platform evidence: macos-arm64",
    );
}

#[test]
fn a_patch_release_is_complete_without_any_leg() {
    let release = Release::with_version("0.1.1");
    let result = release.check_complete();
    assert!(result.status.success(), "{}", text(&result));
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(manifest["platforms"], json!(["linux-x86_64"]));
}

#[test]
fn cross_host_cannot_claim_native_platform() {
    let release = Release::new();
    release.change(
        "ccvl-linux-x86_64.receipt.json",
        "rust_host",
        "aarch64-apple-darwin",
    );
    assert_refused(&release.check(), "native platform");
}

#[test]
fn different_lock_or_source_or_binary_refused() {
    let release = Release::new();
    for field in [
        "source_commit",
        "source_sha256",
        "cargo_lock_sha256",
        "runtime_id",
    ] {
        release.populate();
        release.change("ccvl-linux-x86_64.receipt.json", field, &"b".repeat(64));
        assert_refused(&release.check(), field);
    }
    release.populate();
    fs::write(release.output.join("ccvl-linux-x86_64"), b"changed binary").unwrap();
    assert_refused(&release.check(), "checksum");
}

#[test]
fn missing_gate_and_wrong_linux_receipt_refused() {
    let release = Release::new();
    fs::remove_file(release.output.join("gate-archive.json")).unwrap();
    assert!(!release.check().status.success());
    release.populate();
    release.change(
        "gate-linux-deep.json",
        "native_receipt_sha256",
        &"0".repeat(64),
    );
    assert_refused(&release.check(), "different native package");
}

#[test]
fn artifact_availability_cannot_expand_release_scope() {
    let release = Release::new();
    release.write("ccvl-freebsd-x86_64.receipt.json", &json!({}));
    assert_refused(
        &release.check(),
        "outside the explicit release platform policy",
    );
}

#[test]
fn the_version_alone_names_the_release_kind() {
    for (version, kind) in [
        ("0.0.0", ReleaseKind::Minor),
        ("0.1.0", ReleaseKind::Minor),
        ("0.12.0", ReleaseKind::Minor),
        ("0.0.1", ReleaseKind::Patch),
        ("0.1.1", ReleaseKind::Patch),
        ("0.3.10", ReleaseKind::Patch),
    ] {
        let parsed = parse_release_version(version).unwrap();
        assert_eq!((parsed.version.as_str(), parsed.kind), (version, kind));
    }
    // Valid semantic versions that name no release, and strings that are not versions.
    for version in [
        "1.0.0",
        "1.2.3",
        "10.0.0",
        "0.1",
        "0.1.0.0",
        "0.1.0-rc.1",
        "0.1.0+build.5",
        "0.01.0",
        "0.1.00",
        "v0.1.0",
        "0.1.x",
        "0..1",
        "",
        " 0.1.0",
    ] {
        assert!(parse_release_version(version).is_err(), "{version:?}");
    }
}

#[test]
fn a_patch_release_holds_linux_x86_64_alone() {
    let release = Release::with_version("0.1.1");
    let platforms = release.run(&["platforms"], &[]);
    assert!(platforms.status.success(), "{}", text(&platforms));
    assert_eq!(String::from_utf8_lossy(&platforms.stdout), "linux-x86_64\n");
    let version = release.run(&["version"], &[]);
    assert_eq!(String::from_utf8_lossy(&version.stdout), "0.1.1 patch\n");
    let result = release.check();
    assert!(result.status.success(), "{}", text(&result));
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(manifest["platforms"], json!(["linux-x86_64"]));
    assert_eq!(
        manifest["platform_states"],
        json!({"linux-x86_64": "tested"})
    );
    assert_eq!(manifest["version"], "0.1.1");
    assert_eq!(manifest["release_kind"], "patch");
    let runtime = read_json(&release.output.join("runtime-manifest.json"));
    assert_eq!(runtime["version"], "0.1.1");
    assert_eq!(runtime["platforms"].as_object().unwrap().len(), 1);
    // A build of another platform is outside the policy of this version.
    let minor = Release::new();
    fs::copy(
        minor.output.join("ccvl-windows-x86_64.receipt.json"),
        release.output.join("ccvl-windows-x86_64.receipt.json"),
    )
    .unwrap();
    assert_refused(
        &release.check(),
        "Native evidence outside the explicit release platform policy.",
    );
}

#[test]
fn a_minor_release_holds_every_declared_platform() {
    let release = Release::with_version("0.2.0");
    let version = release.run(&["version"], &[]);
    assert_eq!(String::from_utf8_lossy(&version.stdout), "0.2.0 minor\n");
    let result = release.check();
    assert!(result.status.success(), "{}", text(&result));
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(manifest["platforms"], json!(release_platforms()));
    assert_eq!(manifest["release_kind"], "minor");
    assert_eq!(manifest["version"], "0.2.0");
}

#[test]
fn a_version_that_names_no_release_fails_the_command_and_the_check() {
    for version in ["1.0.0", "0.1.0-rc.1"] {
        let release = Release::with_version(version);
        for arguments in [
            vec!["version"],
            vec!["platforms"],
            vec!["check", "--output", release.output.to_str().unwrap()],
        ] {
            let result = release.run(&arguments, &[]);
            assert!(!result.status.success(), "{version} {arguments:?}");
            assert!(
                text(&result).contains("releasable semantic version")
                    || text(&result).contains("1.0 or later"),
                "{version}: {}",
                text(&result)
            );
        }
    }
}

#[test]
fn invalid_platform_policy_refused() {
    let release = Release::new();
    for policy in [
        "",
        "macos-arm64\n",
        "linux-x86_64\nlinux-x86_64\n",
        "linux-x86_64\ninvented\n",
    ] {
        fs::write(release.root.join(".agent/release-platforms.txt"), policy).unwrap();
        assert_refused(
            &release.run(&["platforms"], &[]),
            "Invalid release platform policy",
        );
    }
    fs::write(
        release.root.join(".agent/release-platforms.txt"),
        "linux-x86_64\nmacos-arm64\n",
    )
    .unwrap();
    let result = release.run(&["platforms"], &[]);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "linux-x86_64\nmacos-arm64\n"
    );
}

#[test]
fn source_archive_or_worktree_mutation_refused() {
    let release = Release::new();
    assert_refused(
        &release.run(&["source"], &[("SOURCE_SHA256", &"0".repeat(64))]),
        "archive checksum",
    );
    fs::write(release.root.join("Cargo.lock"), "changed lock").unwrap();
    assert_refused(&release.run(&["source"], &[]), "Source input changed");
}

#[test]
fn an_lfs_object_must_be_the_one_its_pointer_names() {
    let release = Release::new();
    // The object is accepted (the fixture already holds it); other bytes of the
    // same size are not, and neither are the pointer's own bytes.
    fs::write(release.root.join(LFS_FILE), b"LFS OBJECT BYTES!").unwrap();
    assert_refused(
        &release.run(&["source"], &[]),
        "Source input changed: inputs/assets/lfs.bin",
    );
    fs::write(release.root.join(LFS_FILE), b"other bytes....\n").unwrap();
    assert_refused(&release.run(&["source"], &[]), "Source input changed");
}

#[test]
fn executable_source_mode_change_refused() {
    let release = Release::new();
    fs::set_permissions(
        release.root.join("Cargo.lock"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_refused(&release.run(&["source"], &[]), "executable mode");
}

#[test]
fn only_whether_a_source_file_is_executable_matters_not_its_group_and_other_bits() {
    let release = Release::new();
    let script = release.root.join(EXECUTABLE);
    // A checkout made under a restrictive umask: the macOS clone's user has 077.
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let result = release.run(&["source"], &[]);
    assert!(result.status.success(), "{}", text(&result));
    // The identity is the one the archive names, whatever bits the checkout carries.
    let identity: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(identity, release.identity);
    // Not executable at all is a change, whoever's umask it was.
    fs::set_permissions(&script, fs::Permissions::from_mode(0o600)).unwrap();
    assert_refused(&release.run(&["source"], &[]), "executable mode");
}

#[test]
fn private_repository_refused() {
    let release = Release::new();
    assert_refused(
        &release.run(
            &["source"],
            &[("GITHUB_REPOSITORY", "corbet-labs/applications")],
        ),
        "public",
    );
}

#[test]
fn a_clean_checkout_without_an_archive_has_the_same_identity() {
    let release = Release::new();
    let checkout = release.temp.path().join("checkout");
    fs::create_dir_all(&checkout).unwrap();
    let tree = ccvl::archive::read(&fs::read(&release.archive).unwrap()).unwrap();
    for entry in &tree.entries {
        let path = checkout.join(&entry.name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &entry.data).unwrap();
    }
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&checkout)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", text(&output));
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--quiet", "--template="]);
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "fixture"]);
    let head = git(&["rev-parse", "HEAD"]);
    let mut command = release.command(&[("CI_COMMIT_SHA", &head)]);
    command
        .env_remove("SOURCE_ARCHIVE")
        .env_remove("SOURCE_SHA256")
        .current_dir(&checkout)
        .args(["release-evidence", "source"]);
    let result = command.output().unwrap();
    assert!(result.status.success(), "{}", text(&result));
    let identity: Value = serde_json::from_slice(&result.stdout).unwrap();
    // Provider archive encoding never changes the runtime identity. Git archive
    // modes follow its own umask, so the mode sensitive source digest is only
    // required to be present.
    assert_eq!(identity["source_sha256"].as_str().map(str::len), Some(64));
    for field in ["runtime_id", "cargo_lock_sha256"] {
        assert_eq!(identity[field], release.identity[field], "{field}");
    }
    fs::write(checkout.join("Cargo.lock"), "dirty\n").unwrap();
    let mut dirty = release.command(&[("CI_COMMIT_SHA", &head)]);
    dirty
        .env_remove("SOURCE_ARCHIVE")
        .env_remove("SOURCE_SHA256")
        .current_dir(&checkout)
        .args(["release-evidence", "source"]);
    assert_refused(&dirty.output().unwrap(), "must be clean");
}

#[test]
fn source_prints_sorted_json_and_writes_indented_files() {
    let release = Release::new();
    let result = release.run(&["source"], &[]);
    let printed = String::from_utf8(result.stdout).unwrap();
    assert!(
        printed.starts_with("{\"cargo_lock_sha256\": \""),
        "{printed}"
    );
    assert!(printed.ends_with("}\n"));
    let target = release.temp.path().join("identity.json");
    let result = release.run(&["source", "--output", target.to_str().unwrap()], &[]);
    assert!(result.status.success(), "{}", text(&result));
    let written = fs::read_to_string(&target).unwrap();
    assert!(
        written.starts_with("{\n  \"cargo_lock_sha256\": \""),
        "{written}"
    );
    assert!(written.ends_with("\"\n}\n"));
    fs::write(&target, "different\n").unwrap();
    assert_refused(
        &release.run(&["source", "--output", target.to_str().unwrap()], &[]),
        "Refusing to replace differing evidence",
    );
}

#[test]
fn native_packaging_flow_produces_checkable_evidence() {
    let release = Release::new();
    let binary = release.temp.path().join("tested-ccvl");
    fs::write(
        &binary,
        format!(
            "#!/usr/bin/env bash\n[[ $1 == runtime-id ]] && echo {}\n",
            release.identity_field("runtime_id")
        ),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let dist = release.temp.path().join("fresh");
    let native = |binary: &Path, host: &str| {
        release.run(
            &[
                "native",
                "--platform",
                "linux-x86_64",
                "--binary",
                binary.to_str().unwrap(),
                "--host",
                host,
                "--rustc",
                "rustc 1.97.0 fixture",
                "--cargo",
                "cargo 1.97.0 fixture",
                "--output",
                dist.to_str().unwrap(),
            ],
            &[],
        )
    };
    assert_refused(
        &native(&binary, "aarch64-apple-darwin"),
        "Rust compiler host does not match",
    );
    let stale = release.temp.path().join("stale-ccvl");
    fs::write(&stale, "#!/usr/bin/env bash\necho nope\n").unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o755)).unwrap();
    assert_refused(
        &native(&stale, "x86_64-unknown-linux-gnu"),
        "Compiled runtime differs from the verified source.",
    );
    let result = native(&binary, "x86_64-unknown-linux-gnu");
    assert!(result.status.success(), "{}", text(&result));
    for name in [
        "ccvl-linux-x86_64",
        "ccvl-linux-x86_64.sha256",
        "ccvl-linux-x86_64.runtime-id",
        "ccvl-linux-x86_64.tar.gz",
        "ccvl-linux-x86_64.tar.gz.sha256",
        "ccvl-linux-x86_64.receipt.json",
    ] {
        assert!(dist.join(name).is_file(), "{name}");
    }
    assert_eq!(
        fs::metadata(dist.join("ccvl-linux-x86_64"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let listing = Command::new("tar")
        .args(["-tzvf"])
        .arg(dist.join("ccvl-linux-x86_64.tar.gz"))
        .output()
        .unwrap();
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(listing.contains(".agent/cache/ccvl/bin/ccvl"), "{listing}");
    assert!(listing.contains("Cargo.lock"), "{listing}");
    assert_refused(
        &native(&binary, "x86_64-unknown-linux-gnu"),
        "Package already exists",
    );
    // The platforms nothing here runs are cross-built into the same release;
    // the macOS pair stays pending until its build clone exists.
    let foreign = release.temp.path().join("cross-ccvl");
    fs::write(
        &foreign,
        format!("fixture\0{}\0tail", release.identity_field("runtime_id")),
    )
    .unwrap();
    for (platform, host) in [
        ("linux-aarch64", "aarch64-unknown-linux-gnu"),
        ("windows-x86_64", "x86_64-pc-windows-gnullvm"),
        ("windows-arm64", "aarch64-pc-windows-gnullvm"),
    ] {
        let result = release.run(
            &[
                "cross",
                "--platform",
                platform,
                "--binary",
                foreign.to_str().unwrap(),
                "--host",
                host,
                "--rustc",
                "rustc 1.97.0 fixture",
                "--cargo",
                "cargo 1.97.0 fixture",
                "--tool",
                "cargo-zigbuild fixture",
                "--output",
                dist.to_str().unwrap(),
            ],
            &[],
        );
        assert!(result.status.success(), "{platform}: {}", text(&result));
    }
    // Each gate records the exact native package; the whole set then checks.
    let destination = dist.to_str().unwrap();
    for gate in GATES {
        let result = release.run(&["gate", gate, "--output", destination], &[]);
        assert!(result.status.success(), "{gate}: {}", text(&result));
    }
    let result = release.run(&["check", "--output", destination], &[]);
    assert!(result.status.success(), "{}", text(&result));
    let receipt = read_json(&dist.join("ccvl-linux-x86_64.receipt.json"));
    assert_eq!(receipt["provider"], "forgejo");
    assert_eq!(receipt["run"], "123");
    assert_eq!(receipt["checks"], json!(["public-check", "native-runtime"]));
}

#[test]
fn cross_packaging_records_a_built_receipt_and_refuses_false_claims() {
    let release = Release::new();
    let runtime_id = release.identity_field("runtime_id");
    let binary = release.temp.path().join("cross-ccvl.exe");
    // A foreign executable can not be run; it embeds the runtime id as a string.
    fs::write(&binary, format!("MZ-fixture\0{runtime_id}\0tail")).unwrap();
    let foreign = release.temp.path().join("foreign.exe");
    fs::write(&foreign, "MZ-fixture without identity").unwrap();
    let dist = release.temp.path().join("cross-dist");
    let cross = |platform: &str, binary: &Path, host: &str| {
        release.run(
            &[
                "cross",
                "--platform",
                platform,
                "--binary",
                binary.to_str().unwrap(),
                "--host",
                host,
                "--rustc",
                "rustc 1.97.0 fixture",
                "--cargo",
                "cargo 1.97.0 fixture",
                "--tool",
                "cargo-zigbuild fixture",
                "--output",
                dist.to_str().unwrap(),
            ],
            &[],
        )
    };
    assert_refused(
        &cross("windows-x86_64", &foreign, "x86_64-pc-windows-gnullvm"),
        "does not embed the verified source identity",
    );
    // One Windows ABI for both targets: neither the MSVC nor the old GNU triple is accepted.
    for other in ["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu"] {
        assert_refused(
            &cross("windows-x86_64", &binary, other),
            "Rust compiler host does not match",
        );
    }
    assert_refused(
        &cross("linux-x86_64", &binary, "x86_64-unknown-linux-gnu"),
        "requires native evidence",
    );
    let result = cross("windows-x86_64", &binary, "x86_64-pc-windows-gnullvm");
    assert!(result.status.success(), "{}", text(&result));
    for name in [
        "ccvl-windows-x86_64.exe",
        "ccvl-windows-x86_64.exe.sha256",
        "ccvl-windows-x86_64.exe.runtime-id",
        "ccvl-windows-x86_64.tar.gz",
        "ccvl-windows-x86_64.tar.gz.sha256",
        "ccvl-windows-x86_64.receipt.json",
    ] {
        assert!(dist.join(name).is_file(), "{name}");
    }
    let receipt = read_json(&dist.join("ccvl-windows-x86_64.receipt.json"));
    assert_eq!(receipt["kind"], "cross");
    assert_eq!(receipt["checks"], json!(["cross-build"]));
    assert_eq!(receipt["tested"], false);
    assert_eq!(receipt["tool"], "cargo-zigbuild fixture");
    assert_eq!(receipt["provider"], "forgejo");
    // An ARM64 platform is only ever built: the native command refuses it.
    let native = release.run(
        &[
            "native",
            "--platform",
            "windows-arm64",
            "--binary",
            binary.to_str().unwrap(),
            "--host",
            "aarch64-pc-windows-gnullvm",
            "--rustc",
            "rustc 1.97.0 fixture",
            "--cargo",
            "cargo 1.97.0 fixture",
            "--output",
            dist.to_str().unwrap(),
        ],
        &[],
    );
    assert_refused(&native, "built only and never run");
}

#[test]
fn a_run_identity_is_required_for_new_evidence() {
    let release = Release::new();
    let result = release.run(
        &[
            "gate",
            "rust",
            "--output",
            release.temp.path().join("gates").to_str().unwrap(),
        ],
        &[("GITHUB_RUN_ID", "")],
    );
    assert_refused(&result, "A Forgejo Actions run identity is required.");
}

/// The maintainers' publication scripts (`publish-release.sh`, `dispatch-leg.sh`,
/// `archive-gate.sh`) run on the Linux release host only: they need bash 4 (`mapfile`),
/// GNU `sha256sum --check --strict` and GNU `stat -c`, none of which macOS ships. Their
/// fixtures therefore run where the scripts do. Nothing here covers behaviour that is
/// cross-platform: the `ccvl release-evidence` command and its fixtures above run everywhere
/// this file does, and a macOS run of the guards below would only pass or fail on the host's
/// userland.
#[cfg(target_os = "linux")]
mod maintainer_scripts {
    use std::collections::BTreeMap;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use std::os::unix::fs::symlink;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    /// A Forgejo that keeps its releases in memory: just enough of the API for the
    /// publisher, plus faults a real forge produces.
    #[derive(Default)]
    struct ForgeState {
        head: String,
        next: u64,
        next_asset: u64,
        releases: Vec<ForgeRelease>,
        tags: BTreeMap<String, String>,
        list_error: bool,
        /// The forge refuses to start a workflow.
        dispatch_refused: bool,
        /// Uploads the forge accepts the whole body of and then resets, before storing it.
        drop_uploads: usize,
        /// Uploads of which the forge stores half the body and then resets.
        truncate_uploads: usize,
        requests: Vec<String>,
    }

    struct ForgeRelease {
        id: u64,
        tag: String,
        title: String,
        draft: bool,
        prerelease: bool,
        target: String,
        body: String,
        assets: Vec<(u64, String, Vec<u8>)>,
    }

    type Forge = Arc<Mutex<ForgeState>>;

    const FORGE_TOKEN: &str = "fixture-job-token";
    /// The runner's read-only credential: it can read the assets of a draft, which the job token can not.
    const READ_TOKEN: &str = "fixture-read-token";

    fn release_json(release: &ForgeRelease, port: u16) -> Value {
        json!({
            "id": release.id,
            "tag_name": release.tag,
            "name": release.title,
            "draft": release.draft,
            "prerelease": release.prerelease,
            "target_commitish": release.target,
            "body": release.body,
            "assets": release.assets.iter().map(|(id, name, bytes)| json!({
                "id": id,
                "uuid": id.to_string(),
                "name": name,
                "size": bytes.len(),
                "browser_download_url": format!("https://forge.example.invalid:{port}/attachments/{id}"),
            })).collect::<Vec<_>>(),
        })
    }

    fn serve(state: Forge) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    continue;
                }
                let mut length = 0usize;
                let mut role = Role::None;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if lower.trim() == format!("authorization: token {FORGE_TOKEN}") {
                        role = Role::Job;
                    } else if lower.trim() == format!("authorization: token {READ_TOKEN}") {
                        role = Role::Reader;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap().to_owned();
                let target = parts.next().unwrap().to_owned();
                let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
                let mut state = state.lock().unwrap();
                state.requests.push(format!("{method} {path}"));
                let (status, payload): (u16, Vec<u8>) = match role {
                    Role::None => (401, b"{}".to_vec()),
                    Role::Reader if method != "GET" => (403, b"{}".to_vec()),
                    _ => respond(&mut state, port, role, &method, path, query, &body),
                };
                if status == 0 {
                    // A reset: the connection closes without an answer.
                    continue;
                }
                let header = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&payload);
            }
        });
        port
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Role {
        None,
        Job,
        Reader,
    }

    fn respond(
        state: &mut ForgeState,
        port: u16,
        role: Role,
        method: &str,
        path: &str,
        query: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        let ok = |value: Value| (200, value.to_string().into_bytes());
        let base = "/api/v1/repos/corbet-fsl/ccvl";
        if let Some(id) = path.strip_prefix("/attachments/") {
            let id: u64 = id.parse().unwrap();
            // The assets of a draft are for the read credential alone.
            return state
                .releases
                .iter()
                .filter(|release| !release.draft || role == Role::Reader)
                .flat_map(|release| &release.assets)
                .find(|(asset, _, _)| *asset == id)
                .map_or((404, b"{}".to_vec()), |(_, _, bytes)| (200, bytes.clone()));
        }
        let Some(rest) = path.strip_prefix(base) else {
            return (404, b"{}".to_vec());
        };
        match (method, rest) {
            ("GET", "/branches/main") => ok(json!({"commit": {"id": state.head}})),
            ("GET", "/releases") => {
                if state.list_error {
                    return (500, b"{}".to_vec());
                }
                let page = if query.contains("page=1") {
                    state
                        .releases
                        .iter()
                        .map(|release| release_json(release, port))
                        .collect()
                } else {
                    Vec::new()
                };
                ok(Value::Array(page))
            }
            ("POST", "/releases") => {
                let request: Value = serde_json::from_slice(body).unwrap();
                assert_eq!(request["draft"], true, "releases are created as drafts");
                state.next += 1;
                let release = ForgeRelease {
                    id: state.next,
                    tag: request["tag_name"].as_str().unwrap().to_owned(),
                    title: request["name"].as_str().unwrap().to_owned(),
                    draft: true,
                    prerelease: request["prerelease"].as_bool().unwrap(),
                    target: request["target_commitish"].as_str().unwrap().to_owned(),
                    body: request["body"].as_str().unwrap().to_owned(),
                    assets: Vec::new(),
                };
                let answer = release_json(&release, port);
                state.releases.push(release);
                (201, answer.to_string().into_bytes())
            }
            ("POST", workflow)
                if workflow.starts_with("/actions/workflows/")
                    && workflow.ends_with("/dispatches") =>
            {
                if state.dispatch_refused {
                    (404, b"{}".to_vec())
                } else {
                    (204, Vec::new())
                }
            }
            ("GET", tag) if tag.starts_with("/tags/") => state
                .tags
                .get(&tag[6..])
                .map_or((404, b"{}".to_vec()), |commit| {
                    ok(json!({"commit": {"sha": commit}}))
                }),
            (_, release) if release.starts_with("/releases/") => {
                let mut pieces = release["/releases/".len()..].split('/');
                let id: u64 = pieces.next().unwrap().parse().unwrap();
                let Some(index) = state.releases.iter().position(|release| release.id == id) else {
                    return (404, b"{}".to_vec());
                };
                match (method, pieces.next()) {
                    ("POST", Some("assets")) => {
                        if state.drop_uploads > 0 {
                            state.drop_uploads -= 1;
                            return (0, Vec::new());
                        }
                        let name = query.strip_prefix("name=").unwrap().to_owned();
                        if state.truncate_uploads > 0 {
                            state.truncate_uploads -= 1;
                            state.next_asset += 1;
                            let asset = 1000 + state.next_asset;
                            let release = &mut state.releases[index];
                            release
                                .assets
                                .push((asset, name, body[..body.len() / 2].to_vec()));
                            return (0, Vec::new());
                        }
                        state.next_asset += 1;
                        let asset = 1000 + state.next_asset;
                        let release = &mut state.releases[index];
                        assert!(
                            release
                                .assets
                                .iter()
                                .all(|(_, existing, _)| *existing != name),
                            "{name} uploaded twice"
                        );
                        release.assets.push((asset, name, body.to_vec()));
                        (201, b"{}".to_vec())
                    }
                    ("DELETE", Some("assets")) => {
                        let asset: u64 = pieces.next().unwrap().parse().unwrap();
                        state.releases[index]
                            .assets
                            .retain(|(existing, _, _)| *existing != asset);
                        (204, Vec::new())
                    }
                    ("PATCH", None) => {
                        let request: Value = serde_json::from_slice(body).unwrap();
                        let release = &mut state.releases[index];
                        // A draft has no tag yet, so a promotion may rename it.
                        if let Some(tag) = request["tag_name"].as_str() {
                            assert!(release.draft, "only a draft is renamed");
                            tag.clone_into(&mut release.tag);
                        }
                        if let Some(title) = request["name"].as_str() {
                            title.clone_into(&mut release.title);
                        }
                        if let Some(draft) = request["draft"].as_bool() {
                            release.draft = draft;
                        }
                        if let Some(prerelease) = request["prerelease"].as_bool() {
                            release.prerelease = prerelease;
                        }
                        if let Some(target) = request["target_commitish"].as_str() {
                            target.clone_into(&mut release.target);
                        }
                        if let Some(notes) = request["body"].as_str() {
                            notes.clone_into(&mut release.body);
                        }
                        let (tag, target) = (release.tag.clone(), release.target.clone());
                        let answer = release_json(release, port);
                        if !state.releases[index].draft {
                            state.tags.insert(tag, target);
                        }
                        ok(answer)
                    }
                    _ => (404, b"{}".to_vec()),
                }
            }
            _ => (404, b"{}".to_vec()),
        }
    }

    struct Publisher<'a> {
        release: &'a Release,
        forge: Forge,
        port: u16,
        read_token_file: PathBuf,
    }

    impl<'a> Publisher<'a> {
        fn new(release: &'a Release) -> Self {
            for tool in ["jq", "curl"] {
                assert!(
                    Command::new(tool).arg("--version").output().is_ok(),
                    "{tool} is required for actual publisher guard fixtures"
                );
            }
            let forge: Forge = Arc::new(Mutex::new(ForgeState {
                head: COMMIT.to_owned(),
                ..ForgeState::default()
            }));
            let port = serve(Arc::clone(&forge));
            let read_token_file = release.temp.path().join("read-token");
            fs::write(&read_token_file, format!("{READ_TOKEN}\n")).unwrap();
            Self {
                release,
                forge,
                port,
                read_token_file,
            }
        }

        fn run(&self, arguments: &[&str], changes: &[(&str, &str)]) -> Output {
            self.run_script("publish-release.sh", arguments, changes)
        }

        fn run_script(&self, script: &str, arguments: &[&str], changes: &[(&str, &str)]) -> Output {
            let server = format!("http://127.0.0.1:{}", self.port);
            for _ in 0..20 {
                let output = Command::new("bash")
                    .arg(self.release.root.join(".agent/scripts").join(script))
                    .args(arguments)
                    .env_clear()
                    .envs(
                        std::env::vars()
                            .filter(|(key, _)| matches!(key.as_str(), "PATH" | "HOME" | "TMPDIR")),
                    )
                    .envs(self.release.env.iter().map(|(key, value)| (key, value)))
                    .env("GITHUB_SERVER_URL", &server)
                    .env("CCID_FORGE_TOKEN", FORGE_TOKEN)
                    .env("GITHUB_TOKEN", FORGE_TOKEN)
                    .env("CCVL_UPLOAD_RETRY_DELAY", "0")
                    .env("CCVL_FORGE_READ_TOKEN_FILE", &self.read_token_file)
                    .env("CCVL_RELEASE_EVIDENCE_BIN", BIN)
                    .envs(changes.iter().copied())
                    .current_dir(&self.release.root)
                    .output();
                match output {
                    Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    other => return other.unwrap(),
                }
            }
            panic!("publisher script stayed busy");
        }

        fn publish(&self) -> Output {
            self.run(&[self.release.output.to_str().unwrap()], &[])
        }

        /// The Windows stage's second half, with `report` as what the clone wrote.
        fn attest_windows(&self, report: &Value, run: &str) -> Output {
            let path = self.release.temp.path().join("clone-report.json");
            fs::write(&path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
            self.run_script(
                "windows-attest.sh",
                &[path.to_str().unwrap()],
                &[("CI_COMMIT_SHA", COMMIT), ("GITHUB_RUN_ID", run)],
            )
        }

        /// Uploads and deletions so far: what a stage changed on the forge.
        fn writes(&self) -> usize {
            self.requests()
                .iter()
                .filter(|request| request.starts_with("POST") || request.starts_with("DELETE"))
                .count()
        }

        fn promote(&self) -> Output {
            self.run(&["--promote"], &[("CI_COMMIT_SHA", COMMIT)])
        }

        fn requests(&self) -> Vec<String> {
            self.forge.lock().unwrap().requests.clone()
        }

        fn uploads(&self) -> usize {
            self.requests()
                .iter()
                .filter(|request| request.starts_with("POST") && request.ends_with("/assets"))
                .count()
        }

        fn state<T>(&self, change: impl FnOnce(&mut ForgeState) -> T) -> T {
            change(&mut self.forge.lock().unwrap())
        }

        /// The internal name of the draft of this commit.
        fn draft_tag() -> String {
            format!("build-{COMMIT}")
        }

        fn preflight(&self) -> Output {
            self.run(&["--preflight"], &[("CI_COMMIT_SHA", COMMIT)])
        }

        /// The end of the release run and of every leg: start the next leg, if one is due.
        fn dispatch(&self) -> Output {
            self.run_script("dispatch-leg.sh", &[], &[("CI_COMMIT_SHA", COMMIT)])
        }

        /// The workflows the forge was asked to start, in order.
        fn dispatched(&self) -> Vec<String> {
            self.requests()
                .iter()
                .filter_map(|request| {
                    let path = request.strip_prefix("POST ")?;
                    let workflow = path.split("/actions/workflows/").nth(1)?;
                    Some(workflow.strip_suffix("/dispatches")?.to_owned())
                })
                .collect()
        }

        /// What a leg leaves in the draft: every file of the evidence directory the draft does
        /// not hold yet, and the receipts it replaced. The manifests stay as the release run
        /// made them; they are derived again at promotion.
        fn leave_in_draft(&self) {
            self.leave_files(|_| true);
        }

        /// The same, for the files of the evidence directory that `keep` selects.
        fn leave_files(&self, keep: impl Fn(&str) -> bool) {
            let tag = Self::draft_tag();
            for entry in fs::read_dir(&self.release.output).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "manifest.json" || name == "runtime-manifest.json" || !keep(&name) {
                    continue;
                }
                let bytes = fs::read(entry.path()).unwrap();
                self.state(|state| {
                    state.next_asset += 1;
                    let id = 1000 + state.next_asset;
                    let draft = release_mut(state, &tag);
                    match draft.assets.iter_mut().find(|(_, held, _)| *held == name) {
                        Some(asset) => asset.2 = bytes,
                        None => draft.assets.push((id, name, bytes)),
                    }
                });
            }
        }

        /// The macOS leg has run, whatever the Windows leg left in the draft: only the macOS
        /// files are added, so a receipt the attestation re-issued is not put back.
        fn finish_macos_leg(&self) {
            self.release.finish_macos();
            self.leave_files(|name| name.starts_with("ccvl-macos-"));
        }

        /// Both legs have run: the Windows clone has tested its executable, the macOS clone
        /// has built, tested and packaged its platforms.
        fn finish_legs(&self) {
            self.release.finish_legs();
            self.leave_in_draft();
        }
    }

    fn release_mut<'a>(state: &'a mut ForgeState, tag: &str) -> &'a mut ForgeRelease {
        state
            .releases
            .iter_mut()
            .find(|release| release.tag == tag)
            .unwrap()
    }

    #[test]
    fn publication_makes_one_complete_draft_and_repeat_is_immutable() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        // Four built platforms: 13 runtime files and 17 bundle and evidence files, in one release.
        let first_uploads = publisher.uploads();
        assert_eq!(first_uploads, 30);
        publisher.state(|state| {
            assert_eq!(state.releases.len(), 1);
            assert!(state.tags.is_empty(), "a draft creates no tag");
            let draft = &state.releases[0];
            assert_eq!(draft.tag, Publisher::draft_tag());
            assert!(draft.draft && !draft.prerelease);
            assert_eq!(draft.assets.len(), 30);
            assert!(
                draft.body.contains("ccvl 0.1.0 (minor release)")
                    && draft.body.contains("linux-x86_64 tested")
                    && draft.body.contains("linux-aarch64 built, not tested")
                    && draft.body.contains("windows-arm64 built, not tested")
                    && draft.body.contains("macos-arm64 pending"),
                "{}",
                draft.body
            );
        });
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(publisher.uploads(), first_uploads);
        let tag = Publisher::draft_tag();
        publisher.state(|state| {
            // Other bytes of the right size: a size mismatch would be a truncated upload.
            release_mut(state, &tag).assets[0].2[0] ^= 1;
        });
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("No overwrite attempted"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.uploads(), first_uploads);
    }

    fn asset(release: &ForgeRelease, name: &str) -> Vec<u8> {
        let held: Vec<_> = release
            .assets
            .iter()
            .filter(|(_, held, _)| held == name)
            .collect();
        assert_eq!(held.len(), 1, "{name} must be on the release exactly once");
        held[0].2.clone()
    }

    fn asset_json(release: &ForgeRelease, name: &str) -> Value {
        serde_json::from_slice(&asset(release, name)).unwrap()
    }

    #[test]
    fn the_windows_attestation_makes_the_draft_say_tested_and_promotion_agrees() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        let tag = Publisher::draft_tag();
        let cross = publisher
            .state(|state| asset(release_mut(state, &tag), "ccvl-windows-x86_64.receipt.json"));
        let result = publisher.attest_windows(&release.clone_result(), "456");
        assert!(result.status.success(), "{}", text(&result));
        assert!(
            text(&result).contains("windows-x86_64 is tested"),
            "{}",
            text(&result)
        );
        publisher.state(|state| {
            let draft = release_mut(state, &tag);
            assert!(draft.draft, "nothing is published");
            // The replaced receipt is kept, the report is kept, the rest is as it was: 30 + 2.
            assert_eq!(draft.assets.len(), 32);
            assert_eq!(
                asset(draft, "ccvl-windows-x86_64.cross-receipt.json"),
                cross
            );
            let receipt = asset_json(draft, "ccvl-windows-x86_64.receipt.json");
            assert_eq!(
                (receipt["kind"].as_str(), receipt["run"].as_str()),
                (Some("native"), Some("456"))
            );
            assert_eq!(receipt["built_by_run"], "123");
            let report = asset(draft, "ccvl-windows-x86_64.native-result.json");
            assert_eq!(receipt["native_result_sha256"], sha256_hex(&report));
            for name in ["manifest.json", "runtime-manifest.json"] {
                assert_eq!(
                    asset_json(draft, name)["platform_states"]["windows-x86_64"],
                    "tested",
                    "{name}"
                );
            }
        });
        // The macOS leg has run too; promotion checks the whole draft again, writes the
        // manifests again, and agrees.
        publisher.finish_macos_leg();
        let promoted = publisher.promote();
        assert!(promoted.status.success(), "{}", text(&promoted));
        publisher.state(|state| {
            let public = release_mut(state, "v0.1.0");
            assert!(!public.draft);
            assert!(
                public.body.contains("windows-x86_64 tested"),
                "{}",
                public.body
            );
            assert!(
                public.body.contains("windows-arm64 built, not tested"),
                "{}",
                public.body
            );
        });
    }

    #[test]
    fn the_windows_attestation_is_repeatable_and_changes_nothing_it_should_not() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        // A report of a failed clone, a stale main: refused before anything is written.
        let before = publisher.writes();
        let mut failed = release.clone_result();
        failed["checks"][5]["ok"] = json!(false);
        let result = publisher.attest_windows(&failed, "456");
        assert!(!result.status.success());
        assert!(
            text(&result).contains("did not pass test-suite"),
            "{}",
            text(&result)
        );
        publisher.state(|state| state.head = "b".repeat(40));
        let result = publisher.attest_windows(&release.clone_result(), "456");
        assert!(!result.status.success());
        assert!(
            text(&result).contains("no longer main"),
            "{}",
            text(&result)
        );
        publisher.state(|state| state.head = COMMIT.to_owned());
        assert_eq!(publisher.writes(), before, "a refusal writes nothing");
        // Done once; done again, it says so and writes nothing.
        assert!(
            publisher
                .attest_windows(&release.clone_result(), "456")
                .status
                .success()
        );
        let done = publisher.writes();
        let again = publisher.attest_windows(&release.clone_result(), "457");
        assert!(again.status.success(), "{}", text(&again));
        assert!(text(&again).contains("already native"), "{}", text(&again));
        assert_eq!(publisher.writes(), done, "a repeat writes nothing");
        publisher.finish_macos_leg();
        assert!(publisher.promote().status.success());
        // A published release is final: its evidence is never touched.
        let final_writes = publisher.writes();
        let late = publisher.attest_windows(&release.clone_result(), "459");
        assert!(!late.status.success());
        assert_eq!(publisher.writes(), final_writes);
    }

    #[test]
    fn a_stage_that_died_between_removing_and_uploading_the_receipt_starts_from_its_copy() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        let tag = Publisher::draft_tag();
        // What the forge holds after the stage removed the old receipt and died: the copy
        // it kept under another name, and no receipt.
        publisher.state(|state| {
            let draft = release_mut(state, &tag);
            let position = draft
                .assets
                .iter()
                .position(|(_, name, _)| name == "ccvl-windows-x86_64.receipt.json")
                .unwrap();
            let (_, _, bytes) = draft.assets.remove(position);
            draft.assets.push((
                9000,
                "ccvl-windows-x86_64.cross-receipt.json".to_owned(),
                bytes,
            ));
        });
        let healed = publisher.attest_windows(&release.clone_result(), "458");
        assert!(healed.status.success(), "{}", text(&healed));
        assert!(
            text(&healed).contains("starting from ccvl-windows-x86_64.cross-receipt.json"),
            "{}",
            text(&healed)
        );
        publisher.state(|state| {
            let draft = release_mut(state, &tag);
            assert_eq!(
                asset_json(draft, "ccvl-windows-x86_64.receipt.json")["run"],
                "458"
            );
            assert_eq!(
                asset_json(draft, "manifest.json")["platform_states"]["windows-x86_64"],
                "tested"
            );
        });
        publisher.finish_macos_leg();
        assert!(publisher.promote().status.success());
    }

    #[test]
    fn an_interrupted_upload_is_retried_and_never_duplicated() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        publisher.state(|state| state.drop_uploads = 3);
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        // Thirty files, plus the three attempts the forge reset.
        assert_eq!(publisher.uploads(), 33);
        publisher.state(|state| {
            assert_eq!(state.releases.len(), 1);
            assert_eq!(state.releases[0].assets.len(), 30);
        });
        // The forge never recovers: the publisher gives up with a clear message.
        let other = Release::new();
        let failing = Publisher::new(&other);
        failing.state(|state| state.drop_uploads = 1000);
        let result = failing.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("(attempt 4 of 4)"),
            "{}",
            text(&result)
        );
        assert_eq!(failing.uploads(), 4, "one asset, four attempts");
    }

    #[test]
    fn without_the_read_credential_a_draft_can_not_be_verified() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        fs::remove_file(&publisher.read_token_file).unwrap();
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("The forge answered 404 reading the asset"),
            "{}",
            text(&result)
        );
    }

    #[test]
    fn a_truncated_draft_asset_is_removed_and_uploaded_again() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        publisher.state(|state| state.truncate_uploads = 2);
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(publisher.uploads(), 32);
        let removed = publisher
            .requests()
            .iter()
            .filter(|request| request.starts_with("DELETE"))
            .count();
        assert_eq!(removed, 2, "only the two truncated stubs are deleted");
        // A leftover from an earlier run is cleaned the same way, before anything else.
        let other = Release::new();
        let resumed = Publisher::new(&other);
        assert!(resumed.publish().status.success());
        let tag = Publisher::draft_tag();
        resumed.state(|state| {
            let draft = release_mut(state, &tag);
            let asset = draft
                .assets
                .iter_mut()
                .find(|(_, name, _)| name == "ccvl-linux-x86_64")
                .unwrap();
            asset.2.truncate(3);
        });
        let result = resumed.publish();
        assert!(result.status.success(), "{}", text(&result));
        assert!(text(&result).contains("Removing the truncated draft asset ccvl-linux-x86_64"));
        // Right size, other bytes: refused, never replaced.
        resumed.state(|state| {
            let draft = release_mut(state, &tag);
            let asset = draft
                .assets
                .iter_mut()
                .find(|(_, name, _)| name == "ccvl-linux-x86_64")
                .unwrap();
            let last = asset.2.len() - 1;
            asset.2[last] ^= 1;
        });
        let result = resumed.publish();
        assert!(!result.status.success());
        assert!(text(&result).contains("differs"), "{}", text(&result));
    }

    #[test]
    fn a_draft_of_another_commit_is_never_retargeted() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        let draft = Publisher::draft_tag();
        publisher.state(|state| release_mut(state, &draft).target = "b".repeat(40));
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("targets a different source commit"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.uploads(), 30, "nothing is uploaded twice");
    }

    /// The checksums the release carries for the mirrors: `SHA256SUMS` lists every other asset.
    fn sums_of(assets: &[(u64, String, Vec<u8>)]) -> String {
        let mut lines: Vec<(String, String)> = assets
            .iter()
            .filter(|(_, name, _)| name != "SHA256SUMS")
            .map(|(_, name, bytes)| (name.clone(), format!("{}  {name}\n", sha256_hex(bytes))))
            .collect();
        lines.sort();
        lines.into_iter().map(|(_, line)| line).collect()
    }

    #[test]
    fn promotion_checks_the_draft_again_and_publishes_it_as_the_version() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        // Nothing to promote before the draft exists.
        assert!(!publisher.promote().status.success());
        assert!(publisher.publish().status.success());
        // The legs have not run: a minor release is not complete, and nothing changes.
        let uploads = publisher.uploads();
        let result = publisher.promote();
        assert_refused(&result, "Missing platform evidence: macos-x86_64");
        assert_eq!(publisher.uploads(), uploads);
        publisher.state(|state| {
            assert!(state.releases.iter().all(|release| release.draft));
            assert!(state.tags.is_empty());
        });
        // Both legs have left their evidence; the draft's manifests still describe the draft
        // the release run made (macOS pending, Windows built).
        publisher.finish_legs();
        let result = publisher.promote();
        assert!(result.status.success(), "{}", text(&result));
        let output = text(&result);
        assert!(output.contains("Promoted build-"), "{output}");
        // The legs ran after the draft was made, so the promotion derived the manifests
        // afresh and wrote the checksum file the mirrors need.
        for message in [
            "The draft's manifest.json is not what its receipts say; replacing it.",
            "The draft's runtime-manifest.json is not what its receipts say; replacing it.",
            "The draft's SHA256SUMS does not cover its assets; writing it.",
        ] {
            assert!(output.contains(message), "{message}: {output}");
        }
        assert!(!output.contains("downstream"), "{output}");
        let expected_assets = fs::read_dir(&release.output).unwrap().count() + 3;
        publisher.state(|state| {
            // One public release, named and tagged by the version, on this commit.
            assert_eq!(state.releases.len(), 1);
            let public = &state.releases[0];
            assert_eq!(public.tag, "v0.1.0");
            assert_eq!(public.title, "0.1.0");
            assert!(!public.draft && !public.prerelease);
            // The evidence directory, the two manifests derived again, and SHA256SUMS.
            assert_eq!(public.assets.len(), expected_assets);
            assert!(public.body.contains("ccvl 0.1.0 (minor release)"));
            for state_text in [
                "linux-x86_64 tested",
                "windows-x86_64 tested",
                "macos-x86_64 tested",
                "macos-arm64 built, not tested",
            ] {
                assert!(public.body.contains(state_text), "{}", public.body);
            }
            assert_eq!(state.tags.len(), 1);
            assert_eq!(state.tags["v0.1.0"], COMMIT);
            let asset = |name: &str| {
                let (_, _, bytes) = public
                    .assets
                    .iter()
                    .find(|(_, held, _)| held == name)
                    .unwrap_or_else(|| panic!("the release has no {name}"));
                bytes.clone()
            };
            // The manifests say what the receipts say.
            let manifest: Value = serde_json::from_slice(&asset("manifest.json")).unwrap();
            assert_eq!(
                manifest["platform_states"],
                json!({
                    "linux-x86_64": "tested",
                    "linux-aarch64": "built",
                    "windows-x86_64": "tested",
                    "windows-arm64": "built",
                    "macos-x86_64": "tested",
                    "macos-arm64": "built",
                })
            );
            // SHA256SUMS lists every other asset (the mirrors' pull job fails closed without an
            // entry), in coreutils format, in byte order, and not itself.
            let sums = String::from_utf8(asset("SHA256SUMS")).unwrap();
            assert_eq!(sums, sums_of(&public.assets));
            assert_eq!(sums.lines().count(), public.assets.len() - 1);
            assert!(sums.contains("  ccvl-macos-arm64.receipt.json\n"), "{sums}");
            assert!(sums.contains("  manifest.json\n"), "{sums}");
        });
        // A repeat verifies and changes nothing.
        let uploads = publisher.uploads();
        let result = publisher.promote();
        assert!(result.status.success(), "{}", text(&result));
        assert!(text(&result).contains("is public already"));
        assert_eq!(publisher.uploads(), uploads);
        // A checksum file that no longer covers the release is not accepted as the release.
        publisher.state(|state| {
            let public = &mut state.releases[0];
            let asset = public
                .assets
                .iter_mut()
                .find(|(_, name, _)| name == "SHA256SUMS")
                .unwrap();
            asset.2.extend_from_slice(b"stray\n");
        });
        assert_refused(&publisher.promote(), "SHA256SUMS");
        // A corrupted draft never becomes public.
        let other = Release::new();
        let second = Publisher::new(&other);
        assert!(second.publish().status.success());
        second.finish_legs();
        let tag = Publisher::draft_tag();
        second.state(|state| {
            let asset = release_mut(state, &tag)
                .assets
                .iter_mut()
                .find(|(_, name, _)| name == "ccvl-linux-x86_64")
                .unwrap();
            asset.2 = b"swapped binary".to_vec();
        });
        assert!(!second.promote().status.success());
        second.state(|state| {
            assert!(state.releases.iter().all(|release| release.draft));
            assert!(state.tags.is_empty());
        });
    }

    #[test]
    fn a_minor_release_is_promoted_only_when_both_legs_have_passed() {
        // The macOS leg without the Windows leg.
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        release.finish_macos();
        publisher.leave_in_draft();
        assert_refused(
            &publisher.promote(),
            "windows-x86_64 is only built: its leg, release-windows, has not tested it",
        );
        // The Windows leg without the macOS leg.
        let other = Release::new();
        let second = Publisher::new(&other);
        assert!(second.publish().status.success());
        other.finish_windows();
        second.leave_in_draft();
        assert_refused(
            &second.promote(),
            "Missing platform evidence: macos-x86_64 (its leg, release-macos, has not left a receipt)",
        );
        // Neither refusal published, tagged or changed anything on the forge.
        for forge in [&publisher, &second] {
            forge.state(|state| {
                assert!(state.releases.iter().all(|release| release.draft));
                assert!(state.tags.is_empty());
            });
        }
        // A patch release holds Linux x86_64 alone and has no leg to wait for.
        let patch = Release::with_version("0.1.1");
        let third = Publisher::new(&patch);
        assert!(third.publish().status.success());
        assert!(third.promote().status.success());
    }

    #[test]
    fn the_release_run_starts_the_first_leg_and_each_leg_the_next() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        // Without a draft there is nothing to continue.
        assert_refused(&publisher.dispatch(), "There is no draft build-");
        assert_eq!(publisher.dispatched(), [] as [&str; 0]);
        assert!(publisher.publish().status.success());
        // The end of the release run: the Windows leg is due.
        let result = publisher.dispatch();
        assert!(result.status.success(), "{}", text(&result));
        assert!(
            text(&result).contains("Started release-windows"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.dispatched(), ["release-windows.yml"]);
        // The Windows clone tested the executable: the macOS leg is due.
        release.finish_windows();
        publisher.leave_in_draft();
        let result = publisher.dispatch();
        assert!(result.status.success(), "{}", text(&result));
        assert!(
            text(&result).contains("Started release-macos"),
            "{}",
            text(&result)
        );
        // One macOS platform is not the leg: it is due until both left their receipt.
        release.write_platform("macos-x86_64");
        publisher.leave_in_draft();
        assert!(publisher.dispatch().status.success());
        assert_eq!(
            publisher.dispatched(),
            [
                "release-windows.yml",
                "release-macos.yml",
                "release-macos.yml"
            ]
        );
        release.finish_macos();
        publisher.leave_in_draft();
        // Both legs are done: nothing is started, and a re-run of a leg ends the chain here.
        let result = publisher.dispatch();
        assert!(result.status.success(), "{}", text(&result));
        assert!(
            text(&result).contains("release-promote can check the draft"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.dispatched().len(), 3);
    }

    #[test]
    fn a_patch_release_has_no_legs_and_a_stale_or_refused_dispatch_fails_visibly() {
        let patch = Release::with_version("0.1.1");
        let publisher = Publisher::new(&patch);
        assert!(publisher.publish().status.success());
        let result = publisher.dispatch();
        assert!(result.status.success(), "{}", text(&result));
        assert!(
            text(&result).contains("a patch release"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.dispatched(), [] as [&str; 0]);
        // Main moved on: the draft is stale and no leg is started for another commit.
        let release = Release::new();
        let stale = Publisher::new(&release);
        assert!(stale.publish().status.success());
        stale.state(|state| state.head = "b".repeat(40));
        assert_refused(&stale.dispatch(), "no longer main");
        stale.state(|state| state.head = COMMIT.to_owned());
        // The forge does not start the workflow: the run fails, it is not a quiet success.
        stale.state(|state| state.dispatch_refused = true);
        assert_refused(
            &stale.dispatch(),
            "did not start the workflow release-windows",
        );
        // The one request the forge refused; nothing else was asked of it.
        assert_eq!(stale.dispatched(), ["release-windows.yml"]);
    }

    #[test]
    fn a_version_is_released_once() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.preflight().status.success());
        assert!(publisher.publish().status.success());
        publisher.finish_legs();
        assert!(publisher.promote().status.success());
        let uploads = publisher.uploads();
        // The same version again: refused before anything is created or uploaded.
        for result in [publisher.preflight(), publisher.publish()] {
            assert!(!result.status.success());
            assert!(
                text(&result).contains("Release v0.1.0 exists"),
                "{}",
                text(&result)
            );
        }
        assert_eq!(publisher.uploads(), uploads);
        // A draft named for the version blocks it too.
        let other = Release::new();
        let drafted = Publisher::new(&other);
        drafted.state(|state| {
            state.releases.push(ForgeRelease {
                id: 900,
                tag: "v0.1.0".to_owned(),
                title: "0.1.0".to_owned(),
                draft: true,
                prerelease: false,
                target: COMMIT.to_owned(),
                body: String::new(),
                assets: Vec::new(),
            });
        });
        let result = drafted.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("Release v0.1.0 exists"),
            "{}",
            text(&result)
        );
        // So does a stray tag without a release.
        let third = Release::new();
        let tagged = Publisher::new(&third);
        tagged.state(|state| {
            state.tags.insert("v0.1.0".to_owned(), "b".repeat(40));
        });
        let result = tagged.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("A tag v0.1.0 exists"),
            "{}",
            text(&result)
        );
        assert_eq!(tagged.uploads(), 0);
        assert!(
            !tagged
                .requests()
                .iter()
                .any(|request| request.starts_with("POST"))
        );
    }

    #[test]
    fn a_version_that_names_no_release_is_refused_before_the_forge_is_asked() {
        for version in ["1.0.0", "0.1", "0.1.0-rc.1", "0.1.0+build.5", "0.01.0"] {
            let release = Release::with_version(version);
            let publisher = Publisher::new(&release);
            let result = publisher.preflight();
            assert!(!result.status.success(), "{version}: {}", text(&result));
            assert!(
                text(&result).contains("releasable semantic version")
                    || text(&result).contains("1.0 or later"),
                "{version}: {}",
                text(&result)
            );
            assert!(publisher.requests().is_empty(), "{version}");
        }
    }

    #[test]
    fn a_patch_release_drafts_and_promotes_linux_x86_64_alone() {
        let release = Release::with_version("0.1.1");
        let publisher = Publisher::new(&release);
        let preflight = publisher.preflight();
        assert!(preflight.status.success(), "{}", text(&preflight));
        assert!(
            text(&preflight).contains("v0.1.1 (patch release)")
                && text(&preflight).contains("Platforms: linux-x86_64."),
            "{}",
            text(&preflight)
        );
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        // linux-x86_64: 3 runtime files, 2 bundle files, its receipt, the two manifests and four gates.
        assert_eq!(publisher.uploads(), 12);
        let result = publisher.promote();
        assert!(result.status.success(), "{}", text(&result));
        publisher.state(|state| {
            assert_eq!(state.releases.len(), 1);
            assert_eq!(state.releases[0].tag, "v0.1.1");
            assert_eq!(state.tags["v0.1.1"], COMMIT);
            let names: Vec<&str> = state.releases[0]
                .assets
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect();
            assert!(
                names.iter().all(|name| !name.contains("windows")
                    && !name.contains("aarch64")
                    && !name.contains("arm64")
                    && !name.contains("macos")),
                "{names:?}"
            );
            assert!(state.releases[0].body.contains("(patch release)"));
        });
    }

    #[test]
    fn unknown_draft_lookup_never_creates_or_uploads() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        publisher.state(|state| state.list_error = true);
        assert!(!publisher.publish().status.success());
        let requests = publisher.requests();
        assert!(
            !requests.iter().any(|request| request.starts_with("POST")),
            "{requests:?}"
        );
    }

    #[test]
    fn existing_wrong_tag_prevents_any_publication() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        publisher.state(|state| {
            state.tags.insert(Publisher::draft_tag(), "b".repeat(40));
        });
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("targets a different source commit"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.uploads(), 0);
    }

    #[test]
    fn stale_main_and_missing_native_evidence_prevent_any_publication_call() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        publisher.state(|state| state.head = "b".repeat(40));
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("no longer main"),
            "{}",
            text(&result)
        );
        assert!(
            !publisher
                .requests()
                .iter()
                .any(|request| request.starts_with("POST"))
        );
        publisher.state(|state| state.head = COMMIT.to_owned());
        fs::remove_file(release.output.join("ccvl-linux-x86_64.receipt.json")).unwrap();
        let before = publisher.requests().len();
        assert!(!publisher.publish().status.success());
        assert_eq!(publisher.requests().len(), before);
    }

    /// The archive gate selects its minimal executor; the check itself stays the
    /// verified script of the checkout.
    struct ArchiveGate {
        temp: TempDir,
        dist: PathBuf,
        marker: PathBuf,
        executor: PathBuf,
    }

    impl ArchiveGate {
        fn new() -> Self {
            let temp = tempfile::Builder::new()
                .prefix("ccvl-archive-gate-test-")
                .tempdir()
                .unwrap();
            let root = temp.path().to_path_buf();
            let scripts = root.join("repo/.agent/scripts");
            fs::create_dir_all(&scripts).unwrap();
            let dist = root.join("package directory");
            fs::create_dir_all(&dist).unwrap();
            fs::copy(
                repository().join(".agent/scripts/archive-gate.sh"),
                scripts.join("archive-gate.sh"),
            )
            .unwrap();
            fs::write(
            scripts.join("check-release-archive.sh"),
            "#!/usr/bin/env bash\nset -eu\ntest \"$1\" = \"$EXPECTED_DIST\"\nprintf gate >> \"$GATE_MARKER\"\n",
        )
        .unwrap();
            // Stands in for the built-in minimal executor: it only records its call.
            fs::write(
            scripts.join("archive-executor.sh"),
            "#!/usr/bin/env bash\nset -eu\nprintf builtin > \"$EXECUTOR_MARKER\"\nexec bash \"$1\" \"$2\"\n",
        )
        .unwrap();
            let executor = root.join("external executor");
            fs::write(
            &executor,
            "#!/usr/bin/env bash\nset -eu\ntest \"$0\" != \"$ORIGINAL_EXECUTOR\"\nprintf external > \"$EXECUTOR_MARKER\"\nexec bash \"$1\" \"$2\"\n",
        )
        .unwrap();
            Self {
                marker: root.join("gate-called"),
                dist,
                executor,
                temp,
            }
        }

        fn invoke(&self, changes: &[(&str, &str)]) -> Output {
            let root = self.temp.path();
            for _ in 0..20 {
                let output = Command::new("bash")
                    .arg(root.join("repo/.agent/scripts/archive-gate.sh"))
                    .arg(&self.dist)
                    .env_clear()
                    .envs(
                        std::env::vars()
                            .filter(|(key, _)| matches!(key.as_str(), "PATH" | "HOME" | "TMPDIR")),
                    )
                    .env("EXPECTED_DIST", &self.dist)
                    .env("GATE_MARKER", &self.marker)
                    .env("EXECUTOR_MARKER", root.join("executor-called"))
                    .env("ORIGINAL_EXECUTOR", &self.executor)
                    .envs(changes.iter().copied())
                    .output();
                match output {
                    Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    other => return other.unwrap(),
                }
            }
            panic!("archive gate stayed busy");
        }

        fn executor_called(&self) -> Option<String> {
            fs::read_to_string(self.temp.path().join("executor-called")).ok()
        }
    }

    #[test]
    fn a_minimal_worker_runs_the_verified_check_directly_and_others_use_the_builtin_executor() {
        let fixture = ArchiveGate::new();
        let result = fixture.invoke(&[("CCVL_ARCHIVE_EXECUTOR", "direct")]);
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(fs::read_to_string(&fixture.marker).unwrap(), "gate");
        assert_eq!(fixture.executor_called(), None);
        let result = fixture.invoke(&[]);
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(fixture.executor_called().as_deref(), Some("builtin"));
    }

    #[test]
    fn verified_executor_snapshot_receives_unchanged_check_and_package() {
        let fixture = ArchiveGate::new();
        let executor = fixture.executor.display().to_string();
        let result = fixture.invoke(&[
            ("ARCHIVE_EXECUTOR", &executor),
            ("ARCHIVE_EXECUTOR_SHA256", &file_hash(&fixture.executor)),
        ]);
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(fs::read_to_string(&fixture.marker).unwrap(), "gate");
        assert_eq!(fixture.executor_called().as_deref(), Some("external"));
    }

    #[test]
    fn incomplete_changed_or_untrusted_executor_never_executes() {
        let fixture = ArchiveGate::new();
        let checksum = file_hash(&fixture.executor);
        let executor = fixture.executor.display().to_string();
        let link = fixture.temp.path().join("executor-link");
        symlink(&fixture.executor, &link).unwrap();
        let link = link.display().to_string();
        let zeros = "0".repeat(64);
        let cases: Vec<Vec<(&str, &str)>> = vec![
            vec![("ARCHIVE_EXECUTOR", &executor)],
            vec![("ARCHIVE_EXECUTOR_SHA256", &checksum)],
            vec![
                ("ARCHIVE_EXECUTOR", &executor),
                ("ARCHIVE_EXECUTOR_SHA256", &zeros),
            ],
            vec![
                ("ARCHIVE_EXECUTOR", &link),
                ("ARCHIVE_EXECUTOR_SHA256", &checksum),
            ],
        ];
        for inputs in cases {
            let result = fixture.invoke(&inputs);
            assert!(!result.status.success(), "{inputs:?}: {}", text(&result));
            assert!(!fixture.marker.exists());
            assert_eq!(fixture.executor_called(), None);
        }
    }
}
