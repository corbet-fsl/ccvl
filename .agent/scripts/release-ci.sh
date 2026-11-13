#!/usr/bin/env bash
# Release stages on a Forgejo runner; no installation, no scheduler of its own.
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
action="${1:?Usage: release-ci.sh rust|lint|native|cross|linux-deep|archive|check|windows}"
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
    # The version and its preflight (a version that names no release, or one released already,
    # stops the run before anything is built) are `ccid release build`'s, before this stage runs.
    bash .agent/scripts/ci-check.sh "$action"
    release_evidence gate "$action" --output "$CCVL_RELEASE_DIR"
    ;;
  native)
    # Built portably and run on this host: the platform that has a test executor.
    bash .agent/scripts/native-release.sh "${CCVL_RELEASE_PLATFORM:-linux-x86_64}" "$CCVL_RELEASE_DIR"
    ;;
  cross)
    # Built here for the platforms nothing here can run: ARM64 (never run anywhere
    # yet), and x86_64 Windows until its test executor exists. macOS waits for
    # the macOS build clone and stays pending. Which of them this release holds
    # follows from its version (`ccvl release-evidence platforms`): a patch release
    # holds Linux x86_64 alone and builds nothing here.
    mapfile -t wanted < <(release_evidence platforms)
    built=0
    for platform in linux-aarch64 windows-x86_64 windows-arm64; do
      if printf '%s\n' "${wanted[@]}" | grep -Fxq -- "$platform"; then
        bash .agent/scripts/cross-release.sh "$platform" "$CCVL_RELEASE_DIR"
        built=$((built + 1))
      fi
    done
    printf 'Cross stage: %s platform(s) built; this release holds: %s.\n' "$built" "${wanted[*]}"
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
  windows)
    # The Windows leg, after the release job has left the draft: the Windows x86_64
    # executable of this commit, as shipped, and the test suite built for the same ABI,
    # tested in the offline Windows clone; then the draft says `tested` for the platform.
    report="$CCVL_RELEASE_DIR/windows-x86_64.clone-report.json"
    bash .agent/scripts/windows-test.sh "$report"
    # A release that does not hold the platform (a patch release) leaves no report.
    if [[ -s $report ]]; then bash .agent/scripts/windows-attest.sh "$report"; fi
    ;;
  *) echo "Unknown release stage: $action" >&2; exit 2 ;;
esac
