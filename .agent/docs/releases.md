# Compiled releases

Normal users download the Linux x86_64 workspace bundle from the
latest release of the repository. Each
archive contains the exact checked-in workspace plus its optimized native
binary in `.agent/cache/ccvl/bin/`. Extract it and run the included dispatcher;
neither Git nor Rust is required.

## Required delivery path

Forgejo is the single source and the only place a release is built. The release is the
`[release]` table of `.ci/ccid.toml` and the one workflow `ccid render` writes from it,
`.forgejo/workflows/release.yml` (ccid `docs/release.md`): one run, four jobs ordered by
`needs`. `release` runs on our own `linux` runners: it builds, verifies and drafts the
release of the exact `main` commit (`.ci/release-build.sh`); for a minor release the
platform legs (below) run after it on their own runners; `release-promote` then makes
the verified draft the public release `v<version>` on Forgejo and does nothing else:
the files reach the copies on GitHub and GitLab by an upload from the primary's side
(see "Mirrors" below). The run is a manual action, dispatched
deliberately, never per commit. No copy on another forge runs CI, so none builds or
publishes a release.
Current stable Rust is selected without an exact compiler pin; the runner keeps
it in persistent storage and every receipt records the actual compiler version.

## Release policy: semantic versioning, released sparingly

Julian, 2026-11-13: "use semantic versioning. we are not at 1.0 - we should not
release for every patch or minor version. And for Mac and Windows / arm even more
sparingly. We must keep down the compute."

Nothing is a knob: the version in `Cargo.toml` (the only place it is written)
decides everything, and `ccvl release-evidence version` is the one place that reads
it.

- **The release is `v<version>`**, named the version (`0.1.0`), the one public
  release of that version. Below 1.0 a version is exactly `0.MINOR.PATCH` in plain
  digits. A pre-release or build suffix (`0.2.0-rc.1`) names no release, and `1.0`
  and later wait for a decision on this policy: the run refuses both, in its first
  stage, before anything is built.
- **A version is released once.** The run refuses when a release or a tag
  `v<version>` exists, draft or public. Releasing again means raising the version.
- **The platform set follows the version**, with no input:

  | Version | Kind | Platforms |
  |---|---|---|
  | `0.y.0` | minor | all six: Linux, macOS and Windows x86_64 built and tested where an executor exists; Linux, macOS and Windows ARM64 built, never run |
  | `0.y.z`, `z > 0` | patch | `linux-x86_64` only |

  A patch release therefore builds and uploads one platform. A Mac, Windows or
  ARM user at a patch version has no prebuilt runtime for it and either takes the
  runtime of the last minor release or builds from source (`setup --from-source`).
- **Per-commit CI stays small.** The `ci` workflow verifies on Linux only, with
  cached results. No commit ever builds or tests macOS, Windows or ARM64: those run
  in a release, and only in a minor one.
- **Releases are batched.** Raising the version is cheap and releasing is not, so
  changes accumulate on `main` until a release is worth it; a patch release is for
  a fix someone needs on Linux, a minor release is the occasion for the other
  platforms. A bump is not a release and a release is never started by a commit.

To release: raise `version` in `Cargo.toml` on `main` (the version is part of the
runtime identity, so the bump is the commit that is released) and dispatch `release`
on that commit. The run goes to the end by itself: a minor release runs `release-windows`
and then `release-macos` after `release` (the workflow's `needs`; nothing polls, nothing is
dispatched, there is no input to set), and `release-promote` runs when every leg that ran
has passed. It refuses a minor release whose platform legs have not left their final
evidence (the `release-seal` check), so a failed leg stops the run with a draft that can
not be promoted: repair the cause and dispatch `release` again on the same commit (the
draft is resumed; `ccid release discard` clears one whose bytes cannot match). A patch
release has no legs.

The declared platform set is `.agent/release-platforms.txt`; a release holds all of
it or, for a patch, `linux-x86_64`, as listed by `ccvl release-evidence platforms`.
Missing artifacts never shrink that set. Each platform holds one of three states:

| State | Platforms | Evidence |
|---|---|---|
| `tested` | linux-x86_64 | built on the runner, then run there: public check and native runtime checks |
| `tested` | windows-x86_64 | cross-built on the Linux runner, then the shipped executable and the whole test suite pass in the offline Windows clone (`release-windows`, below), which re-issues its receipt as a native one |
| `tested` | macos-x86_64 | built natively in the macOS build clone and tested there: the whole Rust test suite, the public check and the runtime checks (`release-macos`, below) |
| `built` | linux-aarch64, windows-arm64, macos-arm64 | cross-built, never run; ARM is never emulated. macos-arm64 is cross-built in the macOS clone with the Command Line Tools |
| `pending` | macos-x86_64, macos-arm64, in a fresh draft only | the macOS leg has not run yet: no receipt, no asset |

A `built` platform never claims tests, and a `pending` one has no asset. The states
of a fresh draft are not a state a release may end in: `windows-x86_64` is `built`
until the Windows clone has tested it and the macOS platforms are `pending` until the
macOS clone has built them. `release-promote` checks the set as **complete**
(`ccvl release-evidence check --complete`): no platform `pending`, and the platforms
that have a native executor (`windows-x86_64`, `macos-x86_64`) `tested` on it. A
minor release whose legs have not all passed is refused, naming the leg to repair. The
Linux aarch64 binary is linked by `cargo zigbuild`, and so is the Linux x86_64
binary, against a glibc 2.39 baseline and the system loader, so they run on an
ordinary Linux and not only inside the runner's Nix store.

Both Windows platforms use one ABI, `gnullvm` (`x86_64-pc-windows-gnullvm` and
`aarch64-pc-windows-gnullvm`): a plain `cargo build --target` linked by the clang
and lld of llvm-mingw against its mingw-w64 UCRT runtime. The tool is the upstream
release archive as the flake input `llvm-mingw-dist` (flake.nix, pinned in
flake.lock and pushed to the srv Celler ci cache on every CI run, so a GitHub
outage never blocks the build; `.ci/llvm-mingw.nix` only unpacks and patches it
for the Nix loader, `.ci/llvm-mingw.sh` prints its store path): nothing is
compiled, every licence is free, no Microsoft SDK is accepted, and it ships the
`synchronization` import library that Rust's standard library needs. The package
carries no `libunwind.dll.a`, so Rust's `-lunwind` can only resolve to the static
library whatever the Rust version (rustc 1.99 took the import library, 1.100 does not).
The cross stage names the C, C++ and archive tools of the target itself (a runner's
generic `CC` would otherwise be used for the Windows target) and refuses an
executable that imports a mingw runtime DLL, so a clean Windows 10 or newer needs
nothing installed; the x86_64 file is the one the Windows clone runs.

The choice was made by measurement (2026-11-13, `measure4`, the release profile,
a fresh target directory with warm downloads and no compile cache, 8 jobs, two
runs each; the second run of each pair in the opposite order):

| x86_64 release build | llvm-mingw (gnullvm) | cargo-xwin (MSVC) | cargo-zigbuild (GNU) |
|---|---|---|---|
| CPU | 842 and 962 CPU-s | 885 and 916 CPU-s | fails |
| wall | 174 and 208 s | 189 and 191 s | fails |
| RAM | peak 6.06 and 5.50 GiB | 5.54 and 5.97 GiB | fails |
| IO | 254 to 377 MiB read | 30 to 185 MiB read | fails |
| Storage | +1.42 GiB target, 642 MiB toolchain in the Nix store | +1.41 GiB target, the Windows SDK cache (259 MiB to 1.1 GiB) | fails |

llvm-mingw is within 1 % of xwin on the mean of every lane, far below the 5 %
the choice allowed, and a warm rebuild is 4 % to 5 % cheaper (217 against 226
CPU-s, 54 against 57 s). The run-to-run noise on a shared host is about 10 %.
For aarch64-pc-windows-gnullvm the same measurement is 855 and 958 CPU-s, 196 and
216 s, 5.64 and 5.49 GiB, +1.39 GiB. Neither alternative builds that target: zig
cannot link `synchronization` (for either Windows target), and cargo-xwin stops in
the `psm` build script of `aarch64-pc-windows-msvc` (clang-cl cannot assemble the
GNU-syntax file psm picks for a cross build, and cargo-xwin sets the compiler of the
target itself, so a `CC` override does not help). xwin also needs the Microsoft SDK
licence accepted, which is Julian's decision to make and was not needed.

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

The `release` run ends with one DRAFT release on Forgejo, published with the job's
own token; the legs of a minor release add their platforms to it, and the draft is
the review point. `release-promote` re-reads the draft of the exact commit, checks the
whole set again as complete, derives `manifest.json` and `runtime-manifest.json` from
the receipts afresh (the legs ran after the draft was made, so the draft's copies may
be older: where they differ they are replaced in the draft), adds `SHA256SUMS` and
makes it the public release `v<version>`.

The promotion rule: promote may replace ONLY the manifests (`manifest.json`,
`runtime-manifest.json`) and `SHA256SUMS`, ONLY while the release is a draft,
nothing else.

How assets are written, as the code does it. A published release is never changed.
On a DRAFT, the `release` run's publisher removes an asset whose size differs from
our file of that name (the forge keeps the part of an interrupted upload) and
uploads it again; an asset of the right size is never touched, and if its bytes
differ from our file they are refused, never replaced. A leg that runs again
replaces the files of its own platform that differ from what it now produced, in the
draft only, receipt last. The promotion replaces the two manifests and `SHA256SUMS`,
which are derived data, where they differ from what the receipts say. Nothing else
is ever overwritten or deleted. A draft left by a run that died is not resumed by the
next run (its receipts name the run that made them): delete it, by hand, and release
again.

The release publishes more than 400 MB over the home uplink, which has run anywhere from
a few KB/s to 0.8 MB/s. The checks of a release therefore get a deadline of two hours (ccid
sets it; its default for other checks is 45 minutes), and one upload may take 600 s plus one
second per 20 KB of the file, in `ccid release` (the release job and the legs). Incomplete published releases, different bytes,
missing native results, platform legs that have not passed and superseded main
revisions fail closed. The release carries the executables, the workspace bundles,
the full native/gate receipts, a runtime manifest that binds the version, the
runtime identity and the executables' checksums, and `SHA256SUMS`.

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
ccvl release-evidence attest --platform P --result FILE --output DIR
ccvl release-evidence gate rust|lint|linux-deep|archive --output DIR
ccvl release-evidence check [--complete] --output DIR
ccvl release-evidence platforms
ccvl release-evidence version
```

It runs from the repository root (or `--root DIR`) and reads the CI source
identity from the environment the Forgejo runner provides (`GITHUB_REPOSITORY`,
`CI_COMMIT_SHA` or `GITHUB_SHA`, an optional `SOURCE_ARCHIVE` with its
`SOURCE_SHA256`, and `GITHUB_RUN_ID`). Failures print the message alone and
exit 1. The release scripts reach it through
`.agent/scripts/release-evidence.sh`: `native-release.sh` first builds the
host binary and uses that exact tested binary, so no binary has to exist
beforehand; `version` prints the release version with its kind (`0.1.0 minor`) and
refuses a version that names no release; every other stage runs the command from the checked out source
with `cargo run --locked` on the already provisioned toolchain. Set
`CCVL_RELEASE_EVIDENCE_BIN` to a built binary to reuse it instead. Nothing is
installed implicitly.

## Forgejo release routes

The workflows are rendered by `ccid` from `.ci/ccid.toml`; edit the manifest,
never the generated `.forgejo/workflows/*.yml`. Each one is a thin shim that
hands over to `ccid job-run`, which stages the exact commit, takes the persistent
Cargo target lock and runs the job command.

| Job of `release.yml` | Check (from `[release]`) | Runner requirement |
|---|---|---|
| `release` | `release-build` (`.ci/release-build.sh`): the stages `rust`, `lint`, `native`, `cross`, `linux-deep`, `archive`, `check` in this order, never reused from a cache; before it `ccid release build` refuses a version that names no release or is released already, a source that is not `main` and a red verdict, and refreshes the third-party locks; after it ccid drafts the release and uploads every file | `linux` runner |
| `release-windows` | `release-windows` (`.ci/run-windows.sh`): fetch the shipped `ccvl-windows-x86_64.exe` from the draft of this commit, run it and the gnullvm test suite in the offline Windows clone, then make the draft say `tested` | `windows` runner (an ordinary slot that owns the offline clone for the job) |
| `release-macos` | `release-macos` (`.agent/scripts/macos-leg.sh`): build, test and package macOS in the macOS clone (below) | `macos` runner (the macOS build clone); the runner's read credential |
| `release-promote` | `release-seal` (`.agent/scripts/release-seal.sh`): re-check the draft of this commit as complete (every platform leg has passed) and derive the two manifests; then ccid writes `SHA256SUMS` and makes it the public release `v<version>`; nothing is copied anywhere | `linux` runner |

`.ci/release-build.sh` provisions the tools (zig, cargo-zigbuild, llvm-mingw, the
lint and PDF tools) and the three cross targets in the runner's persistent storage, then runs
each stage through `bash .agent/scripts/release-ci.sh <stage>`. All stages share one directory,
`$CCID_RELEASE_DIR` (`CCVL_RELEASE_DIR`): what is in it when they are done is the release, and
`ccid release build` uploads it to the draft. The native stage uses
`bash .agent/scripts/native-release.sh PLATFORM OUTPUT` and the cross stage
`bash .agent/scripts/cross-release.sh PLATFORM OUTPUT`; a future platform is
added to the release policy before it is advertised. The draft, its uploads (checked by size on
the release, resumable) and the promotion are `ccid release` (ccid `docs/release.md`); the legs
read the draft with `ccid release fetch` and leave their evidence with `ccid release put`.

### The Windows test: the file that ships and the suite that vouches for it

`release-windows` is the second job of the same run, on the same `main` commit: it starts
once `release` has left the draft (a minor release only; a patch release has no leg and the
job is skipped). It always runs before `release-promote` renames the draft. A patch release
holds no Windows executable.
Otherwise it has three parts, all for the one ABI, `x86_64-pc-windows-gnullvm`.

1. **The file that ships.** `windows-test.sh` reads the `build-<commit>` draft for the
   Windows receipt and `ccvl-windows-x86_64.exe`, and stops unless the receipt (platform,
   Rust host), the `.sha256` asset, the `.runtime-id` asset and the downloaded bytes all
   agree. The executable crosses into the clone (`vm-win`), which has no network and none
   of Julian's shares, and runs from the clone's own disk
   (`windows-clone-exe.ps1`): its SHA-256 is checked once more inside, `--version` and
   `runtime-id` run (the identity compiled into the binary must be the release identity),
   and `public-check` renders and verifies every document from the source of the same commit.
2. **The suite that vouches for it.** The same script builds the test executables of the
   workspace (`cargo test --locked --workspace --all-features --no-run --target
   x86_64-pc-windows-gnullvm`, with `-Wl,--exclude-all-symbols` because the dev profile of
   ctypst's `cdylib` would otherwise export more symbols than a DLL can hold) on the
   runner, with the tools that built the executable:
   `.ci/windows-env.sh` provisions llvm-mingw from `.ci/llvm-mingw.sh` and the Rust
   toolchain with the gnullvm standard library, and `llvm-mingw-env.sh` names the linker
   and the per-target `CC_`, `CXX_` and `AR_` tools, the very file `cross-release.sh` uses.
   The executables embed the absolute path of the checkout, so the clone gets the
   checkout at the same path below the share (`vm-win mirror`) and runs every executable
   (`windows-clone-suite.ps1`) from its own disk, from the root of that tree. A
   copy from the share that fails in the guest is retried and counted, never skipped.
   Doc tests are not part of it (they need a host that runs the target).
3. **The native receipt.** `windows-test.sh` writes a JSON report: one entry per check
   (`sha256`, `architecture`, `version`, `runtime-id`, `public-check`, `test-suite`), the
   test counts, the activation facts of the clone and the compiler and tools of the test
   build. `windows-attest.sh` fetches the draft, and `ccvl release-evidence attest`
   accepts the report only if it is about this very executable (its SHA-256 and runtime
   identity), this source commit and this platform, every check passed and the suite ran
   without a failure. It then re-issues the cross receipt as a **native** one
   (`checks: ["public-check", "native-runtime"]`, `tested` removed, the run that tested is
   the provenance, `built_by_run` and `cross_receipt_sha256` keep the build on record) and
   keeps the report next to it as `ccvl-windows-x86_64.native-result.json`, whose checksum
   the receipt records. `release-evidence check` refuses a native Windows receipt without
   that report, with a report that was altered or that is not about the executable in the
   package: nobody can make a platform `tested` by editing a receipt. The manifests are
   written again by `check` itself, so promotion, which checks the whole draft once more,
   writes the same bytes; the draft then holds the new `receipt`, `manifest.json` and
   `runtime-manifest.json`, the report, and the replaced cross receipt as
   `ccvl-windows-x86_64.cross-receipt.json`. Nothing is published, and a published release
   is never touched. Run twice, the second run changes nothing.

The job fails if the executable or any test fails, and the draft is then left as it was.
`CCVL_WINDOWS_ASSET_DIR=DIR` runs the first two parts on assets from a directory (a local
rehearsal of a `cross-release.sh` output); it writes the report and attests nothing.

### The macOS leg: built, tested and packaged where it runs

`release-macos` is the macOS twin of the Windows leg, the third job of the run. It is rendered
like the others (it needs a macOS ccid on the tool channel, which the runner's tool environment
names: the hand-written workflow it replaces existed only because ccid had none). The job of the
label `macos` runs inside the macOS build clone as the user `ci` (bash 3.2, the BSD userland,
the Command Line Tools, Rust and the compile cache on the clone's persistent disk, no git-lfs,
no node) and the clone is rolled back after it. `ccid job-run` puts the exact commit at the
stable source path of that disk (`CCID_SOURCE_ROOT`) and `ccid release leg macos` runs
`.agent/scripts/macos-leg.sh`:

1. **Before any work**: `ccid release leg macos` has checked that the commit is still `main`
   and that the draft `build-<commit>` exists and is a draft; the leg itself checks that the
   release holds macOS (`$CCID_RELEASE_PLATFORMS`; a patch release does not).
2. **The source.** `git archive` of the commit is the source archive of the evidence (the
   committed LFS pointers, as the Linux run stages it), and `lfs-fetch.sh` puts the LFS
   objects over the pointers without git-lfs (the LFS API, each object checked against
   its pointer's size and SHA-256), so the checkout the build sees is the source the Linux
   run built from.
3. **The suite.** `cargo test --locked --workspace --all-features --no-fail-fast` runs in the
   clone. The publication scripts and their fixtures need bash 4 and GNU tools and are
   gated to Linux; everything else runs. A failure stops the leg before anything is packaged.
4. **macos-x86_64** is built natively (`native-release.sh`: the release profile, the public
   check, the runtime checks) and packaged with a **native** receipt.
5. **macos-arm64** is cross-built in the same clone with the Command Line Tools
   (`cross-release.sh`), checked to be an arm64 Mach-O and never run; its **cross** receipt
   says `built`.
6. **Evidence.** The two receipts must equal the draft's Linux receipt in source, lock and
   runtime identity before anything is uploaded.
7. **The draft.** The executables, their checksums and runtime identities, the bundles and
   the receipts of both platforms go into the draft, the receipt of a platform last. A
   file the draft holds with other bytes is replaced, the same is left (`ccid release put`);
   the manifests are not touched here (the promotion derives them).
8. The workflow orders the next job; nothing is dispatched.

The suite and the builds share the clone's compile cache (kache with the srv remote). Measured
on the first run, 2026-11-13: 23 minutes in all, of which the test suite took 5, the macos-x86_64
release build 8, the cold macos-arm64 cross build 10 and the upload of 12 files (210 MB) 4; a
leg with a warm cache is shorter.

### Mirrors: the files are uploaded from the primary's side

Promotion publishes on Forgejo and nothing else, and ccvl holds no credential for any other
host. No job runs on a copy (operating model R6, R7): the release files reach the GitHub and
GitLab copies through each forge's API, uploaded from the primary's side (R45), and nothing
in this repository renders a mirror workflow. `SHA256SUMS` lists every other asset of the
release (coreutils format, byte order, itself excluded), so a copy can be verified against it;
the promotion writes it last, so it covers the manifests as published. Bitbucket has no
releases and is a git mirror.

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

## Release identities

One release per version, and the other names are identities, not releases:

- `v<version>` is the public release: the tag of the version's commit, immutable.
  It holds every asset of the version: the released platforms' executables with
  their `.sha256` files and `.runtime-id` files, the workspace bundles
  (`ccvl-<platform>.tar.gz` with checksums, the user-facing download including
  current templates and documents), the native/gate receipts, the manifests
  (`manifest.json`, `runtime-manifest.json`) and `SHA256SUMS`, the checksums of every
  other asset, which a copy can verify against.
- `build-<git-commit>` is the internal name of the draft of one run's commit. It
  never turns public as such: it is renamed to `v<version>` by `release-promote`
  (a draft has no git tag yet, so the rename is the publication, and the tag
  appears on that commit). Drafts of different commits never meet, and a draft of
  an earlier commit is stale: it is deleted by hand, never reused.
- The runtime identity (`runtime_id`, the SHA-256 over the runtime sources, see
  below) stays in every receipt, in `runtime-manifest.json`, in the `.runtime-id`
  assets and in the release notes. It is no longer a release name: the version
  is part of the identity (`Cargo.toml` is one of its inputs), so every released
  version has a runtime of its own and nothing is reused across releases.

A failed release leaves the last published release intact; a checkout with newer
runtime source refuses an old binary. The runtime manifest binds the released
platform set of its version.

### How a downstream finds the runtime

A published runtime is found by the version of the source, then proved by its
checksum and identity, never by trust in the name:

1. the source names its release: `v<version>`, the version in its `Cargo.toml`;
2. `<releases>/v<version>/ccvl-linux-x86_64.sha256` and `.../ccvl-linux-x86_64`
   are downloaded (a patch release holds Linux x86_64 and is enough for the
   Linux x86_64 downstream check);
3. the executable must match the published checksum, and its `runtime-id` must
   equal the identity computed from the source being checked.

`setup` does exactly this. (`ccvl private-check` no longer needs a published runtime:
between releases there is none, so it is anchored on the upstream commit's successful
`ci` run, see [private-downstream](private-downstream.md).)
A source whose runtime sources differ from the release of its version (any commit
after the release that touched the runtime, or an unreleased version) fails the
third step as stale; it needs a release of its own version or, for development,
`setup --from-source`. Missing assets or an unreachable release host fail the
prerequisite and are never answered with a source build.

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
`<origin>/releases/download/v<version>/`, where `<origin>` is the
HTTPS form of the checkout's `origin` remote and `<version>` the version in
`Cargo.toml`; `CCVL_RELEASE_BASE` overrides it. Only a released version has a
runtime to download.
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
remain release assets. Private downstream content stays on trusted internal workers,
with its own ownership and document gates.

Linux independent verification runs one public check and retains its freshly
verified PDFs for Poppler, QPDF and pixel comparisons. Each variant still
compiles twice inside Rust and once in a separate process. Download identity
checking uses `doctor`; complete setup remains tested in the clean archive
environment. No document-validation result is reused across CI runs.
