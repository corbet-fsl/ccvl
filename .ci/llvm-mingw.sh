#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Print the store path of the llvm-mingw toolchain (clang, lld, the mingw-w64 UCRT
# runtime) that links both Windows targets of ccvl, the *-pc-windows-gnullvm pair.
# Used by the release run and by any job that builds for Windows, so there is one
# definition: .ci/llvm-mingw.nix, built against a pinned nixpkgs.
#
#   .ci/llvm-mingw.sh NIXPKGS_FLAKE_REF     e.g. the tool_source of .ci/run-release.sh
#
# The package is the upstream release archive, pinned by hash and only patched for the
# Nix loader; nothing is compiled. --impure only admits the local .nix file: the
# nixpkgs reference and the archive hash are both pinned.
set -euo pipefail
source_ref="${1:?Usage: llvm-mingw.sh NIXPKGS_FLAKE_REF}"
here="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
path="$(nix build --impure --no-link --print-out-paths --expr \
  "(builtins.getFlake \"$source_ref\").legacyPackages.x86_64-linux.callPackage $here/llvm-mingw.nix {}")"
test -x "$path/bin/x86_64-w64-mingw32-clang" -a -x "$path/bin/aarch64-w64-mingw32-clang"
printf '%s\n' "$path"
