//! Source-bound native packages and release receipts shared by CI providers.
//!
//! Maintainer route, hidden from the normal help: `ccvl release-evidence
//! <source|native|cross|attest|gate|check|platforms|version>`. The release scripts call it; see
//! `.agent/docs/releases.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use serde_json::{Map, Value, json};

use crate::archive::{
    self, Env, Kind, absolute, command_text, dumps, file_sha256, lfs_pointer, sha256_hex,
    unsafe_member_path,
};

const REPOSITORY: &str = "corbet-fsl/ccvl";

/// What the release holds for one platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Built and run on its own platform: only a `native` receipt satisfies it.
    Native,
    /// Built and run on its own platform once that executor exists. Until then a
    /// `cross` receipt (built, tests pending) is accepted.
    NativeWhenAvailable,
    /// Built for a platform nothing here can run (there is no native ARM64
    /// executor and no emulation): only a `cross` receipt, never a test claim.
    BuiltOnly,
}

/// Release platform names, the Rust target triple each is built for, and the
/// evidence each holds. Both Windows platforms are built for the one `gnullvm`
/// ABI (clang and lld with the mingw-w64 UCRT runtime, from `llvm-mingw`): free
/// licences, no Microsoft SDK on the runner, and the `x86_64` executable that is
/// tested in the Windows clone is the very file that ships.
pub const PLATFORMS: [(&str, &str, Plan); 6] = [
    ("linux-x86_64", "x86_64-unknown-linux-gnu", Plan::Native),
    (
        "linux-aarch64",
        "aarch64-unknown-linux-gnu",
        Plan::BuiltOnly,
    ),
    (
        "macos-x86_64",
        "x86_64-apple-darwin",
        Plan::NativeWhenAvailable,
    ),
    ("macos-arm64", "aarch64-apple-darwin", Plan::BuiltOnly),
    (
        "windows-x86_64",
        "x86_64-pc-windows-gnullvm",
        Plan::NativeWhenAvailable,
    ),
    (
        "windows-arm64",
        "aarch64-pc-windows-gnullvm",
        Plan::BuiltOnly,
    ),
];
/// Platforms that only their own executor can build: the `release-macos` leg builds them
/// in the macOS build clone, after the release run has left its draft. A fresh draft has no
/// receipt for them yet and calls them `pending`; `check --complete` (a promotion) refuses
/// that state.
pub const PENDING_ALLOWED: [&str; 2] = ["macos-x86_64", "macos-arm64"];
pub const GATES: [&str; 4] = ["rust", "lint", "linux-deep", "archive"];
/// What the native executor of a platform that this host can not run reports about the
/// executable that ships (`attest`): one entry per id, every one of them `ok`.
pub const NATIVE_RESULT_CHECKS: [&str; 6] = [
    "sha256",
    "architecture",
    "version",
    "runtime-id",
    "public-check",
    "test-suite",
];

/// What a release holds, derived from its version alone (no input, no knob).
///
/// Julian, 2026-11-13: "use semantic versioning. we are not at 1.0 - we should not
/// release for every patch or minor version. And for Mac and Windows / arm even
/// more sparingly. We must keep down the compute."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseKind {
    /// `0.y.0`: every declared platform (Linux, macOS and Windows `x86_64` built and
    /// tested where an executor exists, the three ARM64 platforms built only).
    Minor,
    /// `0.y.z` with `z > 0`: Linux `x86_64` alone.
    Patch,
}

impl ReleaseKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }
}

/// The version a release is published under, as `v<version>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseVersion {
    pub version: String,
    pub kind: ReleaseKind,
}

/// A releasable version is exactly `0.MINOR.PATCH` in plain decimal. A pre-release
/// or build suffix is valid semver but names no release; `1.0` and later wait for
/// a decision on the policy itself.
pub fn parse_release_version(version: &str) -> Result<ReleaseVersion> {
    let number = |part: &str| {
        !part.is_empty()
            && part.len() <= 9
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|part| number(part)) {
        return fail(format!(
            "Version {version:?} is not a releasable semantic version: MAJOR.MINOR.PATCH in plain digits, with no pre-release or build suffix."
        ));
    }
    if parts[0] != "0" {
        return fail(format!(
            "Version {version} is 1.0 or later; the release policy covers 0.y.z only and needs a decision before a 1.0 release."
        ));
    }
    let kind = if parts[2] == "0" {
        ReleaseKind::Minor
    } else {
        ReleaseKind::Patch
    };
    Ok(ReleaseVersion {
        version: version.to_owned(),
        kind,
    })
}

/// The version of the workspace package (`Cargo.toml`), checked against the policy.
pub fn release_version(root: &Path) -> Result<ReleaseVersion> {
    let text = fs::read_to_string(root.join("Cargo.toml")).context("cannot read Cargo.toml")?;
    let manifest: toml::Table = text.parse().context("Cargo.toml is not valid TOML")?;
    let version = manifest
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .context("Cargo.toml has no package version")?;
    parse_release_version(version)
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Verify the checkout against the exact CI source and print its identity.
    Source {
        /// Write the identity as JSON instead of printing it.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Package one tested native executable with its receipt.
    Native {
        #[arg(long, value_parser = platform_names())]
        platform: String,
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        rustc: String,
        #[arg(long)]
        cargo: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Package one cross-built executable (built, never run here) with its receipt.
    Cross {
        #[arg(long, value_parser = platform_names())]
        platform: String,
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        rustc: String,
        #[arg(long)]
        cargo: String,
        /// Identity of the cross toolchain (for example `cargo-zigbuild 0.20 zig 0.16`
        /// or `llvm-mingw 20261104 clang 23.1.5`).
        #[arg(long)]
        tool: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Turn the cross receipt of a platform into a native one, after its executable ran
    /// and passed in the native executor of that platform (the Windows clone).
    Attest {
        #[arg(long, value_parser = platform_names())]
        platform: String,
        /// The executor's report (`ccvl-<platform>.native-result.json` is made of it).
        #[arg(long)]
        result: PathBuf,
        /// The directory holding the cross-built package of the platform and its receipt.
        #[arg(long)]
        output: PathBuf,
    },
    /// Record one release gate receipt.
    Gate {
        #[arg(value_parser = GATES)]
        gate: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify the release evidence and write the manifests.
    ///
    /// A fresh draft may still await a platform leg: macOS `pending` (no receipt yet) and
    /// Windows `x86_64` only `built` (its clone has not tested it yet). `--complete` is
    /// the state a release is promoted in.
    Check {
        #[arg(long)]
        output: PathBuf,
        /// Require the final evidence of every platform the version holds: no `pending`
        /// platform, and the platforms with a native executor `tested` on it.
        #[arg(long)]
        complete: bool,
    },
    /// Print the explicit release targets of this version.
    Platforms,
    /// Print the release version and its kind (`minor` or `patch`), or refuse a
    /// version that names no release.
    Version,
}

fn platform_names() -> Vec<&'static str> {
    PLATFORMS.iter().map(|(name, _, _)| *name).collect()
}

fn host_of(platform: &str) -> Option<&'static str> {
    PLATFORMS
        .iter()
        .find(|(name, _, _)| *name == platform)
        .map(|(_, host, _)| *host)
}

fn plan_of(platform: &str) -> Option<Plan> {
    PLATFORMS
        .iter()
        .find(|(name, _, _)| *name == platform)
        .map(|(_, _, plan)| *plan)
}

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(anyhow!(message.into()))
}

/// Run one subcommand from the workspace `root` (the current directory unless
/// given). Like the helper it replaced, failures print the bare message.
// The reason keeps both clippys quiet: one wants a `must_use` here (`must_use_candidate`), the
// newer one rejects a bare one on a type that is `must_use` already (`double_must_use`).
#[must_use = "the exit code is the verdict of the command"]
pub fn run(root: Option<&Path>, command: &Command) -> ExitCode {
    let outcome = root
        .map_or_else(
            || std::env::current_dir().context("cannot resolve the current directory"),
            |root| Ok(root.to_path_buf()),
        )
        .and_then(|root| execute(&root, &Env::from_process(), command));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

pub fn execute(root: &Path, env: &Env, command: &Command) -> Result<()> {
    match command {
        Command::Platforms => {
            println!("{}", release_platforms(root)?.join("\n"));
        }
        Command::Version => {
            let release = release_version(root)?;
            println!("{} {}", release.version, release.kind.name());
        }
        Command::Source { output } => {
            let (identity, _) = source(root, env)?;
            match output {
                Some(path) => write_json(path, &identity.to_value())?,
                None => println!("{}", dumps(&identity.to_value(), None)),
            }
        }
        Command::Native {
            platform,
            binary,
            host,
            rustc,
            cargo,
            output,
        } => native(
            root,
            env,
            &NativeArguments {
                platform,
                binary,
                host,
                rustc,
                cargo,
                output,
                tool: None,
            },
        )?,
        Command::Cross {
            platform,
            binary,
            host,
            rustc,
            cargo,
            tool,
            output,
        } => native(
            root,
            env,
            &NativeArguments {
                platform,
                binary,
                host,
                rustc,
                cargo,
                output,
                tool: Some(tool),
            },
        )?,
        Command::Attest {
            platform,
            result,
            output,
        } => attest(root, env, platform, result, output)?,
        Command::Gate { gate: name, output } => gate(root, env, name, output)?,
        Command::Check { output, complete } => check(root, env, output, *complete)?,
    }
    Ok(())
}

/// Explicit source policy; available artifacts never choose release scope.
///
/// `.agent/release-platforms.txt` declares every platform the project supports. A
/// release holds all of them when its version is `0.y.0` and Linux `x86_64` alone
/// when it is `0.y.z` with `z > 0`: the version decides, nothing else.
pub fn release_platforms(root: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(root.join(".agent/release-platforms.txt"))
        .context("cannot read .agent/release-platforms.txt")?;
    let platforms: Vec<String> = text.lines().map(str::to_owned).collect();
    let unique: BTreeSet<&String> = platforms.iter().collect();
    if platforms.is_empty()
        || unique.len() != platforms.len()
        || platforms.iter().any(|platform| host_of(platform).is_none())
        || !platforms.iter().any(|platform| platform == "linux-x86_64")
    {
        return fail(
            "Invalid release platform policy; unique known targets including Linux x86_64 are required.",
        );
    }
    Ok(match release_version(root)?.kind {
        ReleaseKind::Minor => platforms,
        ReleaseKind::Patch => vec!["linux-x86_64".to_owned()],
    })
}

#[derive(Clone, Debug)]
pub struct Identity {
    pub repository: String,
    pub source_commit: String,
    pub source_sha256: String,
    pub source_archive_sha256: String,
    pub cargo_lock_sha256: String,
    pub runtime_id: String,
}

impl Identity {
    fn map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "schema": 1,
            "repository": self.repository,
            "source_commit": self.source_commit,
            "source_sha256": self.source_sha256,
            "source_archive_sha256": self.source_archive_sha256,
            "cargo_lock_sha256": self.cargo_lock_sha256,
            "runtime_id": self.runtime_id,
        }) else {
            unreachable!("object literal")
        };
        map
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::Object(self.map())
    }

    fn get(&self, field: &str) -> Value {
        self.map().get(field).cloned().unwrap_or(Value::Null)
    }
}

struct SourceFile {
    payload: Vec<u8>,
    mode: u32,
}

type SourceFiles = BTreeMap<String, SourceFile>;

fn engine_package(path: &str) -> bool {
    path.strip_prefix(".agent/typst/").is_some_and(|name| {
        !name.contains('/')
            && [".typ", ".toml", ".json"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
    })
}

fn is_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    command_text(root, Path::new("git"), args)
}

/// Verify that the checkout is the exact CI source and derive its identity.
fn source(root: &Path, env: &Env) -> Result<(Identity, SourceFiles)> {
    let repository = env.nonempty("GITHUB_REPOSITORY").unwrap_or_default();
    if repository != REPOSITORY {
        return fail("Release work requires the public corbet-fsl/ccvl repository.");
    }
    let commit = env
        .first_nonempty(&["CI_COMMIT_SHA", "GITHUB_SHA"])
        .unwrap_or_default();
    if !is_commit(commit) {
        return fail("An exact CI source commit is required.");
    }
    let data = if let Some(archive_path) = env.nonempty("SOURCE_ARCHIVE") {
        let data = fs::read(archive_path).with_context(|| format!("cannot read {archive_path}"))?;
        if env.get("SOURCE_SHA256") != Some(sha256_hex(&data).as_str()) {
            return fail("Source archive checksum mismatch.");
        }
        data
    } else {
        if git(root, &["rev-parse", "HEAD"])? != commit {
            return fail("Checkout differs from the requested source commit.");
        }
        // Generated ignored caches are allowed; changed tracked/public inputs are not.
        if !git(root, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty() {
            return fail("Release source checkout must be clean.");
        }
        let output = std::process::Command::new("git")
            .args(["archive", commit])
            .current_dir(root)
            .output()
            .context("cannot run git archive")?;
        if !output.status.success() {
            bail!("git archive failed with {}.", output.status);
        }
        output.stdout
    };
    let archive = archive::read(&data)?;
    if archive.comment.as_deref() != Some(commit) {
        return fail("Source archive is not the requested Git commit.");
    }
    let mut files = SourceFiles::new();
    for member in archive.entries {
        if unsafe_member_path(&member.name) {
            return fail("Unsafe source archive path.");
        }
        if member.kind == Kind::Directory {
            continue;
        }
        if member.kind != Kind::File || files.contains_key(&member.name) {
            return fail("Only unique regular source files are supported.");
        }
        let path = root.join(&member.name);
        let metadata = fs::symlink_metadata(&path).ok();
        if !metadata.as_ref().is_some_and(fs::Metadata::is_file) {
            return fail(format!("Missing source input: {}", member.name));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = metadata
                .as_ref()
                .map_or(0, |metadata| metadata.permissions().mode());
            // Git records one bit: executable or not. The group and other bits of a checkout
            // are whatever the umask of its owner made them (the macOS clone's user has 077).
            if (mode & 0o111 != 0) != (member.mode & 0o111 != 0) {
                return fail(format!("Source executable mode changed: {}", member.name));
            }
        }
        let actual = file_sha256(&path)?;
        let size = metadata.as_ref().map(fs::Metadata::len);
        // An archive made without the LFS filter holds the committed pointer; the
        // checkout then holds the exact object that pointer names.
        let payload = if actual == sha256_hex(&member.data) {
            member.data
        } else if lfs_pointer(&member.data)
            .is_some_and(|(oid, bytes)| oid == actual && size == Some(bytes))
        {
            fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?
        } else {
            return fail(format!("Source input changed: {}", member.name));
        };
        files.insert(
            member.name,
            SourceFile {
                payload,
                mode: member.mode,
            },
        );
    }
    let required = [
        "Cargo.toml",
        ".agent/core/Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".agent/build.rs",
        "ccvl.json",
    ];
    if !required
        .iter()
        .chain(&[".agent/release-platforms.txt"])
        .all(|path| files.contains_key(*path))
    {
        return fail("Incomplete source archive.");
    }
    let mut fingerprint_paths: Vec<String> =
        required.iter().map(|path| (*path).to_owned()).collect();
    fingerprint_paths.extend(
        files
            .keys()
            .filter(|path| {
                ((path.starts_with(".agent/src/") || path.starts_with(".agent/core/src/"))
                    && Path::new(path.as_str())
                        .extension()
                        .is_some_and(|extension| extension == "rs"))
                    || engine_package(path)
            })
            .cloned(),
    );
    // Required inputs come first in their fixed order; sources follow sorted.
    fingerprint_paths[required.len()..].sort();
    let runtime_id = sha256_hex(
        fingerprint_paths
            .iter()
            .map(|path| format!("{path} {}\n", sha256_hex(&files[path].payload)))
            .collect::<Vec<String>>()
            .concat()
            .as_bytes(),
    );
    // Canonical file content/mode identity permits differently encoded provider archives.
    let source_sha256 = sha256_hex(
        files
            .iter()
            .map(|(path, file)| format!("{path} {:o} {}\n", file.mode, sha256_hex(&file.payload)))
            .collect::<Vec<String>>()
            .concat()
            .as_bytes(),
    );
    Ok((
        Identity {
            repository: repository.to_owned(),
            source_commit: commit.to_owned(),
            source_sha256,
            source_archive_sha256: sha256_hex(&data),
            cargo_lock_sha256: sha256_hex(&files["Cargo.lock"].payload),
            runtime_id,
        },
        files,
    ))
}

fn identity_equal(actual: &Value, expected: &Identity) -> Result<()> {
    for field in [
        "schema",
        "repository",
        "source_commit",
        "source_sha256",
        "cargo_lock_sha256",
        "runtime_id",
    ] {
        if actual.get(field) != Some(&expected.get(field)) {
            return fail(format!("Release receipt differs in {field}"));
        }
    }
    let archive = actual
        .get("source_archive_sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_digest(archive) {
        return fail("Missing source archive identity.");
    }
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = format!("{}\n", dumps(value, Some(2)));
    if path.exists() && fs::read(path)? != encoded.as_bytes() {
        return fail(format!(
            "Refusing to replace differing evidence: {}",
            path.display()
        ));
    }
    fs::write(path, encoded).with_context(|| format!("cannot write {}", path.display()))
}

fn load(path: &Path) -> Result<Value> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid JSON in {}", path.display()))
}

/// The Forgejo Actions run that produced the evidence: it is the only CI engine.
fn provenance(env: &Env) -> Result<Map<String, Value>> {
    let Some(run) = env
        .nonempty("GITHUB_RUN_ID")
        .filter(|run| run.bytes().all(|byte| byte.is_ascii_digit()))
        .filter(|_| env.get("FORGEJO_ACTIONS") == Some("true"))
    else {
        return fail("A Forgejo Actions run identity is required.");
    };
    let mut map = Map::new();
    map.insert("provider".into(), "forgejo".into());
    map.insert("run".into(), run.into());
    Ok(map)
}

fn windows(platform: &str) -> bool {
    platform.starts_with("windows-")
}

struct NativeArguments<'a> {
    platform: &'a str,
    binary: &'a Path,
    host: &'a str,
    rustc: &'a str,
    cargo: &'a str,
    output: &'a Path,
    /// The cross toolchain when the executable was built for another platform
    /// and can not be run here; `None` for an executable that ran.
    tool: Option<&'a str>,
}

/// A cross-built executable is not run; it is verified to embed the identity of
/// the verified source (the runtime id is a string constant of the binary).
fn embeds_identity(binary: &Path, runtime_id: &str) -> Result<bool> {
    let bytes = fs::read(binary).with_context(|| format!("cannot read {}", binary.display()))?;
    let needle = runtime_id.as_bytes();
    Ok(bytes.windows(needle.len()).any(|window| window == needle))
}

fn native(root: &Path, env: &Env, arguments: &NativeArguments) -> Result<()> {
    let (identity, files) = source(root, env)?;
    if host_of(arguments.platform) != Some(arguments.host) {
        return fail("Rust compiler host does not match the requested native platform.");
    }
    match (plan_of(arguments.platform), arguments.tool) {
        (Some(Plan::BuiltOnly), None) => {
            return fail(
                "This platform is built only and never run; it can not carry native evidence.",
            );
        }
        (Some(Plan::Native), Some(_)) => {
            return fail("This platform requires native evidence; a cross build can not stand in.");
        }
        _ => {}
    }
    let binary = absolute(arguments.binary)?;
    let binary = binary.canonicalize().unwrap_or(binary);
    if arguments.tool.is_some() {
        if !embeds_identity(&binary, &identity.runtime_id)? {
            return fail("Cross-built runtime does not embed the verified source identity.");
        }
    } else if command_text(root, &binary, &["runtime-id"])? != identity.runtime_id {
        return fail("Compiled runtime differs from the verified source.");
    }
    fs::create_dir_all(arguments.output)?;
    let output = arguments.output.canonicalize()?;
    let platform = arguments.platform;
    let name = format!(
        "ccvl-{platform}{}",
        if windows(platform) { ".exe" } else { "" }
    );
    let exe = if windows(platform) {
        "ccvl.exe"
    } else {
        "ccvl"
    };
    let archive_name = format!("ccvl-{platform}.tar.gz");
    let binary_path = output.join(&name);
    if binary_path.exists() && file_sha256(&binary_path)? != file_sha256(&binary)? {
        return fail("Output already contains a different native executable.");
    }
    let payload = fs::read(&binary)?;
    fs::write(&binary_path, &payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o755))?;
    }
    let archive_path = output.join(&archive_name);
    if archive_path.exists() {
        return fail("Package already exists; retain it and use a new attempt directory.");
    }
    let runtime_entry = format!(".agent/cache/ccvl/bin/{exe}");
    archive::write_tar_gz(
        &archive_path,
        files
            .iter()
            .map(|(path, file)| (path.as_str(), file.mode, file.payload.as_slice()))
            .chain([(runtime_entry.as_str(), 0o755, payload.as_slice())]),
    )?;
    let assets = BTreeMap::from([
        (name.clone(), file_sha256(&binary_path)?),
        (archive_name.clone(), file_sha256(&archive_path)?),
    ]);
    for (filename, checksum) in &assets {
        fs::write(
            output.join(format!("{filename}.sha256")),
            format!("{checksum}  {filename}\n"),
        )?;
    }
    fs::write(
        output.join(format!("{name}.runtime-id")),
        format!("{}\n", identity.runtime_id),
    )?;
    let mut receipt = identity.map();
    receipt.extend(provenance(env)?);
    let (kind, checks) = if arguments.tool.is_some() {
        ("cross", json!(["cross-build"]))
    } else {
        ("native", json!(["public-check", "native-runtime"]))
    };
    for (key, value) in [
        ("kind", json!(kind)),
        ("platform", json!(platform)),
        ("rust_host", json!(arguments.host)),
        ("rustc", json!(arguments.rustc)),
        ("cargo", json!(arguments.cargo)),
        ("checks", checks),
        ("assets", json!(assets)),
    ] {
        receipt.insert(key.into(), value);
    }
    if let Some(tool) = arguments.tool {
        // Built, not tested: the executable was never run.
        receipt.insert("tested".into(), json!(false));
        receipt.insert("tool".into(), json!(tool));
    }
    write_json(
        &output.join(format!("ccvl-{platform}.receipt.json")),
        &Value::Object(receipt),
    )
}

/// The executable and the archive a receipt names are in `output`, with the checksum
/// files and the runtime identity of this source. Returns the executable's sha256.
fn verify_package(
    output: &Path,
    platform: &str,
    receipt: &Value,
    identity: &Identity,
) -> Result<String> {
    let name = format!(
        "ccvl-{platform}{}",
        if windows(platform) { ".exe" } else { "" }
    );
    let expected_names: BTreeSet<String> = [name.clone(), format!("ccvl-{platform}.tar.gz")].into();
    let assets = receipt
        .get("assets")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if assets.keys().cloned().collect::<BTreeSet<_>>() != expected_names {
        return fail(format!("Unexpected native asset list: {platform}"));
    }
    for (filename, checksum) in &assets {
        let actual = file_sha256(&output.join(filename))?;
        if checksum.as_str() != Some(actual.as_str()) {
            return fail(format!("Artifact checksum mismatch: {filename}"));
        }
        let recorded = fs::read_to_string(output.join(format!("{filename}.sha256")))?;
        if recorded != format!("{actual}  {filename}\n") {
            return fail(format!("Checksum file mismatch: {filename}"));
        }
    }
    let runtime = fs::read_to_string(output.join(format!("{name}.runtime-id")))?;
    if runtime.trim() != identity.runtime_id {
        return fail(format!("Runtime identity mismatch: {platform}"));
    }
    Ok(assets[&name].as_str().unwrap_or_default().to_owned())
}

fn native_result_name(platform: &str) -> String {
    format!("ccvl-{platform}.native-result.json")
}

/// The report of the native executor is about this very executable and this source,
/// and every check in it passed, the test suite among them.
fn native_result_matches(
    result: &Value,
    platform: &str,
    binary_sha256: &str,
    identity: &Identity,
) -> Result<()> {
    let text = |name: &str| result.get(name).and_then(Value::as_str);
    if result.get("schema") != Some(&json!(1))
        || text("platform") != Some(platform)
        || text("rust_host") != host_of(platform)
        || text("source_commit") != Some(identity.source_commit.as_str())
    {
        return fail(format!(
            "The native executor's report is not about this platform and source: {platform}"
        ));
    }
    if text("executable_sha256") != Some(binary_sha256) {
        return fail(format!(
            "The native executor ran a different executable: {platform}"
        ));
    }
    if text("runtime_id") != Some(identity.runtime_id.as_str()) {
        return fail(format!(
            "The native executor reports a different runtime identity: {platform}"
        ));
    }
    let checks = result
        .get("checks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for id in NATIVE_RESULT_CHECKS {
        let mut entries = checks
            .iter()
            .filter(|entry| entry.get("id").and_then(Value::as_str) == Some(id));
        match (entries.next(), entries.next()) {
            (Some(entry), None) if entry.get("ok") == Some(&json!(true)) => {}
            _ => {
                return fail(format!("The native executor did not pass {id}: {platform}"));
            }
        }
    }
    let count = |name: &str| {
        result
            .pointer(&format!("/tests/{name}"))
            .and_then(Value::as_u64)
    };
    if count("passed").is_none_or(|passed| passed == 0) || count("failed") != Some(0) {
        return fail(format!(
            "The native test suite did not pass completely: {platform}"
        ));
    }
    Ok(())
}

/// A native receipt of a platform this host can not run names the executor's report
/// (`native_result`), which sits next to it with the checksum the receipt records.
fn verify_native_result(
    output: &Path,
    platform: &str,
    receipt: &Value,
    binary_sha256: &str,
    identity: &Identity,
) -> Result<()> {
    let name = native_result_name(platform);
    let path = output.join(&name);
    if receipt.get("native_result").and_then(Value::as_str) != Some(name.as_str()) || !path.exists()
    {
        return fail(format!(
            "Missing native executor report: {platform} (ccvl release-evidence attest)"
        ));
    }
    if receipt.get("native_result_sha256").and_then(Value::as_str)
        != Some(file_sha256(&path)?.as_str())
    {
        return fail(format!("Native executor report checksum mismatch: {name}"));
    }
    native_result_matches(&load(&path)?, platform, binary_sha256, identity)
}

/// Re-issue the cross receipt of `platform` as a native one. The executable was built
/// here and ran in the native executor of its platform (for Windows `x86_64`: the offline
/// Windows clone), which reported on exactly that file; the report is kept next to the
/// receipt and the receipt records its checksum. Needs the package of the cross build in
/// `output`; replaces its cross receipt.
fn attest(root: &Path, env: &Env, platform: &str, result: &Path, output: &Path) -> Result<()> {
    let (identity, _) = source(root, env)?;
    if plan_of(platform) != Some(Plan::NativeWhenAvailable) {
        return fail(format!(
            "This platform has no native executor to attest: {platform}"
        ));
    }
    let host = host_of(platform).unwrap_or_default();
    let receipt_path = output.join(format!("ccvl-{platform}.receipt.json"));
    let cross = load(&receipt_path)?;
    identity_equal(&cross, &identity)?;
    let field = |name: &str| cross.get(name).and_then(Value::as_str);
    if field("kind") != Some("cross")
        || (field("platform"), field("rust_host")) != (Some(platform), Some(host))
        || cross.get("checks") != Some(&json!(["cross-build"]))
        || cross.get("tested") != Some(&json!(false))
        || !truthy(cross.get("tool"))
        || !has_provenance(&cross)
    {
        return fail(format!(
            "Only the cross-build receipt of {platform} can be attested."
        ));
    }
    let binary_sha256 = verify_package(output, platform, &cross, &identity)?;
    let report = fs::read(result).with_context(|| format!("cannot read {}", result.display()))?;
    let report_value: Value = serde_json::from_slice(&report)
        .with_context(|| format!("invalid JSON in {}", result.display()))?;
    native_result_matches(&report_value, platform, &binary_sha256, &identity)?;
    let report_path = output.join(native_result_name(platform));
    if report_path.exists() && fs::read(&report_path)? != report {
        return fail(format!(
            "Refusing to replace differing evidence: {}",
            report_path.display()
        ));
    }
    fs::write(&report_path, &report)
        .with_context(|| format!("cannot write {}", report_path.display()))?;
    let mut receipt = cross
        .as_object()
        .cloned()
        .context("The cross receipt is not an object.")?;
    receipt.remove("tested");
    // The run that built stays on record; the run that tested is the provenance now.
    receipt.insert("built_by_run".into(), cross["run"].clone());
    receipt.insert(
        "cross_receipt_sha256".into(),
        file_sha256(&receipt_path)?.into(),
    );
    receipt.extend(provenance(env)?);
    receipt.insert("kind".into(), "native".into());
    receipt.insert("checks".into(), json!(["public-check", "native-runtime"]));
    receipt.insert("native_result".into(), native_result_name(platform).into());
    receipt.insert(
        "native_result_sha256".into(),
        file_sha256(&report_path)?.into(),
    );
    fs::remove_file(&receipt_path)?;
    write_json(&receipt_path, &Value::Object(receipt))
}

fn gate(root: &Path, env: &Env, name: &str, output: &Path) -> Result<()> {
    let (identity, _) = source(root, env)?;
    let mut receipt = identity.map();
    receipt.extend(provenance(env)?);
    receipt.insert("kind".into(), "gate".into());
    receipt.insert("gate".into(), name.into());
    if matches!(name, "linux-deep" | "archive") {
        let path = output.join("ccvl-linux-x86_64.receipt.json");
        identity_equal(&load(&path)?, &identity)?;
        receipt.insert("native_receipt_sha256".into(), file_sha256(&path)?.into());
    }
    write_json(
        &output.join(format!("gate-{name}.json")),
        &Value::Object(receipt),
    )
}

/// Loose truthiness (empty and zero values are false) of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

fn has_provenance(receipt: &Value) -> bool {
    matches!(
        receipt.get("provider").and_then(Value::as_str),
        Some("forgejo")
    ) && truthy(receipt.get("run"))
}

/// The workflow whose run leaves the evidence of a platform that only its own executor can
/// produce (`release-windows` for the Windows clone, `release-macos` for the macOS clone).
fn leg_of(platform: &str) -> &'static str {
    if windows(platform) {
        "release-windows"
    } else {
        "release-macos"
    }
}

fn check(root: &Path, env: &Env, output: &Path, complete: bool) -> Result<()> {
    let (identity, _) = source(root, env)?;
    let platforms = release_platforms(root)?;
    let expected_native: BTreeSet<String> = platforms
        .iter()
        .map(|platform| format!("ccvl-{platform}.receipt.json"))
        .collect();
    for entry in
        fs::read_dir(output).with_context(|| format!("cannot read {}", output.display()))?
    {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.len() >= "ccvl-.receipt.json".len()
            && name.starts_with("ccvl-")
            && name.ends_with(".receipt.json")
            && !expected_native.contains(&name)
        {
            return fail("Native evidence outside the explicit release platform policy.");
        }
    }
    let mut receipts = BTreeMap::new();
    let mut runtimes = Map::new();
    // What the release holds per platform: `tested` (built and run on its own
    // platform), `built` (cross-built, never run: ARM64 by design, x86_64 until its
    // test executor exists) or `pending` (no executor yet; the release completes).
    let mut states = Map::new();
    for platform in &platforms {
        let host = host_of(platform).unwrap_or_default();
        let plan = plan_of(platform).unwrap_or(Plan::Native);
        let file = format!("ccvl-{platform}.receipt.json");
        let path = output.join(&file);
        if !path.exists() && PENDING_ALLOWED.contains(&platform.as_str()) {
            if complete {
                return fail(format!(
                    "Missing platform evidence: {platform} (its leg, {}, has not left a receipt)",
                    leg_of(platform)
                ));
            }
            states.insert(platform.clone(), json!("pending"));
            continue;
        }
        let receipt = load(&path)?;
        identity_equal(&receipt, &identity)?;
        let field = |name: &str| receipt.get(name).and_then(Value::as_str);
        let (kind, state, checks) = match (plan, field("kind")) {
            (Plan::Native | Plan::NativeWhenAvailable, Some("native")) => (
                "native",
                "tested",
                json!(["public-check", "native-runtime"]),
            ),
            (Plan::NativeWhenAvailable | Plan::BuiltOnly, Some("cross")) => {
                ("cross", "built", json!(["cross-build"]))
            }
            _ => return fail(format!("Missing native platform evidence: {platform}")),
        };
        if (field("platform"), field("rust_host")) != (Some(platform.as_str()), Some(host)) {
            return fail(format!("Missing native platform evidence: {platform}"));
        }
        if complete && plan == Plan::NativeWhenAvailable && state != "tested" {
            return fail(format!(
                "{platform} is only built: its leg, {}, has not tested it",
                leg_of(platform)
            ));
        }
        if receipt.get("checks") != Some(&checks)
            || !truthy(receipt.get("rustc"))
            || !truthy(receipt.get("cargo"))
        {
            return fail(format!("Incomplete native check evidence: {platform}"));
        }
        if kind == "cross"
            && (receipt.get("tested") != Some(&json!(false)) || !truthy(receipt.get("tool")))
        {
            return fail(format!("Incomplete cross-build evidence: {platform}"));
        }
        if !has_provenance(&receipt) {
            return fail(format!("Missing native run provenance: {platform}"));
        }
        let binary_sha256 = verify_package(output, platform, &receipt, &identity)?;
        if kind == "native" && windows(platform) {
            // This host can not run a Windows executable: the receipt stands on the
            // report of the executor that did (see `attest`).
            verify_native_result(output, platform, &receipt, &binary_sha256, &identity)?;
        }
        receipts.insert(file, file_sha256(&path)?);
        states.insert(platform.clone(), json!(state));
        runtimes.insert(
            platform.clone(),
            json!({"binary_sha256": binary_sha256, "rust_host": host, "state": state}),
        );
    }
    for name in GATES {
        let file = format!("gate-{name}.json");
        let path = output.join(&file);
        let receipt = load(&path)?;
        identity_equal(&receipt, &identity)?;
        let field = |key: &str| receipt.get(key).and_then(Value::as_str);
        if (field("kind"), field("gate")) != (Some("gate"), Some(name)) {
            return fail(format!("Missing release gate: {name}"));
        }
        if !has_provenance(&receipt) {
            return fail(format!("Missing gate run provenance: {name}"));
        }
        if matches!(name, "linux-deep" | "archive") {
            let native = receipts
                .get("ccvl-linux-x86_64.receipt.json")
                .context("ccvl-linux-x86_64.receipt.json")?;
            if field("native_receipt_sha256") != Some(native.as_str()) {
                return fail("Linux verification used a different native package.");
            }
        }
        receipts.insert(file, file_sha256(&path)?);
    }
    let version = release_version(root)?;
    let mut manifest = identity.map();
    manifest.insert("version".into(), json!(version.version));
    manifest.insert("release_kind".into(), json!(version.kind.name()));
    manifest.insert("platforms".into(), json!(platforms));
    manifest.insert("platform_states".into(), Value::Object(states.clone()));
    manifest.insert("receipts".into(), json!(receipts));
    write_json(&output.join("manifest.json"), &Value::Object(manifest))?;
    write_json(
        &output.join("runtime-manifest.json"),
        &json!({
            "schema": 1,
            "version": version.version,
            "runtime_id": identity.runtime_id,
            "cargo_lock_sha256": identity.cargo_lock_sha256,
            "platforms": runtimes,
            "platform_states": states,
        }),
    )?;
    println!("{} {}", identity.source_commit, identity.runtime_id);
    Ok(())
}
