#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Sourced from the root of a ccvl checkout by .ci/run-windows.sh: the tools that build
# the Windows test suite on a runner of the label `windows`. llvm-mingw (clang, lld and the
# mingw-w64 UCRT runtime of the one Windows ABI, gnullvm; .ci/llvm-mingw.nix) comes from
# the Nix store, and the Rust toolchain from the release job's persistent storage, whose
# gnullvm standard library is provisioned here, explicitly, under the shared toolchain lock.
# Needs CI_CACHE_ROOT (or CARGO_HOME) and leaves the lock held (fd 8) for the caller's life.
# shellcheck disable=SC2034 # the TOOLCHAIN_* settings are read by toolchain-lock.sh
cache_root=${CI_CACHE_ROOT:-${CARGO_HOME:-}}
[[ $cache_root = /* ]] || { echo 'An absolute persistent CI_CACHE_ROOT or CARGO_HOME is required' >&2; exit 2; }
toolchains=${CI_TOOLCHAIN_ROOT:-$cache_root/ccid-cached/toolchains}
[[ $toolchains = /* ]] || { echo 'CI_TOOLCHAIN_ROOT must be absolute' >&2; exit 2; }
mkdir -p "$toolchains"
# The pinned nixpkgs of the release job is the one definition of where the tools come from.
tool_source=$(sed -n 's/^tool_source=//p' "$PWD/.ci/release-build.sh")
[[ $tool_source == github:NixOS/nixpkgs/* && $tool_source != *[[:space:]]* ]] || {
  echo 'No pinned nixpkgs reference in .ci/release-build.sh.' >&2; exit 2
}
# Last on PATH: llvm-mingw's unprefixed clang and lld must never shadow the host toolchain.
llvm_mingw=$(bash "$PWD/.ci/llvm-mingw.sh" "$tool_source")
PATH="$PATH:$llvm_mingw/bin"
export RUSTUP_HOME="$toolchains/rustup"
export CARGO_HOME="$toolchains/cargo"
export RUSTUP_INIT_SKIP_PATH_CHECK=yes RUSTUP_TOOLCHAIN=stable
PATH="$CARGO_HOME/bin:$PATH"
export CI_JOBS="${CI_JOBS:-2}"
export CARGO_BUILD_JOBS="$CI_JOBS"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}"
# The shared toolchain lock, with the one target this job needs (the release run provisions it too).
TOOLCHAIN_LOCK_FILE="$toolchains/install.lock"
TOOLCHAIN_LOCK_TIMEOUT="${CI_TIMEOUT:-2700}"
TOOLCHAIN_WANT_NIGHTLY=0
TOOLCHAIN_WASM_TARGETS='stable:x86_64-pc-windows-gnullvm'
# shellcheck source=toolchain-lock.sh
source "$PWD/.ci/toolchain-lock.sh"
toolchain_ensure
cargo_bin=$("$CARGO_HOME/bin/rustup" which --toolchain stable cargo)
test -x "$cargo_bin"
PATH="$(dirname "$cargo_bin"):$PATH"
export PATH
