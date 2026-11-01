#!/usr/bin/env bash
# Locate the maintainer `release-evidence` command of the ccvl binary.
# Source this file, run ccvl_prepare_release_evidence once in the calling shell,
# then run "${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" <subcommand> ... anywhere.
#
# CCVL_RELEASE_EVIDENCE_BIN names an already built ccvl binary (the native
# build uses the one it just compiled). Without it the command runs from this
# checkout with the existing Rust toolchain; nothing is installed.

ccvl_prepare_release_evidence() {
  local repo_root
  repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
  if [[ -n ${CCVL_RELEASE_EVIDENCE_BIN:-} ]]; then
    CCVL_RELEASE_EVIDENCE_COMMAND=("$CCVL_RELEASE_EVIDENCE_BIN" release-evidence)
    return 0
  fi
  if [[ -z ${CCVL_CARGO_COMMAND+set} ]]; then
    # shellcheck source=.agent/scripts/rust-toolchain.sh
    source "$repo_root/.agent/scripts/rust-toolchain.sh"
    # The selection report must not mix into captured evidence output.
    ccvl_select_rust_toolchain >&2
  fi
  export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
  # shellcheck disable=SC2034 # Read by the sourcing script.
  CCVL_RELEASE_EVIDENCE_COMMAND=("${CCVL_CARGO_COMMAND[@]}" run --locked --quiet --bin ccvl -- release-evidence)
}
