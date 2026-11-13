#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run the Windows stage of one exact commit on a Forgejo runner of the label `windows`:
# the executable the release run left in its draft, and the test suite built for the same
# ABI, tested in the offline Windows clone that this job owns; then the drafts say `tested`.
# The executable is not built here. The test suite is, with the tools that built it:
# llvm-mingw (.ci/llvm-mingw.nix) and the Rust toolchain of the release job, whose
# gnullvm standard library this job provisions explicitly.
set -euo pipefail
exec 2>&1
: "${CCID_BIN:?Missing verified ccid job runtime}"
: "${CI_COMMIT_SHA:?Missing verified source revision}"
: "${CHECKS:?Missing declared check selection}"
case "$CHECKS" in
  release-windows) ;;
  *) echo "CHECKS is not the declared Windows test: $CHECKS" >&2; exit 2 ;;
esac
# Tools: llvm-mingw, the Rust toolchain with the gnullvm standard library (.ci/windows-env.sh).
# shellcheck source=windows-env.sh
source "$PWD/.ci/windows-env.sh"
PATH="$(dirname "$CCID_BIN"):$PATH"
export PATH
export CCID_SOURCE_REVISION="$CI_COMMIT_SHA"
"$CCID_BIN" render --repo "$PWD" --check
rustc -vV
cargo --version
x86_64-w64-mingw32-clang --version | head -1
exec "$CCID_BIN" check --repo "$PWD" --check "$CHECKS"
