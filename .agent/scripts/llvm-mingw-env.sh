#!/usr/bin/env bash
# Sourced, not run: names the llvm-mingw tools of one Windows gnullvm target for cargo and
# for the cc-rs build scripts of its dependencies. The release cross build
# (cross-release.sh) and the Windows test suite (windows-test.sh) both use it, so the
# executable that ships and the tests that vouch for it are linked and compiled the same way.
#
#   ccvl_llvm_mingw_env x86_64-pc-windows-gnullvm
#
# The tools come from the llvm-mingw package of .ci/llvm-mingw.nix and must be on PATH
# (last, so their unprefixed clang and lld never shadow the host toolchain).
#
# Every tool is named per target. A runner that exports a generic CC (a host cc behind a
# compile cache) would otherwise be used for the Windows target too, and cc-rs only looks
# for the mingw clang when no CC is set.

# ccvl_llvm_mingw_prefix TRIPLE: the tool prefix of the target, printed.
ccvl_llvm_mingw_prefix() {
  case "$1" in
    x86_64-pc-windows-gnullvm) printf 'x86_64-w64-mingw32\n' ;;
    aarch64-pc-windows-gnullvm) printf 'aarch64-w64-mingw32\n' ;;
    *) echo "Not a gnullvm Windows target: $1" >&2; return 2 ;;
  esac
}

# ccvl_llvm_mingw_env TRIPLE: export the linker and the C, C++ and archiver tools of TRIPLE.
ccvl_llvm_mingw_env() {
  local triple="$1" prefix tool underscored
  prefix="$(ccvl_llvm_mingw_prefix "$triple")" || return 2
  for tool in "$prefix-clang" "$prefix-clang++" "$prefix-ar" llvm-objdump; do
    command -v "$tool" >/dev/null || { echo "Missing cross toolchain: $tool" >&2; return 2; }
  done
  underscored="${triple//-/_}"
  export "CARGO_TARGET_${underscored^^}_LINKER=$(command -v "$prefix-clang")"
  export "CC_$underscored=$(command -v "$prefix-clang")"
  export "CXX_$underscored=$(command -v "$prefix-clang++")"
  export "AR_$underscored=$(command -v "$prefix-ar")"
}

# ccvl_llvm_mingw_identity TRIPLE: "llvm-mingw RELEASE, clang VERSION" of the package that
# serves TRIPLE, printed (the tool identity of a receipt).
ccvl_llvm_mingw_identity() {
  local triple="$1" prefix clang clang_version package mingw_release
  prefix="$(ccvl_llvm_mingw_prefix "$triple")" || return 2
  clang="$prefix-clang"
  clang_version="$("$clang" --version | sed -n '1s/.*clang version \([0-9][0-9.]*\).*/\1/p')"
  package="$(basename "$(dirname "$(dirname "$(readlink -f "$(command -v "$clang")")")")")"
  if [[ $package == *llvm-mingw-* ]]; then mingw_release="${package##*llvm-mingw-}"; else mingw_release=unknown; fi
  printf 'llvm-mingw %s, clang %s\n' "$mingw_release" "${clang_version:-unknown}"
}
