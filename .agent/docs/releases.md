# Compiled releases

Normal users download the Linux x86_64 workspace bundle from the
latest release of the repository. Each
archive contains the exact checked-in workspace plus its optimized native
binary in `.agent/cache/ccvl/bin/`. Extract it and run the included dispatcher;
neither Git nor Rust is required.

## Required delivery path

Forgejo is the single source and the only place a release is built. The `release`
workflow of `corbet-fsl/ccvl` runs on our own `linux` runners: it builds, verifies
and drafts the release of the exact `main` commit. The separate `release-promote`
workflow then makes the verified drafts public and copies them to GitHub and
GitLab. Both are manual actions. The GitHub mirror runs the checks of pull
requests on GitHub's own compute and never builds or publishes a release.
Current stable Rust is selected without an exact compiler pin; the runner keeps
it in persistent storage and every receipt records the actual compiler version.

The explicit released platform set is declared in `.agent/release-platforms.txt`
and listed by `ccvl release-evidence platforms`. Missing artifacts never shrink
that set. Each platform holds one of three states:

| State | Platforms | Evidence |
|---|---|---|
| `tested` | linux-x86_64 | built on the runner, then run there: public check and native runtime checks |
| `built` | linux-aarch64, windows-x86_64, windows-arm64 | cross-built on the Linux runner, never run there; ARM is never emulated, x86_64 Windows is tested later in its Windows build clone |
| `pending` | macos-x86_64, macos-arm64 | no executor yet; the release completes without them and records the state |

A `built` platform never claims tests, and a `pending` one has no asset. The
cross platforms are linked by `cargo zigbuild` (Windows for the GNU ABI, so no
Microsoft SDK is needed). The Linux x86_64 binary is linked the same way against
a glibc 2.39 baseline and the system loader, so it runs on an ordinary Linux and
not only inside the runner's Nix store.

`cargo zigbuild` was chosen over `cargo xwin` for the Windows targets by
measurement (2026-11-12, the Windows x86_64 release build, `measure4`):

| Lane | cargo-zigbuild (GNU ABI) | cargo-xwin (MSVC ABI) |
|---|---|---|
| CPU | 1395 CPU-s, 209 s wall | 1475 CPU-s, 280 s wall (includes the first SDK download) |
| RAM | peak 6.2 GiB | peak 8.1 GiB |
| IO | 965 MiB read | 757 MiB read, plus the SDK download |
| Storage | +1.4 GiB target | +1.4 GiB target and +1.1 GiB Windows SDK cache |

xwin also downloads the Microsoft CRT and SDK under their licence terms; zig
needs neither. The price is the GNU ABI, which a Windows user does not notice for
a console tool. Revisit the choice when the Windows build clone can build MSVC
natively.

For the tested platform, the native command verifies the running
OS/architecture and compiler host, builds the locked target, then runs public
document and native runtime checks before packaging. A cross-build is not native
evidence. Linux independent PDF verification reuses that exact executable. Rust
tests, Clippy, workflow/shell/license checks and the real Git/Rust-free archive
executor are separate required gates.

Each package includes a receipt binding its CI provider/run, exact source
commit, canonical workspace contents, source archive, Cargo.lock, compiler,
native host, runtime identity and asset checksums. The publisher requires every
released platform's receipt and four gate receipts from the same source and
dependency identity;
the Linux deep/archive gates additionally identify their exact native receipt.
Canonical source file/mode hashes establish equal source contents however the
source archive encodes tar metadata. Reports are retained beside the artifacts.
They are trusted CI evidence, not cryptographic attestations against a
compromised executor.

The `release` run ends with complete DRAFT releases on Forgejo, published with
the job's own token; the drafts are the review point. `release-promote` re-reads
the drafts of the exact commit, checks the whole set again and makes them public.
Existing assets are never overwritten or deleted, including draft assets.
Repeated publication verifies matching bytes and uploads only missing draft
assets.
Incomplete published releases, different bytes, missing native results and
superseded main revisions fail closed. Workspace release assets include full
native/gate receipts; the runtime release has a stable runtime manifest so a
matching immutable runtime can be reused by later workspace-only revisions.

## Release evidence command

Source identity, native packaging, gate receipts, verification and the
platform policy are one hidden maintainer command of the ccvl binary, not a
separate helper language:

```text
ccvl release-evidence source [--output FILE]
ccvl release-evidence native --platform P --binary FILE --host TRIPLE \
  --rustc VERSION --cargo VERSION --output DIR
ccvl release-evidence cross --platform P --binary FILE --host TRIPLE \
  --rustc VERSION --cargo VERSION --tool IDENTITY --output DIR
ccvl release-evidence gate rust|lint|linux-deep|archive --output DIR
ccvl release-evidence check --output DIR
ccvl release-evidence platforms
```

It runs from the repository root (or `--root DIR`) and reads the CI source
identity from the environment the Forgejo runner provides (`GITHUB_REPOSITORY`,
`CI_COMMIT_SHA` or `GITHUB_SHA`, an optional `SOURCE_ARCHIVE` with its
`SOURCE_SHA256`, and `GITHUB_RUN_ID`). Failures print the message alone and
exit 1. The release scripts reach it through
`.agent/scripts/release-evidence.sh`: `native-release.sh` first builds the
host binary and uses that exact tested binary, so no binary has to exist
beforehand; every other stage runs the command from the checked out source
with `cargo run --locked` on the already provisioned toolchain. Set
`CCVL_RELEASE_EVIDENCE_BIN` to a built binary to reuse it instead. Nothing is
installed implicitly.

## Forgejo release routes

The workflows are rendered by `ccid` from `.ci/ccid.toml`; edit the manifest,
never the generated `.forgejo/workflows/*.yml`. Each one is a thin shim that
hands over to `ccid job-run`, which stages the exact commit, takes the persistent
Cargo target lock and runs the job command.

| Workflow | Action | Runner requirement |
|---|---|---|
| `release` | `release-rust`, `release-lint`, `release-native`, `release-cross`, `release-linux-deep`, `release-archive`, `release-check`, `release-publish`, in this order, never reused from a cache | `linux` runner with Nix; the stage tools come from the pinned nixpkgs |
| `release-promote` | re-check the drafts of this commit, make them public, copy them downstream | `linux` runner; the downstream credential |
| `release-windows` | `release-windows`: fetch the shipped `ccvl-windows-x86_64.exe` from the drafts of this commit and run it in the offline Windows clone | `windows` runner (an ordinary srv slot that owns the clone for the job); the runner's read credential |

`.ci/run-release.sh` provisions the tools (zig, cargo-zigbuild, llvm-mingw, the
lint and PDF tools) and the three cross targets in the runner's persistent storage, then runs
each stage through `bash .agent/scripts/ci-check.sh release-<stage>`, which calls
`release-ci.sh <stage>`. All stages share one directory under the locked Cargo
target, keyed by source commit and run (`CCVL_RELEASE_DIR`). The native stage uses
`bash .agent/scripts/native-release.sh PLATFORM OUTPUT` and the cross stage
`bash .agent/scripts/cross-release.sh PLATFORM OUTPUT`; a future platform is
added to the release policy before it is advertised. The publisher,
`.agent/scripts/publish-release.sh`, speaks the Forgejo release API with the
job's own token and never replaces an existing asset.

### The Windows test: the file that ships is the file that runs

`release-windows` is its own manual run on the same `main` commit, after a
`release` run has left the drafts. It builds nothing. `windows-test.sh` reads the
`build-<commit>` draft for the Windows receipt, then the `runtime-<identity>` draft
for `ccvl-windows-x86_64.exe`, and stops unless the receipt (platform, Rust host
`x86_64-pc-windows-gnullvm`), the `.sha256` asset, the `.runtime-id` asset and the
downloaded bytes all agree. The executable then crosses into the clone (`vm-win`),
which has no network and none of Julian's shares, and runs from the clone's own
disk: its SHA-256 is checked once more inside, `--version` and `runtime-id` run
(the identity compiled into the binary must be the release identity), and
`public-check` renders and verifies every document from the source of the same
commit. The job fails if any step fails and prints the executable's SHA-256 and
the guest's results. A passing run does not yet turn the platform from `built`
into `tested` in the manifest: that needs the clone's result to feed a `native`
receipt, which is the next step. `CCVL_WINDOWS_ASSET_DIR=DIR` runs the same checks
on assets from a directory (a local rehearsal of a `cross-release.sh` output).

### Downstream copies

`release-promote` ends with `.agent/scripts/downstream-release.sh`, which copies
the public release to the targets in `.agent/release-downstreams.json` (GitHub and
GitLab; Bitbucket is a git mirror and has no releases). The upload credentials are
one runner-level secret, loaded as a systemd credential and named by
`CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE`; they are not per-organisation Forgejo
secrets, no build stage ever sees the variable, and the file is never printed. A
runner without that credential skips the copy with a clear `token missing`
message and the promotion still succeeds. The calls are few and strictly
sequential (two releases per copy, one upload per asset) to respect the GitHub
automation pacing policy, and a copy never replaces an existing asset.

An already provisioned sandbox can be selected for `release-archive` with the
paired `ARCHIVE_EXECUTOR` and `ARCHIVE_EXECUTOR_SHA256` inputs. The workflow
copies and verifies this standalone executable before passing it the verified
archive-check script and explicit package directory. The executor must run that
unchanged check in a real minimal filesystem and preserve its exact package and
CI identities. It must provide existing runtime tools without installing them or
exposing Git/Rust through another path. Keep host-specific sandbox configuration
outside ccvl. With neither input, the check runs directly and still requires an
already minimal worker; an incomplete or mismatched pair fails before execution.

**Executor availability remains a separate delivery prerequisite.** Verify the
actual native OS/architecture, existing tools and minimal filesystem for each
run. Keep host inventory and provisioning details outside ccvl. These routes do
not provision workers or turn Linux evidence into another platform's evidence.

A binary linked to Nix store paths needs a separately verified ordinary-Linux
runtime path before distribution. When adapting a retained native executable,
preserve its original compile provenance, record the exact transformation and
run public/runtime and Linux deep checks on the final bytes before publication.
The unchanged archive check must then run that exact bundle in a real minimal
filesystem with its ordinary runtime libraries and Git/Rust absent. Record the
tested OS/libc scope: the released Linux x86_64 path is verified on Ubuntu 24.04
with glibc 2.39. This does not establish compatibility with every Linux
distribution or older libc. A sandbox exposing Nix dependencies proves only that
environment. Neither a masked PATH nor a Linux cross-build supplies missing
native or ordinary-system compatibility evidence.

## Immutable release tags

Two immutable release tags serve different purposes:

- `runtime-<source-fingerprint>` contains the released platforms' executables,
  checksums, identities, and a runtime manifest. Setup uses this exact tag.
- `build-<git-commit>` contains the released platforms' workspace bundles and
  checksums. This is the user-facing latest release, including current templates, documents and
  exact native/gate receipts.

Workspace-only changes reuse a matching runtime identity while still producing
a new set of complete user bundles. A failed release leaves the last verified
download intact; a checkout with newer runtime source refuses that old binary.
The runtime manifest also binds the released platform set. Widening that set
cannot overwrite an existing immutable runtime release; its tag and asset
identity must be resolved explicitly before publication.

## Source identity and setup

The build embeds a SHA-256 identity over `Cargo.toml`,
`.agent/core/Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
`.agent/build.rs`, `ccvl.json` (whose workspace paths the binary embeds), the
sorted Rust source files in `.agent/src/` and `.agent/core/src/`, and the
engine Typst package files (`*.typ`, `*.toml` and `*.json` directly in
`.agent/typst/`), which the binary embeds. Each input contributes its relative path and SHA-256 digest; the
sources sort by byte order of their relative paths.
The Rust, Bash, and PowerShell implementations must produce the same value;
native CI tests enforce this agreement.

The launcher checks the executable's `runtime-id` before every command. The
executable independently checks its workspace, including when called directly.
CV text, style Typst templates, profile data, interview evidence, and
opportunities are runtime inputs rather than compiled code and do not force
recompilation.

Setup downloads the matching runtime from
`<origin>/releases/download/runtime-<fingerprint>/`, where `<origin>` is the
HTTPS form of the checkout's `origin` remote; `CCVL_RELEASE_BASE` overrides it.
An extracted release bundle has no origin remote and needs no download because
it ships its binary.
Setup verifies both the download checksum and embedded source identity before
installing it. It never labels an arbitrary downloaded binary as current and
never silently falls back to compilation. Developers use `setup --from-source`
to request a local build with a suitable installed stable compiler, or a
repository-local stable toolchain when none meets the `Cargo.toml` minimum.
The Forgejo runner reuses a provisioned stable compiler in its persistent
storage and records the actual compiler version.
The channel file remains part of the runtime source identity; download hashes,
source fingerprints, every released target's native checks and all four shared
gates remain mandatory.

## Reusing work

The Forgejo runner retains dependencies in dedicated, locked Cargo storage keyed
by repository identity. Workspace executables receive current source-bound
verification on each run; a cache hit alone is not a passing check. Debug
information is disabled for the CI test build to reduce cache size. Successful
dependency work remains available when a later test fails, so fixes do not
repeat a cold build.

Release binaries are built once per platform per run and retained for later
gates and publication under the run's release directory. Published downloads
remain release assets. The GitHub mirror's pull-request checks use GitHub's free
public runners. Private downstream content stays on trusted internal workers,
with its own ownership and document gates.

Linux independent verification runs one public check and retains its freshly
verified PDFs for Poppler, QPDF and pixel comparisons. Each variant still
compiles twice inside Rust and once in a separate process. Download identity
checking uses `doctor`; complete setup remains tested in the clean archive
environment. No document-validation result is reused across CI runs.
