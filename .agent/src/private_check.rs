//! Check one private downstream on Crow using its matching published runtime.
//!
//! Maintainer route, hidden from the normal help: `ccvl private-check`. It is
//! driven entirely by environment variables set by the Crow route (see
//! `.agent/docs/private-downstream.md`) and runs in the extracted private
//! source tree. It never merges or pushes.

use std::fmt;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Error, Result, anyhow};
use regex::Regex;
use serde_json::{Value, json};

use crate::archive::{self, Env, Kind, dumps, sha256_hex, unsafe_member_path};

const PUBLIC_REPO: &str = "corbet-libs/ccvl";
const ASSET: &str = "ccvl-linux-x86_64";
const CREDENTIALS: [&str; 9] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "DOWNSTREAM_PUSH_KEY",
    "CARGO_REGISTRY_TOKEN",
    "CARGO_REGISTRIES_CRATES_IO_TOKEN",
    "NPM_TOKEN",
    "NODE_AUTH_TOKEN",
    "PYPI_TOKEN",
    "JSR_TOKEN",
];

/// A refusal whose message is safe to show: it names a stage, never content.
#[derive(Debug)]
struct Refusal(String);

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::new(Refusal(message.to_owned())))
    }
}

fn pattern(expression: &str) -> Regex {
    Regex::new(expression).expect("valid expression")
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Execute the check in the current directory; print a receipt or a fixed
/// failure stage.
#[must_use]
pub fn run(root: Option<&Path>) -> ExitCode {
    let outcome = root
        .map_or_else(
            || std::env::current_dir().context("cannot resolve the current directory"),
            |root| Ok(root.to_path_buf()),
        )
        .and_then(|root| check(&root, &Env::from_process()));
    match outcome {
        Ok(receipt) => {
            println!("{}", dumps(&receipt, None));
            ExitCode::SUCCESS
        }
        Err(error) => {
            // Do not print exception payloads: a malformed private policy or
            // missing private path could otherwise leak its contents into an
            // upstream log.
            eprintln!("{}", failure_text(&error));
            ExitCode::from(2)
        }
    }
}

fn failure_text(error: &Error) -> String {
    error.downcast_ref::<Refusal>().map_or_else(
        || "Private check inputs are unavailable or invalid.".to_owned(),
        ToString::to_string,
    )
}

/// Commands run with a scrubbed Git environment and their output captured:
/// gate errors can contain private document text or paths, so only the
/// failing stage is reported.
struct Runner {
    env: Env,
}

impl Runner {
    fn new(env: &Env) -> Self {
        // No checkout hooks, credential helpers, replacement objects, or
        // private remote configuration are inherited by this job-only repository.
        let mut env = env.clone();
        let inherited: Vec<String> = env
            .pairs()
            .map(|(key, _)| key.clone())
            .filter(|key| key.starts_with("GIT_"))
            .collect();
        for key in inherited {
            env.remove(&key);
        }
        for (key, value) in [
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_NO_REPLACE_OBJECTS", "1"),
        ] {
            env.set(key, value);
        }
        Self { env }
    }

    fn run(
        &self,
        label: &str,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let mut attempts = 0;
        let output = loop {
            let mut command = Command::new(program);
            command
                .args(args)
                .env_clear()
                .envs(self.env.pairs())
                .stdin(Stdio::null());
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
            match command.output() {
                // A freshly written executable can be briefly busy.
                Err(error)
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 =>
                {
                    attempts += 1;
                    sleep(Duration::from_millis(50));
                }
                other => break other.with_context(|| format!("cannot run {program}"))?,
            }
        };
        require(
            output.status.success(),
            &format!("{label} failed; no private content was published."),
        )?;
        Ok(output.stdout)
    }
}

/// Path, executable bit and checksum of every regular file of a source archive.
type Inventory = std::collections::BTreeMap<String, (u32, String)>;

fn source_files(data: &[u8], commit: &str) -> Result<Inventory> {
    let archive = archive::read(data)?;
    require(
        archive.comment.as_deref() == Some(commit),
        "Source archive commit mismatch.",
    )?;
    let mut files = Inventory::new();
    for entry in archive.entries {
        require(
            !unsafe_member_path(&entry.name),
            "Unsafe source archive path.",
        )?;
        if entry.kind == Kind::Directory {
            continue;
        }
        require(
            entry.kind == Kind::File && !files.contains_key(&entry.name),
            "Private checks require unique regular source files.",
        )?;
        files.insert(entry.name, (entry.mode & 0o111, sha256_hex(&entry.data)));
    }
    Ok(files)
}

fn executable_bits(path: &Path) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        Ok(fs::metadata(path)?.permissions().mode() & 0o111)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(0)
    }
}

fn required(env: &Env, key: &str) -> Result<String> {
    env.get(key)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("missing {key}"))
}

fn check(root: &Path, env: &Env) -> Result<Value> {
    let public_url = env.get("CCVL_UPSTREAM_URL").unwrap_or_default().to_owned();
    let repository = env.nonempty("CI_REPO");
    require(
        env.get("CI") == Some("crow")
            && env.get("CI_REPO_PRIVATE") == Some("true")
            && repository.is_some_and(|repository| repository != PUBLIC_REPO)
            && env.nonempty("GITHUB_ACTIONS").is_none(),
        "Private downstream checks require a private repository on Crow.",
    )?;
    require(
        env.get("CI_PIPELINE_EVENT") == Some("manual"),
        "Private downstream checks require a manual event.",
    )?;
    require(
        pattern(r"^https://[^/\s]+/\S+\.git$").is_match(&public_url),
        "Set CCVL_UPSTREAM_URL to the public ccvl repository (https://<forge>/<owner>/ccvl.git).",
    )?;
    let target = env.nonempty("CARGO_TARGET_DIR");
    require(
        target.is_some() && env.get("CCID_TARGET_LOCK_HELD") == target,
        "Private downstream checks require the verified ccid target lock.",
    )?;
    require(
        CREDENTIALS.iter().all(|key| env.nonempty(key).is_none()),
        "Publication credentials are not accepted by the private check stage.",
    )?;
    let commit = env.get("CI_COMMIT_SHA").unwrap_or_default();
    let upstream = env.get("UPSTREAM_COMMIT").unwrap_or_default();
    require(
        is_hex(commit, 40) && is_hex(upstream, 40),
        "Exact private and upstream commits are required.",
    )?;
    for key in ["SOURCE_SHA256", "DOWNSTREAM_SOURCE_SHA256"] {
        require(
            is_hex(env.get(key).unwrap_or_default(), 64),
            "Missing source or bundle digest.",
        )?;
    }
    let archive = fs::read(required(env, "SOURCE_ARCHIVE")?)?;
    require(
        Some(sha256_hex(&archive).as_str()) == env.get("SOURCE_SHA256"),
        "Source archive checksum mismatch.",
    )?;
    let files = source_files(&archive, commit)?;
    // Check the actual ccid extraction too, not merely its archive header.
    for (name, (mode, checksum)) in &files {
        let path = root.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        require(
            metadata.is_file()
                && executable_bits(&path)? == *mode
                && sha256_hex(&fs::read(&path)?) == *checksum,
            "Extracted source differs from its archive.",
        )?;
    }
    let bundle = fs::canonicalize(required(env, "DOWNSTREAM_SOURCE_BUNDLE")?)?;
    let bundle_text = bundle.to_string_lossy().into_owned();
    require(
        Some(sha256_hex(&fs::read(&bundle)?).as_str()) == env.get("DOWNSTREAM_SOURCE_SHA256"),
        "Git bundle checksum mismatch.",
    )?;
    let runner = Runner::new(env);
    let heads = runner.run(
        "Bundle identity",
        "git",
        &["bundle", "list-heads", &bundle_text],
        None,
    )?;
    require(
        String::from_utf8_lossy(&heads).trim() == format!("{commit} refs/heads/source"),
        "Git bundle commit mismatch.",
    )?;
    let scratch = tempfile::Builder::new()
        .prefix("ccvl-private-check-")
        .tempdir()?;
    let verifier = scratch.path().join("verify.git");
    let verifier_text = verifier.to_string_lossy().into_owned();
    runner.run(
        "Empty bundle verifier",
        "git",
        &["init", "--bare", "--template=", "--quiet", &verifier_text],
        None,
    )?;
    runner.run(
        "Complete Git history",
        "git",
        &["bundle", "verify", &bundle_text],
        Some(&verifier),
    )?;
    let checkout = scratch.path().join("source");
    let checkout_text = checkout.to_string_lossy().into_owned();
    runner.run(
        "Private checkout",
        "git",
        &[
            "clone",
            "--quiet",
            "--no-checkout",
            "--template=",
            &bundle_text,
            &checkout_text,
        ],
        None,
    )?;
    runner.run(
        "Exact private checkout",
        "git",
        &["checkout", "--quiet", "--detach", commit],
        Some(&checkout),
    )?;
    // Compare full path/content/executable inventories; two transport files
    // bearing the same commit header cannot substitute different source.
    let committed = runner.run(
        "Committed source identity",
        "git",
        &["archive", commit],
        Some(&checkout),
    )?;
    require(
        source_files(&committed, commit)? == files,
        "Source archive differs from the committed Git bundle tree.",
    )?;
    runner.run(
        "Upstream ancestry",
        "git",
        &["merge-base", "--is-ancestor", upstream, commit],
        Some(&checkout),
    )?;
    let policy: Value =
        serde_json::from_str(&fs::read_to_string(checkout.join("ccvl-downstream.json"))?)?;
    let configuration = policy.get("upstream").cloned().unwrap_or_else(|| json!({}));
    let remote = configuration
        .get("remote")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = configuration.get("url");
    require(
        (url.is_none()
            || url == Some(&Value::Null)
            || url.and_then(Value::as_str) == Some(public_url.as_str()))
            && pattern(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$").is_match(remote),
        "Private policy must name the public ccvl upstream.",
    )?;
    // The Git bundle clone has no private origin URL or push credentials.
    runner.run(
        "Remove bundle origin",
        "git",
        &["remote", "remove", "origin"],
        Some(&checkout),
    )?;
    runner.run(
        "Policy upstream",
        "git",
        &["remote", "add", remote, &public_url],
        Some(&checkout),
    )?;
    // The bootstrap identity algorithm, applied to the exact private checkout.
    let fingerprint =
        crate::runtime_source::fingerprint(&checkout).context("workspace runtime identity")?;
    require(
        is_hex(&fingerprint, 64),
        "Invalid workspace runtime identity.",
    )?;
    require(
        cfg!(all(target_os = "linux", target_arch = "x86_64")),
        "This Crow route requires the existing Linux x86_64 executor.",
    )?;
    let base = format!(
        "{}/releases/download/runtime-{fingerprint}",
        public_url.strip_suffix(".git").unwrap_or(&public_url)
    );
    let checksum_file = scratch.path().join(format!("{ASSET}.sha256"));
    let checksum_text = checksum_file.to_string_lossy().into_owned();
    let curl = |label: &str, output: &str, url: &str| {
        runner.run(
            label,
            "curl",
            &[
                "-q",
                "--fail",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--tlsv1.2",
                "--silent",
                "--show-error",
                "--output",
                output,
                url,
            ],
            None,
        )
    };
    curl(
        "Matching published runtime checksum unavailable",
        &checksum_text,
        &format!("{base}/{ASSET}.sha256"),
    )?;
    let published = fs::read_to_string(&checksum_file)?;
    let checksum = published
        .strip_suffix('\n')
        .unwrap_or(&published)
        .strip_suffix(&format!("  {ASSET}"))
        .filter(|digest| is_hex(digest, 64));
    require(checksum.is_some(), "Invalid published runtime checksum.")?;
    let checksum = checksum.unwrap_or_default();
    let binary = scratch.path().join(ASSET);
    if let Some(existing) = env.nonempty("CCVL_CHECK_RUNTIME") {
        // An existing artifact is accepted only against the current published
        // checksum and runtime ID; its path alone is not release evidence.
        fs::copy(existing, &binary)?;
    } else {
        curl(
            "Matching published runtime unavailable",
            &binary.to_string_lossy(),
            &format!("{base}/{ASSET}"),
        )?;
    }
    require(
        sha256_hex(&fs::read(&binary)?) == checksum,
        "Published runtime checksum mismatch.",
    )?;
    make_private_executable(&binary)?;
    let binary_text = binary.to_string_lossy().into_owned();
    let identity = runner.run(
        "Published runtime identity",
        &binary_text,
        &["runtime-id"],
        Some(&checkout),
    )?;
    require(
        String::from_utf8_lossy(&identity).trim() == fingerprint,
        "Published runtime is stale or does not match this source.",
    )?;
    runner.run(
        "Downstream ownership gate",
        &binary_text,
        &[
            "downstream-check",
            "--policy",
            "ccvl-downstream.json",
            "--upstream-ref",
            upstream,
        ],
        Some(&checkout),
    )?;
    runner.run(
        "Full private document gate",
        &binary_text,
        &["check"],
        Some(&checkout),
    )?;
    Ok(json!({
        "schema": 1,
        "repository": repository,
        "source_commit": commit,
        "upstream_commit": upstream,
        "source_archive_sha256": sha256_hex(&archive),
        "git_bundle_sha256": env.get("DOWNSTREAM_SOURCE_SHA256"),
        "runtime_id": fingerprint,
        "binary_sha256": checksum,
        "checks": ["downstream-check", "check"],
        "merged": false,
        "pushed": false,
    }))
}

fn make_private_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
