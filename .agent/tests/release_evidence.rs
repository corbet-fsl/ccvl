//! Guard fixtures for the release route: release identity, complete native
//! evidence and immutable uploads. They drive the real `ccvl release-evidence`
//! command and the real release scripts against a synthetic source tree.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ccvl::archive::{sha256_hex, write_with_comment};
use ccvl::release_evidence::{GATES, PLATFORMS};
use serde_json::{Value, json};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_ccvl");
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn host_of(platform: &str) -> &'static str {
    PLATFORMS
        .iter()
        .find(|(name, _)| *name == platform)
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
        for name in ["release-evidence.sh", "publish-release.sh"] {
            files.push((
                format!(".agent/scripts/{name}"),
                fs::read(repository().join(".agent/scripts").join(name)).unwrap(),
            ));
        }
        for (name, payload) in &files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, payload).unwrap();
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
            ("CI_REPO".to_owned(), "corbet-fsl/ccvl".to_owned()),
            ("CI_COMMIT_SHA".to_owned(), COMMIT.to_owned()),
            ("CI_PIPELINE_NUMBER".to_owned(), "123".to_owned()),
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
            let host = host_of(&platform);
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
                ("kind", json!("native")),
                ("platform", json!(platform)),
                ("rust_host", json!(host)),
                ("rustc", json!("rustc 1.97.0 fixture")),
                ("cargo", json!("cargo 1.97.0 fixture")),
                ("checks", json!(["public-check", "native-runtime"])),
                ("assets", Value::Object(assets)),
                ("provider", json!("crow")),
                ("run", json!("123")),
            ] {
                receipt[key] = value;
            }
            self.write(&format!("ccvl-{platform}.receipt.json"), &receipt);
        }
        for gate in GATES {
            let mut receipt = self.identity.clone();
            for (key, value) in [
                ("kind", json!("gate")),
                ("gate", json!(gate)),
                ("provider", json!("crow")),
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
    assert_eq!(manifest["platforms"], json!(["linux-x86_64"]));
    assert_eq!(manifest["receipts"].as_object().unwrap().len(), 5);
    let runtime = read_json(&release.output.join("runtime-manifest.json"));
    assert_eq!(
        runtime["platforms"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["linux-x86_64"]
    );
    // The evidence files are byte stable: a repeat verifies and rewrites nothing.
    assert!(release.check().status.success());
}

#[test]
fn one_platform_absent_never_passes() {
    let release = Release::new();
    fs::remove_file(release.output.join("ccvl-linux-x86_64.receipt.json")).unwrap();
    let result = release.check();
    assert!(!result.status.success(), "{}", text(&result));
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
    release.write("ccvl-windows-arm64.receipt.json", &json!({}));
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
        &release.run(&["source"], &[("CI_REPO", "corbet-labs/applications")]),
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
    // Each gate records the exact native package; the whole set then checks.
    let destination = dist.to_str().unwrap();
    for gate in GATES {
        let result = release.run(&["gate", gate, "--output", destination], &[]);
        assert!(result.status.success(), "{gate}: {}", text(&result));
    }
    let result = release.run(&["check", "--output", destination], &[]);
    assert!(result.status.success(), "{}", text(&result));
    let receipt = read_json(&dist.join("ccvl-linux-x86_64.receipt.json"));
    assert_eq!(receipt["provider"], "crow");
    assert_eq!(receipt["run"], "123");
    assert_eq!(receipt["checks"], json!(["public-check", "native-runtime"]));
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
        &[("CI_PIPELINE_NUMBER", "")],
    );
    assert_refused(&result, "A CI run identity is required.");
}

const FAKE_GH: &str = r#"#!/usr/bin/env bash
set -u
root=$FAKE_GH_ROOT
mkdir -p "$root"
printf '%s\n' "$@" | jq -R . | jq -sc . >> "$root/calls"
http() { printf 'HTTP/2.0 %s\n\n%s\n' "$1" "$2"; }
if [[ $1 == api ]]; then
  if [[ $2 == graphql ]]; then
    if [[ -e $root/lookup-error ]]; then cat "$root/lookup-error"; echo; exit 0; fi
    tag=
    for a in "$@"; do [[ $a == tag=* ]] && tag=${a#tag=}; done
    if [[ -e $root/$tag/state ]]; then
      id=$(jq .id "$root/$tag/state")
      found=$(jq -cn --argjson id "$id" --arg tag "$tag" '{databaseId:$id,tagName:$tag}')
    else
      found=null
    fi
    printf '{"data":{"repository":{"release":%s}}}\n' "$found"
    exit 0
  fi
  target=
  for a in "${@:2}"; do [[ $a == repos/* ]] && { target=$a; break; }; done
  case $target in
    */git/ref/heads/main) echo "$CI_COMMIT_SHA"; exit 0 ;;
    */git/tags/*)
      http '200 OK' "$(jq -cn --arg sha "$CI_COMMIT_SHA" '{object:{type:"commit",sha:$sha}}')"
      exit 0 ;;
    */git/ref/tags/*)
      tag=${target##*/tags/}
      release=$root/$tag
      if [[ -e $release/state && $(jq .draft "$release/state") == false ]]; then
        commit=$CI_COMMIT_SHA
        [[ -e $release/tag_override ]] && commit=$(cat "$release/tag_override")
        kind=commit
        [[ -e $release/annotated ]] && kind=tag
        http '200 OK' "$(jq -cn --arg kind "$kind" --arg sha "$commit" '{object:{type:$kind,sha:$sha}}')"
        exit 0
      fi
      http '404 Not Found' '{}'
      exit 1 ;;
    */releases/tags/*)
      tag=${target##*/tags/}
      release=$root/$tag
      if [[ -e $release/state && $(jq .draft "$release/state") == true ]]; then
        http '404 Not Found' '{}'
        exit 1
      fi ;;
    *)
      id=${target##*/}
      release=
      for d in "$root"/*/; do
        if [[ -e $d/state && $(jq .id "$d/state") == "$id" ]]; then release=${d%/}; tag=$(basename "$release"); break; fi
      done
      [[ -n $release ]] || exit 1 ;;
  esac
  if [[ ! -e $release/state ]]; then http '404 Not Found' '{}'; exit 1; fi
  assets=$(ls "$release/assets" | jq -R . | jq -sc 'map({name:.})')
  http '200 OK' "$(jq -c --arg tag "$tag" --argjson assets "$assets" '. + {tag_name:$tag, assets:$assets}' "$release/state")"
  exit 0
fi
[[ $1 == release ]] || { echo "Unexpected gh call $*" >&2; exit 1; }
action=$2
tag=$3
release=$root/$tag
case $action in
  create)
    mkdir "$release" "$release/assets" || exit 1
    id=2
    [[ $tag == runtime-* ]] && id=1
    jq -cn --argjson id "$id" --arg commit "$CI_COMMIT_SHA" '{id:$id,draft:true,target_commitish:$commit}' > "$release/state" ;;
  upload)
    for a in "$@"; do [[ $a == --clobber ]] && exit 1; done
    asset=${*: -1}
    dest=$release/assets/$(basename "$asset")
    [[ ! -e $dest ]] || { echo 'overwriting upload' >&2; exit 1; }
    cp "$asset" "$dest" ;;
  download)
    while (($#)); do
      case $1 in
        --pattern) name=$2; shift ;;
        --output) dest=$2; shift ;;
      esac
      shift
    done
    [[ ! -e $dest ]] || exit 1
    cp "$release/assets/$name" "$dest" ;;
  edit)
    jq -c '.draft = false' "$release/state" > "$release/state.next"
    mv "$release/state.next" "$release/state" ;;
  *) echo "Unexpected gh call $*" >&2; exit 1 ;;
esac
"#;

struct Publisher<'a> {
    release: &'a Release,
    github: PathBuf,
    bin: PathBuf,
}

impl<'a> Publisher<'a> {
    fn new(release: &'a Release) -> Self {
        assert!(
            Command::new("jq").arg("--version").output().is_ok(),
            "jq is required for actual publisher guard fixtures"
        );
        let bin = release.temp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        fs::write(&gh, FAKE_GH).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            release,
            github: release.temp.path().join("github"),
            bin,
        }
    }

    fn publish(&self) -> Output {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        for _ in 0..20 {
            let output = Command::new("bash")
                .arg(self.release.root.join(".agent/scripts/publish-release.sh"))
                .arg(&self.release.output)
                .env_clear()
                .envs(std::env::vars().filter(|(key, _)| matches!(key.as_str(), "HOME" | "TMPDIR")))
                .envs(self.release.env.iter().map(|(key, value)| (key, value)))
                .env("PATH", &path)
                .env("GH_TOKEN", "fixture-token")
                .env("FAKE_GH_ROOT", &self.github)
                .env("CCVL_RELEASE_EVIDENCE_BIN", BIN)
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

    fn calls(&self) -> String {
        fs::read_to_string(self.github.join("calls")).unwrap_or_default()
    }

    fn call_list(&self) -> Vec<Vec<String>> {
        self.calls()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn uploads(&self) -> usize {
        self.calls().matches("\"upload\"").count()
    }

    fn runtime_tag(&self) -> PathBuf {
        self.github.join(format!(
            "runtime-{}",
            self.release.identity_field("runtime_id")
        ))
    }
}

fn rewrite_state(release: &Path, change: impl FnOnce(&mut Value)) {
    let path = release.join("state");
    let mut state = read_json(&path);
    change(&mut state);
    fs::write(path, state.to_string()).unwrap();
}

#[test]
fn publication_is_complete_and_repeat_is_immutable() {
    let release = Release::new();
    let publisher = Publisher::new(&release);
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    let first_uploads = publisher.uploads();
    assert!(first_uploads > 0);
    let creations: Vec<_> = publisher
        .call_list()
        .into_iter()
        .filter(|call| call[..2] == ["release", "create"])
        .collect();
    assert_eq!(creations.len(), 2);
    for call in creations {
        let notes = &call[call
            .iter()
            .position(|argument| argument == "--notes")
            .unwrap()
            + 1];
        assert!(
            notes.contains("Released platforms: linux-x86_64."),
            "{notes}"
        );
        assert!(!notes.contains("All six"));
    }
    assert_eq!(first_uploads, 12);
    assert!(publisher.calls().contains("\"graphql\""));
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    assert_eq!(publisher.uploads(), first_uploads);
    fs::write(
        publisher.runtime_tag().join("assets/ccvl-linux-x86_64"),
        b"different published bytes",
    )
    .unwrap();
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
fn unknown_draft_lookup_never_creates_or_uploads() {
    let release = Release::new();
    let publisher = Publisher::new(&release);
    fs::create_dir_all(&publisher.github).unwrap();
    let runtime_tag = format!("runtime-{}", release.identity_field("runtime_id"));
    let cases = [
        json!({"errors": [{"message": "fixture lookup unavailable"}]}),
        json!({"data": {"repository": null}}),
        json!({"data": {"repository": {}}}),
        json!({"data": {"repository": {"release": {"databaseId": 1, "tagName": "another-release"}}}}),
        json!({"data": {"repository": {"release": {"databaseId": null, "tagName": runtime_tag}}}}),
    ];
    for response in cases {
        fs::write(publisher.github.join("lookup-error"), response.to_string()).unwrap();
        assert!(!publisher.publish().status.success(), "{response}");
    }
    let calls = publisher.calls();
    assert!(!calls.contains("\"create\""));
    assert!(!calls.contains("\"upload\""));
}

#[test]
fn existing_wrong_tag_prevents_further_publication() {
    let release = Release::new();
    let publisher = Publisher::new(&release);
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    let uploads = publisher.uploads();
    fs::write(publisher.runtime_tag().join("tag_override"), "b".repeat(40)).unwrap();
    let result = publisher.publish();
    assert!(!result.status.success());
    assert!(
        text(&result).contains("targets a different source commit"),
        "{}",
        text(&result)
    );
    assert_eq!(publisher.uploads(), uploads);
}

#[test]
fn annotated_tag_is_peeled_and_published_gap_is_not_repaired() {
    let release = Release::new();
    let publisher = Publisher::new(&release);
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    fs::write(publisher.runtime_tag().join("annotated"), "").unwrap();
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    let uploads = publisher.uploads();
    fs::remove_file(publisher.runtime_tag().join("assets/ccvl-linux-x86_64")).unwrap();
    let result = publisher.publish();
    assert!(!result.status.success());
    assert!(text(&result).contains("is incomplete"), "{}", text(&result));
    assert_eq!(publisher.uploads(), uploads);
}

#[test]
fn runtime_may_reuse_earlier_source_but_workspace_tag_may_not() {
    let release = Release::new();
    let publisher = Publisher::new(&release);
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    let runtime = publisher.runtime_tag();
    rewrite_state(&runtime, |state| {
        state["target_commitish"] = json!("b".repeat(40));
    });
    fs::write(runtime.join("tag_override"), "b".repeat(40)).unwrap();
    let result = publisher.publish();
    assert!(result.status.success(), "{}", text(&result));
    let workspace = publisher.github.join(format!("build-{COMMIT}"));
    rewrite_state(&workspace, |state| {
        state["target_commitish"] = json!("b".repeat(40));
    });
    fs::write(workspace.join("tag_override"), "b".repeat(40)).unwrap();
    let result = publisher.publish();
    assert!(!result.status.success());
    assert!(
        text(&result).contains("targets a different source commit"),
        "{}",
        text(&result)
    );
}

#[test]
fn missing_native_evidence_prevents_any_publication_call() {
    let release = Release::new();
    let publisher = Publisher::new(&release);
    fs::remove_file(release.output.join("ccvl-linux-x86_64.receipt.json")).unwrap();
    let result = publisher.publish();
    assert!(!result.status.success());
    assert!(!publisher.github.join("calls").exists());
}

struct ArchiveExecutor {
    temp: TempDir,
    dist: PathBuf,
    marker: PathBuf,
    executor: PathBuf,
    command: String,
    env: Vec<(String, String)>,
}

fn dedent(text: &str) -> String {
    let margin = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|line| line.get(margin..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

impl ArchiveExecutor {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("ccvl-archive-adapter-test-")
            .tempdir()
            .unwrap();
        let root = temp.path().to_path_buf();
        let dist = root.join("package directory");
        fs::create_dir_all(&dist).unwrap();
        let marker = root.join("gate-called");
        let gate = b"#!/usr/bin/env bash\nset -eu\ntest \"$1\" = \"$EXPECTED_DIST\"\nprintf gate > \"$GATE_MARKER\"\n";
        let archive = root.join("source.tar");
        fs::write(
            &archive,
            ccvl::archive::write([(
                ".agent/scripts/check-release-archive.sh",
                0o644,
                gate.as_slice(),
            )]),
        )
        .unwrap();
        let workflow = fs::read_to_string(repository().join(".crow/release-archive.yaml")).unwrap();
        let command = dedent(workflow.split_once("      - |\n").unwrap().1);
        let executor = root.join("external executor");
        fs::write(
            &executor,
            "#!/usr/bin/env bash\nset -eu\ntest \"$0\" != \"$ORIGINAL_EXECUTOR\"\nprintf executor > \"$EXECUTOR_MARKER\"\nexec bash \"$1\" \"$2\"\n",
        )
        .unwrap();
        let env = [
            ("CI_REPO", "corbet-fsl/ccvl".to_owned()),
            ("SOURCE_ARCHIVE", archive.display().to_string()),
            ("SOURCE_SHA256", file_hash(&archive)),
            ("CCVL_RELEASE_DIR", dist.display().to_string()),
            ("EXPECTED_DIST", dist.display().to_string()),
            ("GATE_MARKER", marker.display().to_string()),
            (
                "EXECUTOR_MARKER",
                root.join("executor-called").display().to_string(),
            ),
            ("ORIGINAL_EXECUTOR", executor.display().to_string()),
            ("ARCHIVE_EXECUTOR", String::new()),
            ("ARCHIVE_EXECUTOR_SHA256", String::new()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        Self {
            temp,
            dist,
            marker,
            executor,
            command,
            env,
        }
    }

    fn invoke(&self, changes: &[(&str, &str)]) -> Output {
        for _ in 0..20 {
            let output = Command::new("bash")
                .args(["-c", &self.command])
                .envs(self.env.iter().map(|(key, value)| (key, value)))
                .envs(changes.iter().copied())
                .output();
            match output {
                Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                other => return other.unwrap(),
            }
        }
        panic!("archive adapter stayed busy");
    }

    fn executor_called(&self) -> bool {
        self.temp.path().join("executor-called").exists()
    }
}

#[test]
fn default_route_runs_verified_check_directly() {
    let fixture = ArchiveExecutor::new();
    let result = fixture.invoke(&[]);
    assert!(result.status.success(), "{}", text(&result));
    assert_eq!(fs::read_to_string(&fixture.marker).unwrap(), "gate");
    assert!(!fixture.executor_called());
    let _ = &fixture.dist;
}

#[test]
fn verified_executor_snapshot_receives_unchanged_check_and_package() {
    let fixture = ArchiveExecutor::new();
    let executor = fixture.executor.display().to_string();
    let result = fixture.invoke(&[
        ("ARCHIVE_EXECUTOR", &executor),
        ("ARCHIVE_EXECUTOR_SHA256", &file_hash(&fixture.executor)),
    ]);
    assert!(result.status.success(), "{}", text(&result));
    assert_eq!(fs::read_to_string(&fixture.marker).unwrap(), "gate");
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("executor-called")).unwrap(),
        "executor"
    );
}

#[test]
fn incomplete_changed_or_untrusted_executor_never_executes() {
    let fixture = ArchiveExecutor::new();
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
        vec![
            ("ARCHIVE_EXECUTOR", &executor),
            ("ARCHIVE_EXECUTOR_SHA256", &checksum),
            ("SOURCE_SHA256", &zeros),
        ],
    ];
    for inputs in cases {
        let result = fixture.invoke(&inputs);
        assert!(!result.status.success(), "{inputs:?}: {}", text(&result));
        assert!(!fixture.marker.exists());
        assert!(!fixture.executor_called());
    }
}
