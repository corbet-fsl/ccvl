#!/usr/bin/env bash
# Execute the manifest's selected checks with content-keyed result reuse.
set -euo pipefail
exec 2>&1
: "${CCID_BIN:?Missing verified ccid job runtime}"
: "${CI_COMMIT_SHA:?Missing verified source revision}"
: "${CI_REPOSITORY_URL:?Missing canonical repository identity}"
: "${CHECKS:?Missing declared check selection}"
if [ "$CHECKS" != "rust,documents,lint,styles" ]; then
  echo "CHECKS must be exactly 'rust,documents,lint,styles', got: $CHECKS" >&2
  exit 2
fi
cache_root=${CI_CACHE_ROOT:-${CARGO_HOME:-}}
[[ $cache_root = /* ]] || { echo 'An absolute persistent CI_CACHE_ROOT or CARGO_HOME is required' >&2; exit 2; }
namespace=$(printf '%s' "$CI_REPOSITORY_URL" | sha256sum | cut -c1-24)
state="$cache_root/ccid-cached/$namespace"
toolchains=${CI_TOOLCHAIN_ROOT:-$cache_root/ccid-cached/toolchains}
[[ $toolchains = /* ]] || { echo 'CI_TOOLCHAIN_ROOT must be absolute' >&2; exit 2; }
mkdir -p "$state" "$toolchains"
exec 9>"$state/lock"
flock -w "${CI_TIMEOUT:-2700}" 9
mkdir -p "$state/moon-home"
# .moon/cache/hashes and .moon/cache/outputs are NOT created here: `ccid cached`
# links both to the shared result store (CCID_RESULT_CACHE/moon/) itself and
# refuses to use a job whose directories are anything else ("Moon artifact
# cache belongs to a different coordinator"), which ran every check uncached
# and recorded no verdict. The publication-review gate (.agent/src/public.rs,
# validate_boundary) allows exactly those two links.
# Tool installation is an explicit part of this job, in persistent CI storage.
# Pinned nixpkgs b7c2ada (narHash-pinned): actionlint 1.7.12 matches ci.yml
# exactly; shellcheck/reuse provide lint gates via existing-tool-path;
# poppler/qpdf/file/jq provide the `documents` gate's independent PDF checks
# (check-linux-deep.sh requires file qpdf pdfinfo pdftotext pdffonts
# pdfdetach pdfimages pdftoppm jq). No nightly, no nodejs, no wasm; none of the
# Linux verify gates need them. No git insteadOf: Cargo.lock has zero git
# sources.
tool_source=github:NixOS/nixpkgs/b7c2ada94fe99c15b0dbcf4d11fd7850b957a436?narHash=sha256-IItrdb7Puk05RqOBWZYFC5X6Wl1sJmCfh5MWVHw5iMM=
tool_paths=$(nix build --no-link --print-out-paths "$tool_source#moon" "$tool_source#proto" "$tool_source#actionlint" "$tool_source#shellcheck" "$tool_source#reuse" "$tool_source#poppler" "$tool_source#qpdf" "$tool_source#file" "$tool_source#jq")
for path in $tool_paths; do PATH="$path/bin:$PATH"; done
export PROTO_HOME="$toolchains/proto" RUSTUP_HOME="$toolchains/rustup"
shared_cargo_home=${CARGO_HOME:-}
export CARGO_HOME="$toolchains/cargo" MOON_HOME="$state/moon-home"
export PROTO_AUTO_CLEAN=false PROTO_TELEMETRY=false PROTO_YES=true
export RUSTUP_INIT_SKIP_PATH_CHECK=yes RUSTUP_TOOLCHAIN=stable
export PATH="$(dirname "$CCID_BIN"):$CARGO_HOME/bin:$PROTO_HOME/shims:$PROTO_HOME/bin:$PATH"
export CI_JOBS="${CI_JOBS:-2}" CARGO_BUILD_JOBS="${CI_JOBS:-2}"
export RUST_TEST_THREADS="${CI_TEST_THREADS:-2}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}"
# Shared-first toolchain lock: jobs share a provisioned toolchain and take
# exclusive only to provision missing pieces. Same lock file, so old
# whole-job-exclusive runners interoperate (they just serialize).
TOOLCHAIN_LOCK_FILE="$toolchains/install.lock"
TOOLCHAIN_LOCK_TIMEOUT="${CI_TIMEOUT:-2700}"
TOOLCHAIN_WANT_NIGHTLY=0
TOOLCHAIN_WASM_TARGETS=
# shellcheck source=toolchain-lock.sh
source "$(dirname "${BASH_SOURCE[0]}")/toolchain-lock.sh"
toolchain_ensure
cargo_bin=$("$CARGO_HOME/bin/rustup" which --toolchain stable cargo)
test -x "$cargo_bin"
export PATH="$(dirname "$cargo_bin"):$PATH"
export CCID_SOURCE_REVISION="$CI_COMMIT_SHA"
"$CCID_BIN" render --repo "$PWD" --check
proto --version
moon --version
rustc -vV
cargo --version
actionlint --version
shellcheck --version
reuse --version
file --version
qpdf --version
pdfinfo -v 2>&1 | head -n 2
jq --version
# Cached checks get the declared environment only, never the runner's credentials
# (cmnp: "Credentials and arbitrary inherited variables are not handed to
# cacheable work"). The runner's CC/CXX name the compile-cache wrapper
# (sccache-corbet), which in that environment starts an sccache server without a
# signing credential and fails every C compile of a build script (ring, psm).
# Plain compilers keep the cached checks independent of it; Rust compiles keep
# the kache cache, whose remote lives in a file the declared environment names.
export CC=cc CXX=c++
"$CCID_BIN" cached --repo "$PWD" --check "$CHECKS"
IFS=, read -r -a selected_checks <<< "$CHECKS"
for check in "${selected_checks[@]}"; do test -s ".ccid/results/$check.jsonl"; done
