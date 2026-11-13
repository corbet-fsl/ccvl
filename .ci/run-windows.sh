#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# The Windows leg of a release (`release-windows` of [release]): on a Forgejo runner of the label
# `windows`, the executable the release run left in its draft, and the test suite built for the same
# ABI, tested in the offline Windows clone that this job owns; then the draft says `tested`.
# The executable is not built here. The test suite is, with the tools that built it:
# llvm-mingw (.ci/llvm-mingw.nix) and the Rust toolchain of the release job, whose
# gnullvm standard library this job provisions explicitly.
# `ccid release leg windows` runs this as the leg's one check, with the draft of the commit at hand.
set -euo pipefail
exec 2>&1
: "${CCID_BIN:?Missing verified ccid job runtime}"
: "${CI_COMMIT_SHA:?Missing verified source revision}"
: "${CCID_RELEASE_DIR:?Missing the release directory}"
# Tools: llvm-mingw, the Rust toolchain with the gnullvm standard library (.ci/windows-env.sh).
# shellcheck source=windows-env.sh
source "$PWD/.ci/windows-env.sh"
PATH="$(dirname "$CCID_BIN"):$PATH"
export PATH
export CCID_SOURCE_REVISION="$CI_COMMIT_SHA"
rustc -vV
cargo --version
x86_64-w64-mingw32-clang --version | head -1
export CCVL_RELEASE_DIR="$CCID_RELEASE_DIR"
exec bash .agent/scripts/release-ci.sh windows
