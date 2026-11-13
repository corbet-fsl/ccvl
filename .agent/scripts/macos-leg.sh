#!/usr/bin/env bash
# The macOS leg of a minor release: build, test and package the macOS platforms in the macOS
# build clone and leave them in the draft of the commit.
#
#   macos-leg.sh        from the root of the checkout of the exact commit (the workflow
#                       .forgejo/workflows/release-macos.yml checks it out)
#
#   macos-x86_64   built natively in the clone, tested there (the whole Rust test suite, then the
#                  public check and the runtime checks of native-release.sh), packaged with a
#                  NATIVE receipt
#   macos-arm64    cross-built in the same clone with the Command Line Tools (cross-release.sh),
#                  never run: ARM is never emulated; packaged with a CROSS receipt, `built`
#
# The leg leaves exactly these files in the draft `build-<commit>`, and nothing else: the
# executables, their checksums and runtime identities, the workspace bundles and the receipts.
# The manifests and the checksum file are derived from the receipts when the release is
# promoted. It starts no other leg; it ends with dispatch-leg.sh, which says what is left.
#
# The clone is a Mac of 2026: bash 3.2 (no mapfile, no associative arrays), the BSD userland, no
# git-lfs and no node. Everything below runs there as it is. The job owns the clone for its
# length and the clone is rolled back afterwards; the toolchain, the compile cache and the build
# targets sit on its persistent cache disk (CI_CACHE_ROOT, CI_SOURCE_ROOT).
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

stage() { printf '\n== %s (%s s into the leg)\n' "$1" "$SECONDS"; }
refuse() { printf '%s\n' "$1" >&2; exit "${2:-2}"; }
sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }

# 1. This must be the macOS clone, on a Forgejo run of public ccvl main.
stage 'The clone'
[[ $(uname -s) == Darwin && $(uname -m) == x86_64 ]] || refuse 'The macOS leg runs in the macOS build clone (Darwin, x86_64).'
[[ $(sysctl -in sysctl.proc_translated 2>/dev/null || true) != 1 ]] || refuse 'A translated macOS process can not provide native evidence.'
[[ -z ${GH_TOKEN:-} ]] || refuse 'Publication credentials must not reach the macOS leg.'
[[ ${FORGEJO_ACTIONS:-} == true && ${GITHUB_REPOSITORY:-} == corbet-fsl/ccvl && ${GITHUB_REF_NAME:-} == main ]] \
  || refuse 'The macOS leg requires a Forgejo run on public ccvl main.'
for tool in jq curl git tar cmp shasum rustup cargo clang lipo; do
  command -v "$tool" >/dev/null || refuse "Missing tool: $tool (the macOS clone provides it)."
done
: "${CI_CACHE_ROOT:?the persistent cache disk of the clone}"
sha="${GITHUB_SHA:-}"
[[ $sha =~ ^[0-9a-f]{40}$ ]] || refuse 'The exact source commit (GITHUB_SHA) is required.'
export CI_COMMIT_SHA="$sha"
[[ $(git rev-parse HEAD) == "$sha" ]] || refuse "The checkout is not the commit of this run ($sha)."
printf 'macOS %s, %s, commit %s\n' "$(sw_vers -productVersion)" "$(uname -m)" "$sha"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-macos-leg.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT

# 2. The draft this leg adds to, and what the release holds, before anything is built: a
# leg dispatched for another commit, or for a release without macOS, ends here.
stage 'The draft'
# shellcheck source=.agent/scripts/forge-draft.sh
source .agent/scripts/forge-draft.sh
forge_setup
forge_call GET "$api/repos/$repository/branches/main"
[[ $(jq -r '.commit.id' "$scratch/response") == "$sha" ]] \
  || refuse "Source $sha is no longer main; its draft is stale and nothing is added to it."
draft="$scratch/draft.json"
status=0
forge_release "build-$sha" "$draft" || status=$?
[[ $status == 0 ]] || refuse "There is no draft build-$sha: the release run has not left one."
[[ $(jq -r '.draft' "$draft") == true ]] || refuse "build-$sha is public: its evidence is final."
forge_fetch_asset "$draft" manifest.json "$scratch/manifest.json" || refuse "The draft has no manifest.json to read the release from."
version="$(jq -er '.version' "$scratch/manifest.json")"
kind="$(jq -er '.release_kind' "$scratch/manifest.json")"
if [[ $kind != minor ]] || ! jq -e '.platforms | any(. == "macos-x86_64")' "$scratch/manifest.json" >/dev/null; then
  printf 'ccvl %s is a %s release: it holds no macOS platform, nothing to do.\n' "$version" "$kind"
  exit 0
fi
printf 'ccvl %s, a %s release; the draft build-%s holds %s asset(s).\n' "$version" "$kind" "${sha:0:12}" \
  "$(jq -r '.assets | length' "$draft")"

# 3. The source: the committed pointers are the archive (as the Linux run stages it), the LFS
# objects they name are in the checkout (the clone has no git-lfs).
stage 'The source'
git archive "$sha" > "$scratch/source.tar"
export SOURCE_ARCHIVE="$scratch/source.tar"
SOURCE_SHA256="$(sha256 "$SOURCE_ARCHIVE")"
export SOURCE_SHA256
bash .agent/scripts/lfs-fetch.sh .

# 4. The build environment. The target sits on the persistent cache disk, next to the compile
# cache that makes a rebuild cheap. The release scripts want the verified ccid target lock a
# Linux run holds; nothing else builds in this clone while the job owns it, so the target
# is the job's alone and the lock is held by construction.
export CARGO_TARGET_DIR="$CI_CACHE_ROOT/target/ccvl"
export CCID_TARGET_LOCK_HELD="$CARGO_TARGET_DIR"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}" CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-4}"
mkdir -p "$CARGO_TARGET_DIR"
# shellcheck source=.agent/scripts/rust-toolchain.sh
source .agent/scripts/rust-toolchain.sh
ccvl_select_rust_toolchain

# 5. The test suite, in the clone. The publication scripts and their fixtures run on Linux only
# (they need bash 4 and GNU tools; the tests say so); everything else runs here.
stage 'The Rust test suite'
tests_log="$scratch/cargo-test.log"
tests_rc=0
"${CCVL_CARGO_COMMAND[@]}" test --locked --workspace --all-features --no-fail-fast > "$tests_log" 2>&1 || tests_rc=$?
passed_failed="$(awk '/^test result:/ { passed += $4; failed += $6 } END { print passed + 0, failed + 0 }' "$tests_log")"
printf 'cargo test exit %s: %s tests passed, %s failed.\n' "$tests_rc" "${passed_failed% *}" "${passed_failed#* }"
if [[ $tests_rc != 0 ]]; then
  grep -E '^    [A-Za-z_0-9:]+$|panicked at|^error' "$tests_log" | sort -u | head -n 40 >&2 || true
  refuse 'The test suite failed in the macOS clone; nothing is packaged.' 1
fi

# 6. Build, run and package macos-x86_64; cross-build macos-arm64 beside it.
stage 'macos-x86_64: build, public check, runtime checks, package'
package="$scratch/package"
mkdir -p "$package"
bash .agent/scripts/native-release.sh macos-x86_64 "$package"
# The tested macOS executable is the evidence command from here on.
export CCVL_RELEASE_EVIDENCE_BIN="$package/ccvl-macos-x86_64"
stage 'macos-arm64: cross build, package'
bash .agent/scripts/cross-release.sh macos-arm64 "$package"

# 7. The evidence must be that of the Linux run: one source, one lock, one runtime identity.
stage 'Evidence'
if forge_fetch_asset "$draft" ccvl-linux-x86_64.receipt.json "$scratch/linux.receipt.json"; then
  identity='{schema, repository, source_commit, source_sha256, cargo_lock_sha256, runtime_id}'
  jq -S "$identity" "$scratch/linux.receipt.json" > "$scratch/linux.identity"
  for platform in macos-x86_64 macos-arm64; do
    jq -S "$identity" "$package/ccvl-$platform.receipt.json" > "$scratch/$platform.identity"
    cmp -s -- "$scratch/linux.identity" "$scratch/$platform.identity" \
      || refuse "The receipt of $platform differs from the Linux receipt in source, lock or runtime identity."
  done
  echo 'Source, lock and runtime identity equal those of the Linux run.'
else
  echo 'The draft holds no Linux receipt to compare with; the promotion checks the identities.'
fi
jq -e '.kind == "native" and .platform == "macos-x86_64" and .rust_host == "x86_64-apple-darwin"' \
  "$package/ccvl-macos-x86_64.receipt.json" >/dev/null || refuse 'The macos-x86_64 receipt is not a native one.'
jq -e '.kind == "cross" and .tested == false and .platform == "macos-arm64" and .rust_host == "aarch64-apple-darwin"' \
  "$package/ccvl-macos-arm64.receipt.json" >/dev/null || refuse 'The macos-arm64 receipt is not a built-only cross receipt.'

# 8. Into the draft: the files first, the receipt of a platform last (a receipt means the
# platform is complete). A file the draft holds with other bytes is replaced; the same is left.
stage 'The draft: upload'
for platform in macos-x86_64 macos-arm64; do
  for name in "ccvl-$platform" "ccvl-$platform.sha256" "ccvl-$platform.runtime-id" \
    "ccvl-$platform.tar.gz" "ccvl-$platform.tar.gz.sha256" "ccvl-$platform.receipt.json"; do
    forge_put "$draft" "build-$sha" "$package/$name"
  done
done
forge_release "build-$sha" "$draft"
for platform in macos-x86_64 macos-arm64; do
  for name in "ccvl-$platform" "ccvl-$platform.sha256" "ccvl-$platform.runtime-id" \
    "ccvl-$platform.tar.gz" "ccvl-$platform.tar.gz.sha256" "ccvl-$platform.receipt.json"; do
    held="$(forge_held "$draft" "$name")"
    [[ -n $held && ${held#* } == "$(forge_size "$package/$name")" ]] || refuse "The draft does not hold $name as it should."
  done
done
printf 'macos-x86_64: tested in the clone (%s tests, public check, runtime checks), sha256 %s\n' \
  "${passed_failed% *}" "$(sha256 "$package/ccvl-macos-x86_64")"
printf 'macos-arm64: built, never run, sha256 %s\n' "$(sha256 "$package/ccvl-macos-arm64")"

# 9. Whatever is left to run.
stage 'What is left'
bash .agent/scripts/dispatch-leg.sh
