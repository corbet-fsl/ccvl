#!/usr/bin/env bash
# The archive gate: run the real downloaded bundle where Git and Rust do not exist.
#
# Which minimal executor runs the check:
#   ARCHIVE_EXECUTOR + ARCHIVE_EXECUTOR_SHA256   an executor the runner provides (copied, verified, run)
#   CCVL_ARCHIVE_EXECUTOR=direct                 the worker already is a minimal executor
#   otherwise                                    archive-executor.sh (an unprivileged minimal root)
# The check script is always the verified one of this checkout.
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
dist="${1:?Missing native package directory}"
check="$repo_root/.agent/scripts/check-release-archive.sh"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-archive-gate.XXXXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
script="$scratch/check-release-archive.sh"
cp -- "$check" "$script"
if test -n "${ARCHIVE_EXECUTOR:-}${ARCHIVE_EXECUTOR_SHA256:-}"; then
  test -n "${ARCHIVE_EXECUTOR:-}"
  [[ "${ARCHIVE_EXECUTOR_SHA256:-}" =~ ^[0-9a-f]{64}$ ]]
  test -f "$ARCHIVE_EXECUTOR"
  test ! -L "$ARCHIVE_EXECUTOR"
  cp -- "$ARCHIVE_EXECUTOR" "$scratch/executor"
  printf '%s  %s\n' "$ARCHIVE_EXECUTOR_SHA256" "$scratch/executor" | sha256sum --check --strict
  chmod 0700 "$scratch/executor"
  "$scratch/executor" "$script" "$dist"
elif [[ ${CCVL_ARCHIVE_EXECUTOR:-} == direct ]]; then
  bash "$script" "$dist"
else
  bash "$repo_root/.agent/scripts/archive-executor.sh" "$script" "$dist"
fi
