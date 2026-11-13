//! Check one private downstream on a Forgejo runner using its matching published runtime.
//!
//! Maintainer route, hidden from the normal help: `ccvl private-check`. It runs
//! in the checkout of the private repository that the Forgejo job staged (the
//! exact commit), driven by the environment the Forgejo runner provides (see
//! `.agent/docs/private-downstream.md`). It stages everything it verifies from
//! that checkout and the forge itself; it never merges or pushes.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Error, Result};
use regex::Regex;
use serde_json::{Value, json};

use crate::archive::{self, Env, Kind, dumps, lfs_pointer, sha256_hex, unsafe_member_path};

const PUBLIC_REPO: &str = "corbet-fsl/ccvl";
const ASSET: &str = "ccvl-linux-x86_64";
/// Publication credentials. None may reach this stage.
const CREDENTIALS: [&str; 8] = [
    "GH_TOKEN",
    "DOWNSTREAM_PUSH_KEY",
    "CARGO_REGISTRY_TOKEN",
    "CARGO_REGISTRIES_CRATES_IO_TOKEN",
    "NPM_TOKEN",
    "NODE_AUTH_TOKEN",
    "PYPI_TOKEN",
    "JSR_TOKEN",
];
/// The job's own token. Forgejo exports it under all three names; it is the only
/// token this stage may see, and only when every name carries the same value.
const JOB_TOKENS: [&str; 3] = ["CCID_FORGE_TOKEN", "FORGEJO_TOKEN", "GITHUB_TOKEN"];
/// Variables no child process of this stage receives: the job's tokens (a child
/// that needs one gets it explicitly), the runner's artifact and OIDC tokens, and
/// the path of the runner-level release credential.
const SCRUBBED: [&str; 6] = [
    "CCID_FORGE_TOKEN",
    "FORGEJO_TOKEN",
    "GITHUB_TOKEN",
    "ACTIONS_RUNTIME_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    "CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE",
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
        for key in inherited.iter().map(String::as_str).chain(SCRUBBED) {
            env.remove(key);
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
        self.run_with(label, program, args, cwd, &[], None)
    }

    /// Like `run`, with extra environment for this one command and optional
    /// standard input. A token travels this way, never in the arguments.
    fn run_with(
        &self,
        label: &str,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        extra: &[(&str, &str)],
        stdin: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let mut attempts = 0;
        let output = loop {
            let mut command = Command::new(program);
            command
                .args(args)
                .env_clear()
                .envs(self.env.pairs())
                .envs(extra.iter().copied())
                .stdin(if stdin.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
            match command.spawn() {
                // A freshly written executable can be briefly busy.
                Err(error)
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 =>
                {
                    attempts += 1;
                    sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error).with_context(|| format!("cannot run {program}")),
                Ok(mut child) => {
                    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
                        // A child that exits early is judged by its status.
                        let _ = pipe.write_all(bytes);
                    }
                    break child
                        .wait_with_output()
                        .with_context(|| format!("cannot run {program}"))?;
                }
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
/// Per file: executable bits, the archive's content digest and, for a Git LFS
/// pointer, the object id the pointer names.
type Inventory = std::collections::BTreeMap<String, (u32, String, Option<String>)>;

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
        let object = lfs_pointer(&entry.data).map(|(oid, _)| oid);
        files.insert(
            entry.name,
            (entry.mode & 0o111, sha256_hex(&entry.data), object),
        );
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

/// The repository this run belongs to, after the runner-side guards: a Forgejo
/// runner (not hosted GitHub Actions), a manual dispatch, and a repository that
/// is not the public one. Privacy itself is proven against the forge afterwards.
fn runner_repository(env: &Env) -> Result<String> {
    let repository = env.nonempty("GITHUB_REPOSITORY");
    require(
        env.get("FORGEJO_ACTIONS") == Some("true")
            && env.get("RUNNER_ENVIRONMENT") != Some("github-hosted")
            && repository.is_some_and(|repository| repository != PUBLIC_REPO),
        "Private downstream checks require a private repository on a Forgejo runner.",
    )?;
    require(
        env.get("GITHUB_EVENT_NAME") == Some("workflow_dispatch"),
        "Private downstream checks require a manual dispatch.",
    )?;
    Ok(repository.unwrap_or_default().to_owned())
}

/// The job token, refused unless every name Forgejo exports it under agrees.
fn job_token(env: &Env) -> Result<String> {
    let token = env.first_nonempty(&JOB_TOKENS).unwrap_or_default();
    require(
        pattern(r"^[A-Za-z0-9_.-]{8,}$").is_match(token)
            && JOB_TOKENS
                .iter()
                .all(|key| env.nonempty(key).is_none_or(|value| value == token)),
        "The Forgejo job token is required, and it must be the only token present.",
    )?;
    Ok(token.to_owned())
}

/// Prove with the forge's own answer that the repository is private. The token
/// goes to curl on standard input.
fn prove_private(
    runner: &Runner,
    scratch: &Path,
    api: &str,
    repository: &str,
    token: &str,
) -> Result<()> {
    let output = scratch.join("repository.json");
    let output_text = output.to_string_lossy().into_owned();
    runner.run_with(
        "Repository privacy",
        "curl",
        &[
            "-q",
            "--fail",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--silent",
            "--show-error",
            "-K",
            "-",
            "--output",
            &output_text,
            &format!("{api}/repos/{repository}"),
        ],
        None,
        &[],
        Some(format!("header = \"Authorization: token {token}\"\n").as_bytes()),
    )?;
    let answer: Value = serde_json::from_slice(&fs::read(&output)?)?;
    require(
        answer.get("private") == Some(&Value::Bool(true))
            && answer
                .get("full_name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(repository)),
        "Private downstream checks require a private repository on a Forgejo runner.",
    )
}

/// Where the public repository is served to this runner. The canonical public
/// URL names the repository; the forge that runs the job serves it too (jobs
/// have no route to the public host).
fn served_url(server: &str, public_url: &str) -> String {
    let path = public_url
        .strip_prefix("https://")
        .and_then(|rest| rest.split_once('/'))
        .map_or("", |(_, path)| path);
    format!("{}/{path}", server.trim_end_matches('/'))
}

#[allow(clippy::too_many_lines)]
fn check(root: &Path, env: &Env) -> Result<Value> {
    let repository = runner_repository(env)?;
    let server = env.get("GITHUB_SERVER_URL").unwrap_or_default().to_owned();
    require(
        pattern(r"^https://[^/\s]+$").is_match(server.trim_end_matches('/')),
        "The Forgejo server URL is required.",
    )?;
    let api = env.nonempty("GITHUB_API_URL").map_or_else(
        || format!("{}/api/v1", server.trim_end_matches('/')),
        str::to_owned,
    );
    require(
        pattern(r"^https://[^/\s]+(/\S*)?$").is_match(&api),
        "The Forgejo API URL is invalid.",
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
    let token = job_token(env)?;
    let commit = env
        .first_nonempty(&["CI_COMMIT_SHA", "GITHUB_SHA"])
        .unwrap_or_default();
    require(is_hex(commit, 40), "An exact private commit is required.")?;
    let explicit_upstream = env.nonempty("UPSTREAM_COMMIT");
    require(
        explicit_upstream.is_none_or(|value| is_hex(value, 40)),
        "UPSTREAM_COMMIT must be a full commit id.",
    )?;
    let runner = Runner::new(env);
    // The checkout is the staged source: its HEAD is the commit and its files
    // are the committed files.
    let head = runner.run(
        "Checkout identity",
        "git",
        &["rev-parse", "HEAD"],
        Some(root),
    )?;
    require(
        String::from_utf8_lossy(&head).trim() == commit,
        "The checkout is not the requested private commit.",
    )?;
    let committed = runner.run(
        "Committed source identity",
        "git",
        &["archive", commit],
        Some(root),
    )?;
    let files = source_files(&committed, commit)?;
    // Check the actual extraction too, not merely the committed tree.
    for (name, (mode, checksum, object)) in &files {
        let path = root.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        let digest = if metadata.is_file() {
            sha256_hex(&fs::read(&path)?)
        } else {
            String::new()
        };
        // A committed LFS pointer is satisfied by the object it names.
        require(
            metadata.is_file()
                && executable_bits(&path)? == *mode
                && (digest == *checksum || object.as_deref() == Some(digest.as_str())),
            "Extracted source differs from its archive.",
        )?;
    }
    let scratch = tempfile::Builder::new()
        .prefix("ccvl-private-check-")
        .tempdir()?;
    // Everything below may talk to the forge; the private claim comes first.
    prove_private(&runner, scratch.path(), &api, &repository, &token)?;
    let shallow = runner.run(
        "History state",
        "git",
        &["rev-parse", "--is-shallow-repository"],
        Some(root),
    )?;
    if String::from_utf8_lossy(&shallow).trim() == "true" {
        runner.run_with(
            "Complete Git history",
            "git",
            &["fetch", "--quiet", "--unshallow", "origin"],
            Some(root),
            &[
                ("GIT_CONFIG_COUNT", "1"),
                ("GIT_CONFIG_KEY_0", "http.extraHeader"),
                (
                    "GIT_CONFIG_VALUE_0",
                    &format!("Authorization: token {token}"),
                ),
            ],
            None,
        )?;
    }
    let checkout = scratch.path().join("source");
    let checkout_text = checkout.to_string_lossy().into_owned();
    let root_text = root.to_string_lossy().into_owned();
    // A clone of an incomplete history is refused by Git itself; the clone has
    // no hooks, configuration or credentials of the job checkout.
    runner.run(
        "Private checkout",
        "git",
        &[
            "clone",
            "--quiet",
            "--no-checkout",
            "--no-local",
            "--template=",
            &root_text,
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
    let complete = runner.run(
        "History state",
        "git",
        &["rev-parse", "--is-shallow-repository"],
        Some(&checkout),
    )?;
    require(
        String::from_utf8_lossy(&complete).trim() == "false",
        "The private history is incomplete.",
    )?;
    // Compare full path/content/executable inventories; the clone must yield
    // the very tree the job checked out.
    let cloned = runner.run(
        "Committed source identity",
        "git",
        &["archive", commit],
        Some(&checkout),
    )?;
    require(
        source_files(&cloned, commit)? == files,
        "Source archive differs from the committed Git tree.",
    )?;
    let policy: Value =
        serde_json::from_str(&fs::read_to_string(checkout.join("ccvl-downstream.json"))?)?;
    let configuration = policy.get("upstream").cloned().unwrap_or_else(|| json!({}));
    let remote = configuration
        .get("remote")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let branch = configuration
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or("main");
    let policy_url = configuration.get("url").and_then(Value::as_str);
    let public_url = env
        .nonempty("CCVL_UPSTREAM_URL")
        .or(policy_url)
        .unwrap_or_default()
        .to_owned();
    require(
        pattern(r"^https://[^/\s]+/\S+\.git$").is_match(&public_url)
            && policy_url.is_none_or(|url| url == public_url)
            && pattern(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$").is_match(remote)
            && pattern(r"^[A-Za-z0-9][A-Za-z0-9._/-]*$").is_match(branch),
        "Private policy must name the public ccvl upstream.",
    )?;
    let mirror_url = served_url(&server, &public_url);
    // A runner whose public repository lives elsewhere names where to fetch it.
    let reach = env
        .nonempty("CCVL_UPSTREAM_FETCH_URL")
        .map_or_else(|| mirror_url.clone(), str::to_owned);
    let upstream = if let Some(upstream) = explicit_upstream {
        upstream.to_owned()
    } else {
        // The newest public commit this downstream already contains.
        runner.run(
            "Public upstream",
            "git",
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                &reach,
                &format!("+refs/heads/{branch}:refs/ccvl/upstream"),
            ],
            Some(&checkout),
        )?;
        let base = runner.run(
            "Upstream ancestry",
            "git",
            &["merge-base", commit, "refs/ccvl/upstream"],
            Some(&checkout),
        )?;
        String::from_utf8_lossy(&base).trim().to_owned()
    };
    require(
        is_hex(&upstream, 40),
        "Exact private and upstream commits are required.",
    )?;
    runner.run(
        "Upstream ancestry",
        "git",
        &["merge-base", "--is-ancestor", &upstream, commit],
        Some(&checkout),
    )?;
    // The clone has no private origin URL or push credentials.
    runner.run(
        "Remove clone origin",
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
        "This route requires the existing Linux x86_64 executor.",
    )?;
    // The runtime is an asset of the public release named by the version of this
    // source (`v<version>`); its identity is compared with the source's below.
    let release =
        crate::release_evidence::release_version(&checkout).context("workspace release version")?;
    let base = format!(
        "{}/releases/download/v{}",
        mirror_url.strip_suffix(".git").unwrap_or(&mirror_url),
        release.version
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
            &upstream,
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
        "private": true,
        "source_commit": commit,
        "upstream_commit": upstream,
        "source_archive_sha256": sha256_hex(&committed),
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
