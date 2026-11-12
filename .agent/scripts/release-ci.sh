#!/usr/bin/env bash
# Release stages on a Forgejo runner; no installation, no scheduler of its own.
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
action="${1:?Usage: release-ci.sh rust|lint|native|cross|linux-deep|archive|check|publish|promote}"
if [[ ${FORGEJO_ACTIONS:-} == true ]]; then
  [[ ${GITHUB_REPOSITORY:-} == corbet-fsl/ccvl && ${GITHUB_REF_NAME:-} == main ]] || {
    echo 'Forgejo release stages require public ccvl main.' >&2; exit 2;
  }
  [[ -n ${CARGO_TARGET_DIR:-} && ${CCID_TARGET_LOCK_HELD:-} == "$CARGO_TARGET_DIR" ]] || {
    echo 'Forgejo release stages require the verified ccid target lock.' >&2; exit 2;
  }
fi
# The job's own token is the forge's per-job credential and is always present;
# any other publication credential has no business here.
if [[ -n ${GH_TOKEN:-} ]]; then
  echo 'Publication credentials are not accepted by the release stages.' >&2; exit 2
fi
# The runner-level downstream credential is read by the promote stage only: no
# build, test or repository script runs with its location in the environment.
if [[ "$action" != promote ]]; then
  unset CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE
fi
if [[ -z ${CCVL_RELEASE_DIR:-} ]]; then
  : "${CARGO_TARGET_DIR:?Set CCVL_RELEASE_DIR or the locked Cargo target}"
  : "${CI_COMMIT_SHA:?Missing source commit}"
  run="${GITHUB_RUN_ID:-}"
  : "${run:?Missing Forgejo run number}"
  CCVL_RELEASE_DIR="$CARGO_TARGET_DIR/ccvl-release/$CI_COMMIT_SHA/$run"
fi
mkdir -p "$CCVL_RELEASE_DIR"
CCVL_RELEASE_DIR="$(cd "$CCVL_RELEASE_DIR" && pwd)"
export CCVL_RELEASE_DIR
release_evidence() {
  ccvl_prepare_release_evidence
  "${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" "$@"
}
printf 'Release evidence directory: %s\n' "$CCVL_RELEASE_DIR"
case "$action" in
  rust|lint)
    bash .agent/scripts/ci-check.sh "$action"
    release_evidence gate "$action" --output "$CCVL_RELEASE_DIR"
    ;;
  native)
    # Built portably and run on this host: the platform that has a test executor.
    bash .agent/scripts/native-release.sh "${CCVL_RELEASE_PLATFORM:-linux-x86_64}" "$CCVL_RELEASE_DIR"
    ;;
  cross)
    # Built here for platforms nothing here can run: ARM64 (never run anywhere
    # yet), and x86_64 Windows until its test executor exists. macOS waits for
    # the macOS build clone and stays pending.
    for platform in linux-aarch64 windows-x86_64 windows-arm64; do
      bash .agent/scripts/cross-release.sh "$platform" "$CCVL_RELEASE_DIR"
    done
    ;;
  linux-deep)
    [[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || { echo 'Linux x86_64 verification worker required.' >&2; exit 2; }
    release_evidence source >/dev/null
    (cd "$CCVL_RELEASE_DIR" && sha256sum --check --strict ccvl-linux-x86_64.sha256)
    mkdir -p .agent/cache/ccvl/bin
    cp "$CCVL_RELEASE_DIR/ccvl-linux-x86_64" .agent/cache/ccvl/bin/ccvl
    chmod 0755 .agent/cache/ccvl/bin/ccvl
    bash .agent/scripts/check-linux-deep.sh
    release_evidence gate linux-deep --output "$CCVL_RELEASE_DIR"
    ;;
  archive)
    # The real downloaded bundle must run where Git and Rust do not exist.
    bash .agent/scripts/archive-gate.sh "$CCVL_RELEASE_DIR"
    ;;
  check)
    release_evidence check --output "$CCVL_RELEASE_DIR"
    ;;
  publish)
    # Complete DRAFT releases on the forge; publishing them is the promote stage.
    bash .agent/scripts/publish-release.sh "$CCVL_RELEASE_DIR"
    ;;
  promote)
    # Make the verified drafts of this exact commit public, then copy downstream.
    bash .agent/scripts/publish-release.sh --promote "$CCVL_RELEASE_DIR"
    ;;
  *) echo "Unknown release stage: $action" >&2; exit 2 ;;
esac
