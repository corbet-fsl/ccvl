#!/usr/bin/env bash
# Cross-build one release platform with cargo-zigbuild and package it with its
# receipt. The result is built, never run here: ARM64 has no native executor and is
# never emulated, and Windows x86_64 waits for its test executor.
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
case "$platform" in
  linux-aarch64) triple=aarch64-unknown-linux-gnu; zig_target="$triple.2.39" ;;
  windows-x86_64) triple=x86_64-pc-windows-gnu; zig_target="$triple"; exe=ccvl.exe ;;
  windows-arm64) triple=aarch64-pc-windows-gnullvm; zig_target="$triple"; exe=ccvl.exe ;;
  macos-*) echo "$platform waits for the macOS build clone and stays pending." >&2; exit 2 ;;
  linux-x86_64) echo 'Linux x86_64 is built and run natively: native-release.sh.' >&2; exit 2 ;;
  *) echo "Unknown cross platform: $platform" >&2; exit 2 ;;
esac
for tool in cargo-zigbuild zig; do
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
"${CCVL_CARGO_COMMAND[@]}" zigbuild --locked --release \
  --config .cargo/release-profile.toml --target "$zig_target"
binary="${CARGO_TARGET_DIR:-$repo_root/target}/$triple/release/$exe"
# The tested native executable of this run serves the evidence command when it
# exists; otherwise the command runs from the checkout.
if [[ -x "$output/ccvl-linux-x86_64" ]]; then export CCVL_RELEASE_EVIDENCE_BIN="$output/ccvl-linux-x86_64"; fi
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
ccvl_prepare_release_evidence
tool="cargo-zigbuild $(cargo-zigbuild --version | awk '{print $NF}'), zig $(zig version)"
"${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" cross \
  --platform "$platform" --binary "$binary" --host "$triple" \
  --rustc "$("${CCVL_RUSTC_COMMAND[@]}" --version)" \
  --cargo "$("${CCVL_CARGO_COMMAND[@]}" --version)" \
  --tool "$tool" --output "$output"
