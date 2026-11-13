#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# The stages of the release job, in order, on a Forgejo runner of the label `linux`: this is the one
# `build` check of [release] (`ccid release build` runs it under the verified Cargo target lock and
# uploads what it leaves in $CCID_RELEASE_DIR to the draft of the commit). Release stages are never
# reused from a result cache: each run proves its own commit.
#
# What ccid does around it (docs/release.md of ccid): the version and the preflight (a version is
# released once, the source is main), the third-party lock refresh, the draft, the uploads, the
# platform legs and the promotion. What is left here is the part only ccvl knows: how its platforms
# are built and what their evidence is.
set -euo pipefail
exec 2>&1
: "${CCID_BIN:?Missing verified ccid job runtime}"
: "${CI_COMMIT_SHA:?Missing verified source revision}"
: "${CI_REPOSITORY_URL:?Missing canonical repository identity}"
: "${CCID_RELEASE_DIR:?Missing the release directory}"
cache_root=${CI_CACHE_ROOT:-${CARGO_HOME:-}}
[[ $cache_root = /* ]] || { echo 'An absolute persistent CI_CACHE_ROOT or CARGO_HOME is required' >&2; exit 2; }
namespace=$(printf '%s' "$CI_REPOSITORY_URL" | sha256sum | cut -c1-24)
state="$cache_root/ccid-cached/$namespace"
toolchains=${CI_TOOLCHAIN_ROOT:-$cache_root/ccid-cached/toolchains}
[[ $toolchains = /* ]] || { echo 'CI_TOOLCHAIN_ROOT must be absolute' >&2; exit 2; }
mkdir -p "$state" "$toolchains"
exec 9>"$state/release-lock"
flock -w "${CI_TIMEOUT:-2700}" 9
# Tools come from the pinned nixpkgs of the verify job (substituted, never built):
# the lint and PDF gates' tools, and zig with cargo-zigbuild for the Linux builds
# (x86_64 and aarch64, against a glibc 2.39 baseline).
tool_source=github:NixOS/nixpkgs/b7c2ada94fe99c15b0dbcf4d11fd7850b957a436?narHash=sha256-IItrdb7Puk05RqOBWZYFC5X6Wl1sJmCfh5MWVHw5iMM=
tool_paths=$(nix build --no-link --print-out-paths \
  "$tool_source#actionlint" "$tool_source#shellcheck" "$tool_source#reuse" \
  "$tool_source#poppler" "$tool_source#qpdf" "$tool_source#file" "$tool_source#jq" \
  "$tool_source#zig" "$tool_source#cargo-zigbuild")
for path in $tool_paths; do PATH="$path/bin:$PATH"; done
# Both Windows targets use one ABI, gnullvm, linked by llvm-mingw (clang, lld, the
# mingw-w64 UCRT runtime): the flake package .#llvm-mingw, whose source is the
# flake input llvm-mingw-dist (flake.lock, cached in Celler like every other
# toolchain). nixpkgs has no such package.
llvm_mingw=$(bash "$PWD/.ci/llvm-mingw.sh")
# Last on PATH: the Windows tools are all target-prefixed, and its unprefixed clang and
# lld must never shadow the host toolchain of the Linux builds.
PATH="$PATH:$llvm_mingw/bin"
export RUSTUP_HOME="$toolchains/rustup"
shared_cargo_home=${CARGO_HOME:-}
export CARGO_HOME="$toolchains/cargo"
export RUSTUP_INIT_SKIP_PATH_CHECK=yes RUSTUP_TOOLCHAIN=stable
export PATH="$(dirname "$CCID_BIN"):$CARGO_HOME/bin:$PATH"
export CI_JOBS="${CI_JOBS:-2}" CARGO_BUILD_JOBS="${CI_JOBS:-2}"
export RUST_TEST_THREADS="${CI_TEST_THREADS:-2}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}"
# The shared toolchain lock, with the three cross targets of the release (the two Windows targets share the gnullvm ABI).
TOOLCHAIN_LOCK_FILE="$toolchains/install.lock"
TOOLCHAIN_LOCK_TIMEOUT="${CI_TIMEOUT:-2700}"
TOOLCHAIN_WANT_NIGHTLY=0
TOOLCHAIN_WASM_TARGETS='stable:aarch64-unknown-linux-gnu stable:x86_64-pc-windows-gnullvm stable:aarch64-pc-windows-gnullvm'
# shellcheck source=toolchain-lock.sh
source "$(dirname "${BASH_SOURCE[0]}")/toolchain-lock.sh"
toolchain_ensure
cargo_bin=$("$CARGO_HOME/bin/rustup" which --toolchain stable cargo)
test -x "$cargo_bin"
export PATH="$(dirname "$cargo_bin"):$PATH"
export CCID_SOURCE_REVISION="$CI_COMMIT_SHA"
rustc -vV
cargo --version
zig version
cargo-zigbuild --version
x86_64-w64-mingw32-clang --version | head -1
export CCID_SOURCE_REVISION="$CI_COMMIT_SHA"
rustc -vV
cargo --version
zig version
cargo-zigbuild --version
x86_64-w64-mingw32-clang --version | head -1
# The stages share one directory: what is in it when they are done is the release.
export CCVL_RELEASE_DIR="$CCID_RELEASE_DIR"
for stage in rust lint native cross linux-deep archive check; do
  bash .agent/scripts/release-ci.sh "$stage"
done
