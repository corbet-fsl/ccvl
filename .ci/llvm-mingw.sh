#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Print the store path of the llvm-mingw toolchain (clang, lld, the mingw-w64 UCRT
# runtime) that links both Windows targets of ccvl, the *-pc-windows-gnullvm pair.
# Used by the release run and by any job that builds for Windows, so there is one
# definition: the flake package .#llvm-mingw, whose source is the flake input
# llvm-mingw-dist (flake.lock). `nix flake archive` pushes that input to the srv
# Celler ci cache on every CI run, so a GitHub outage never blocks the build.
set -euo pipefail
here="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
path="$(nix build --no-link --print-out-paths "$here#llvm-mingw")"
test -x "$path/bin/x86_64-w64-mingw32-clang" -a -x "$path/bin/aarch64-w64-mingw32-clang"
printf '%s\n' "$path"
