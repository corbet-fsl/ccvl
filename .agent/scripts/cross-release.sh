#!/usr/bin/env bash
# Cross-build one release platform and package it with its receipt. Linux aarch64 is
# linked by cargo-zigbuild; both Windows platforms are built for the one gnullvm ABI
# with clang and lld and the mingw-w64 UCRT runtime of llvm-mingw (plain cargo build,
# no Microsoft SDK, free licences). The result is built, never run here: ARM64 has no
# native executor and is never emulated, and the Windows x86_64 executable is run
# later, as this very file, in its Windows clone (.agent/scripts/windows-test.sh).
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
platform="${1:?Usage: cross-release.sh PLATFORM OUTPUT}"
output="${2:?Missing package output directory}"
[[ -z ${GH_TOKEN:-} ]] || { echo 'Publication credentials must not reach the cross build.' >&2; exit 2; }
unset CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE
if [[ ${FORGEJO_ACTIONS:-} == true ]]; then
  [[ -n ${CARGO_TARGET_DIR:-} && ${CCID_TARGET_LOCK_HELD:-} == "$CARGO_TARGET_DIR" ]] || {
    echo 'Forgejo cross builds require the verified ccid target lock.' >&2; exit 2;
  }
fi
exe=ccvl
driver=zigbuild
case "$platform" in
  linux-aarch64) triple=aarch64-unknown-linux-gnu; zig_target="$triple.2.39" ;;
  windows-x86_64) triple=x86_64-pc-windows-gnullvm; driver=llvm-mingw; mingw_cc=x86_64-w64-mingw32-clang; exe=ccvl.exe ;;
  windows-arm64) triple=aarch64-pc-windows-gnullvm; driver=llvm-mingw; mingw_cc=aarch64-w64-mingw32-clang; exe=ccvl.exe ;;
  macos-*) echo "$platform waits for the macOS build clone and stays pending." >&2; exit 2 ;;
  linux-x86_64) echo 'Linux x86_64 is built and run natively: native-release.sh.' >&2; exit 2 ;;
  *) echo "Unknown cross platform: $platform" >&2; exit 2 ;;
esac
if [[ $driver == zigbuild ]]; then
  needed=(cargo-zigbuild zig)
else
  needed=("$mingw_cc" "${mingw_cc%-clang}-clang++" "${mingw_cc%-clang}-ar" llvm-objdump)
fi
for tool in "${needed[@]}"; do
  command -v "$tool" >/dev/null || { echo "Missing cross toolchain: $tool" >&2; exit 2; }
done
# shellcheck source=.agent/scripts/rust-toolchain.sh
source .agent/scripts/rust-toolchain.sh
ccvl_select_rust_toolchain
# The target's standard library belongs to the selected toolchain. The release
# job provisions it; a toolchain that still lacks it gets it from its own rustup.
sysroot="$("${CCVL_RUSTC_COMMAND[@]}" --print sysroot)"
if [[ ! -d $sysroot/lib/rustlib/$triple/lib ]]; then
  if [[ ${CCVL_RUSTC_COMMAND[0]##*/} == rustup && ${CCVL_RUSTC_COMMAND[1]} == run ]]; then
    "${CCVL_RUSTC_COMMAND[0]}" target add "$triple" --toolchain "${CCVL_RUSTC_COMMAND[2]}"
  else
    echo "The Rust toolchain has no $triple standard library and cannot add one." >&2; exit 2
  fi
fi
if [[ $driver == zigbuild ]]; then
  "${CCVL_CARGO_COMMAND[@]}" zigbuild --locked --release \
    --config .cargo/release-profile.toml --target "$zig_target"
  tool="cargo-zigbuild $(cargo-zigbuild --version | awk '{print $NF}'), zig $(zig version)"
else
  # clang links the gnullvm targets and compiles their C and assembly (the psm crate, for
  # one). Name the tools per target: a runner that exports a generic CC (a host cc behind
  # a compile cache) would otherwise be used for the Windows target too, and cc-rs only
  # looks for the mingw clang when no CC is set.
  mingw_prefix="${mingw_cc%-clang}"
  triple_underscored="${triple//-/_}"
  export "CARGO_TARGET_${triple_underscored^^}_LINKER=$(command -v "$mingw_cc")"
  export "CC_$triple_underscored=$(command -v "$mingw_cc")"
  export "CXX_$triple_underscored=$(command -v "$mingw_prefix-clang++")"
  export "AR_$triple_underscored=$(command -v "$mingw_prefix-ar")"
  "${CCVL_CARGO_COMMAND[@]}" build --locked --release \
    --config .cargo/release-profile.toml --target "$triple"
  clang_version="$("$mingw_cc" --version | sed -n '1s/.*clang version \([0-9][0-9.]*\).*/\1/p')"
  package="$(basename "$(dirname "$(dirname "$(readlink -f "$(command -v "$mingw_cc")")")")")"
  if [[ $package == *llvm-mingw-* ]]; then mingw_release="${package##*llvm-mingw-}"; else mingw_release=unknown; fi
  tool="llvm-mingw $mingw_release, clang ${clang_version:-unknown}"
fi
binary="${CARGO_TARGET_DIR:-$repo_root/target}/$triple/release/$exe"
if [[ $driver == llvm-mingw ]]; then
  # A Windows executable of this ABI imports Windows system libraries only. A mingw
  # runtime DLL (libunwind, libc++, libwinpthread) is not on a clean Windows, and an ARM64
  # build is never run here, so this is the one place that can tell.
  imports="$(llvm-objdump -p "$binary" | sed -n 's/^[[:space:]]*DLL Name: //p')"
  [[ -n $imports ]] || { echo "No import table read from $binary." >&2; exit 2; }
  if grep -i -E '^(lib|msys|mingw)' <<<"$imports" >&2; then
    echo "$binary imports a mingw runtime DLL that a clean Windows does not have." >&2; exit 2
  fi
fi
# The tested native executable of this run serves the evidence command when it
# exists; otherwise the command runs from the checkout.
if [[ -x "$output/ccvl-linux-x86_64" ]]; then export CCVL_RELEASE_EVIDENCE_BIN="$output/ccvl-linux-x86_64"; fi
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
ccvl_prepare_release_evidence
"${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" cross \
  --platform "$platform" --binary "$binary" --host "$triple" \
  --rustc "$("${CCVL_RUSTC_COMMAND[@]}" --version)" \
  --cargo "$("${CCVL_CARGO_COMMAND[@]}" --version)" \
  --tool "$tool" --output "$output"
