//! Guard fixtures for the release route: release identity, complete native
//! evidence and immutable uploads. They drive the real `ccvl release-evidence`
//! command and the real release scripts against a synthetic source tree.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ccvl::archive::{sha256_hex, write_with_comment};
use ccvl::release_evidence::{GATES, PLATFORMS};
use serde_json::{Value, json};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_ccvl");
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const LFS_FILE: &str = "inputs/assets/lfs.bin";

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

struct Release {
    temp: TempDir,
    root: PathBuf,
    output: PathBuf,
    archive: PathBuf,
    env: Vec<(String, String)>,
    identity: Value,
}

impl Release {
    fn new() -> Self {
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
                b"[package]\nrust-version=\"1.94\"\n".to_vec(),
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
            "downstream-release.sh",
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
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let archive = temp.path().join("source.tar");
        fs::write(
            &archive,
            write_with_comment(
                Some(COMMIT),
                files
                    .iter()
                    .map(|(name, payload)| (name.as_str(), 0o644, payload.as_slice())),
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

    fn populate(&self) {
        for platform in release_platforms() {
            // The macOS build clone does not exist yet: those platforms stay pending.
            if platform.starts_with("macos-") {
                continue;
            }
            let host = host_of(&platform);
            let native = platform == "linux-x86_64";
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
    // x86_64 Windows becomes tested when its executor reports a native receipt.
    let mut data = read_json(&release.output.join("ccvl-windows-x86_64.receipt.json"));
    data["kind"] = json!("native");
    data["checks"] = json!(["public-check", "native-runtime"]);
    data.as_object_mut().unwrap().remove("tested");
    data.as_object_mut().unwrap().remove("tool");
    release.write("ccvl-windows-x86_64.receipt.json", &data);
    assert!(release.check().status.success());
    let manifest = read_json(&release.output.join("manifest.json"));
    assert_eq!(manifest["platform_states"]["windows-x86_64"], "tested");
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
        ("windows-x86_64", "x86_64-pc-windows-gnu"),
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
        &cross("windows-x86_64", &foreign, "x86_64-pc-windows-gnu"),
        "does not embed the verified source identity",
    );
    assert_refused(
        &cross("windows-x86_64", &binary, "x86_64-pc-windows-msvc"),
        "Rust compiler host does not match",
    );
    assert_refused(
        &cross("linux-x86_64", &binary, "x86_64-unknown-linux-gnu"),
        "requires native evidence",
    );
    let result = cross("windows-x86_64", &binary, "x86_64-pc-windows-gnu");
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

/// The maintainers' publication scripts (`publish-release.sh`, `downstream-release.sh`,
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
        /// Uploads the forge accepts the whole body of and then resets, before storing it.
        drop_uploads: usize,
        /// Uploads of which the forge stores half the body and then resets.
        truncate_uploads: usize,
        requests: Vec<String>,
    }

    struct ForgeRelease {
        id: u64,
        tag: String,
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
            let server = format!("http://127.0.0.1:{}", self.port);
            for _ in 0..20 {
                let output = Command::new("bash")
                    .arg(self.release.root.join(".agent/scripts/publish-release.sh"))
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

        fn runtime_tag(&self) -> String {
            format!("runtime-{}", self.release.identity_field("runtime_id"))
        }

        fn workspace_tag() -> String {
            format!("build-{COMMIT}")
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
    fn publication_makes_complete_drafts_and_repeat_is_immutable() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        // Four built platforms: 13 runtime files and 17 bundle and evidence files.
        let first_uploads = publisher.uploads();
        assert_eq!(first_uploads, 30);
        publisher.state(|state| {
            assert_eq!(state.releases.len(), 2);
            assert!(state.tags.is_empty(), "drafts create no tag");
            for release in &state.releases {
                assert!(release.draft, "{} must stay a draft", release.tag);
                assert!(
                    release.body.contains("linux-x86_64 tested")
                        && release.body.contains("linux-aarch64 built, not tested")
                        && release.body.contains("windows-arm64 built, not tested")
                        && release.body.contains("macos-arm64 pending"),
                    "{}",
                    release.body
                );
            }
            assert!(state.releases[0].prerelease && !state.releases[1].prerelease);
        });
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(publisher.uploads(), first_uploads);
        let tag = publisher.runtime_tag();
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
            assert_eq!(state.releases.len(), 2);
            let stored: usize = state
                .releases
                .iter()
                .map(|release| release.assets.len())
                .sum();
            assert_eq!(stored, 30);
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
        let runtime = resumed.runtime_tag();
        resumed.state(|state| {
            let draft = release_mut(state, &runtime);
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
            let draft = release_mut(state, &runtime);
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
    fn a_runtime_draft_of_an_interrupted_earlier_commit_follows_the_new_commit() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        let runtime = publisher.runtime_tag();
        let workspace = Publisher::workspace_tag();
        let stale = "b".repeat(40);
        publisher.state(|state| release_mut(state, &runtime).target = stale.clone());
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        publisher.state(|state| {
            assert_eq!(release_mut(state, &runtime).target, COMMIT);
            assert!(release_mut(state, &runtime).body.contains(COMMIT));
            assert_eq!(release_mut(state, &runtime).assets.len(), 13);
        });
        // The workspace draft is bound to its own commit and is never retargeted.
        publisher.state(|state| release_mut(state, &workspace).target = stale.clone());
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("targets a different source commit"),
            "{}",
            text(&result)
        );
        assert_eq!(publisher.uploads(), 30, "nothing is uploaded twice");
    }

    #[test]
    fn promotion_checks_the_drafts_again_and_publishes_them() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        // Nothing to promote before the drafts exist.
        assert!(!publisher.promote().status.success());
        assert!(publisher.publish().status.success());
        let result = publisher.promote();
        assert!(result.status.success(), "{}", text(&result));
        let output = text(&result);
        // No downstream credential exists on this runner: reported, not failed.
        assert!(
            output.contains("downstream github: skipped: no upload token"),
            "{output}"
        );
        assert!(
            output.contains("downstream gitlab: skipped: no upload token"),
            "{output}"
        );
        publisher.state(|state| {
            assert!(state.releases.iter().all(|release| !release.draft));
            assert_eq!(state.tags.len(), 2);
            assert!(state.tags.values().all(|commit| commit == COMMIT));
        });
        // A repeat verifies and changes nothing.
        assert!(publisher.promote().status.success());
        // A corrupted draft never becomes public.
        let other = Release::new();
        let second = Publisher::new(&other);
        assert!(second.publish().status.success());
        let tag = second.runtime_tag();
        second.state(|state| {
            let asset = release_mut(state, &tag)
                .assets
                .iter_mut()
                .find(|(_, name, _)| name == "ccvl-linux-x86_64")
                .unwrap();
            asset.2 = b"swapped binary".to_vec();
        });
        assert!(!second.promote().status.success());
        second.state(|state| assert!(state.releases.iter().all(|release| release.draft)));
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
        let tag = publisher.runtime_tag();
        publisher.state(|state| {
            state.tags.insert(tag, "b".repeat(40));
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
    fn published_gap_is_not_repaired_and_runtime_may_reuse_earlier_source() {
        let release = Release::new();
        let publisher = Publisher::new(&release);
        assert!(publisher.publish().status.success());
        assert!(publisher.promote().status.success());
        // A matching immutable runtime of an earlier source is reused...
        let runtime = publisher.runtime_tag();
        publisher.state(|state| {
            release_mut(state, &runtime).target = "b".repeat(40);
            state.tags.insert(runtime.clone(), "b".repeat(40));
        });
        let result = publisher.publish();
        assert!(result.status.success(), "{}", text(&result));
        // ...a workspace release of another source is not.
        let workspace = Publisher::workspace_tag();
        publisher.state(|state| {
            release_mut(state, &workspace).target = "b".repeat(40);
            state.tags.insert(workspace.clone(), "b".repeat(40));
        });
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(
            text(&result).contains("targets a different source commit"),
            "{}",
            text(&result)
        );
        // A published release missing an asset is incomplete, and nothing repairs it.
        publisher.state(|state| {
            let release = release_mut(state, &workspace);
            release.target = COMMIT.to_owned();
            release.assets.pop();
            state.tags.insert(workspace.clone(), COMMIT.to_owned());
        });
        let uploads = publisher.uploads();
        let result = publisher.publish();
        assert!(!result.status.success());
        assert!(text(&result).contains("is incomplete"), "{}", text(&result));
        assert_eq!(publisher.uploads(), uploads);
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

    #[test]
    fn downstream_without_a_token_or_a_route_is_skipped_and_never_fails() {
        let release = Release::new();
        assert!(release.check().status.success());
        let script = release.root.join(".agent/scripts/downstream-release.sh");
        // Targets that nothing listens on: no route from this runner.
        fs::write(
        release.root.join(".agent/release-downstreams.json"),
        json!({
            "github": {"api": "http://127.0.0.1:1", "uploads": "http://127.0.0.1:1", "repository": "corbet-fsl/ccvl"},
            "gitlab": {"api": "http://127.0.0.1:1", "project": "corbet-fsl/ccvl"},
        })
        .to_string(),
    )
    .unwrap();
        let tokens = release.temp.path().join("tokens.json");
        fs::write(
            &tokens,
            json!({"github": "fixture-github-token", "gitlab": "fixture-gitlab-token"}).to_string(),
        )
        .unwrap();
        for (tokens_file, expected) in [
            ("", "skipped: no upload token"),
            (
                tokens.to_str().unwrap(),
                "is not reachable from this runner",
            ),
        ] {
            let result = Command::new("bash")
                .arg(&script)
                .arg(&release.output)
                .env_clear()
                .envs(
                    std::env::vars()
                        .filter(|(key, _)| matches!(key.as_str(), "PATH" | "HOME" | "TMPDIR")),
                )
                .env("CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE", tokens_file)
                .current_dir(&release.root)
                .output()
                .unwrap();
            assert!(result.status.success(), "{}", text(&result));
            let combined = text(&result);
            assert_eq!(combined.matches(expected).count(), 2, "{combined}");
            assert!(!combined.contains("fixture-github-token"), "{combined}");
        }
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
