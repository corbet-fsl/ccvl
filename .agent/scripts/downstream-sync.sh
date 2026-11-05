#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run only on a trusted internal worker with an exact staged public source tree.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
source_commit="${1:?Exact public source commit required}"
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || exit 2
: "${SOURCE_ARCHIVE:?Verified source archive required}"
: "${SOURCE_SHA256:?Verified source digest required}"
: "${UPSTREAM_SOURCE_BUNDLE:?Exact upstream Git bundle required}"
: "${UPSTREAM_SOURCE_SHA256:?Upstream Git bundle digest required}"
: "${DOWNSTREAM_PUSH_KEY:?Existing downstream push identity required}"
: "${DOWNSTREAM_SSH_URL:?Existing downstream SSH URL required}"
[[ "$SOURCE_SHA256" =~ ^[0-9a-f]{64}$ ]] || exit 2
[[ "$UPSTREAM_SOURCE_SHA256" =~ ^[0-9a-f]{64}$ ]] || exit 2
printf '%s  %s\n' "$SOURCE_SHA256" "$SOURCE_ARCHIVE" | sha256sum --check --strict
test "$(git get-tar-commit-id < "$SOURCE_ARCHIVE")" = "$source_commit"
printf '%s  %s\n' "$UPSTREAM_SOURCE_SHA256" "$UPSTREAM_SOURCE_BUNDLE" | sha256sum --check --strict
test "$(git bundle list-heads "$UPSTREAM_SOURCE_BUNDLE")" = "$source_commit refs/heads/source"

cache_root="${CI_CACHE_ROOT:-${CARGO_HOME:-}}"
: "${cache_root:?Persistent Cargo home or CI cache root required}"
# A separate namespace prevents interference with normal product checks. Use
# ccid's actual lock as well, including when a caller points ccid at this target.
export CARGO_TARGET_DIR="$cache_root/targets/ccvl-downstream"
[[ "$CARGO_TARGET_DIR" = /* ]] || { echo 'Cargo target must be absolute.' >&2; exit 2; }
mkdir -p "$CARGO_TARGET_DIR/.ccid"
CARGO_TARGET_DIR="$(cd "$CARGO_TARGET_DIR" && pwd -P)"
export CARGO_TARGET_DIR
command -v flock >/dev/null
exec 9>"$CARGO_TARGET_DIR/.ccid/lock"
flock --wait 60 9

export CARGO_BUILD_JOBS="${CI_JOBS:-${CARGO_BUILD_JOBS:-2}}"
# shellcheck source=.agent/scripts/memory-admission.sh
source .agent/scripts/memory-admission.sh
CARGO_BUILD_JOBS="$(ccvl_memory_admission)"
export CARGO_BUILD_JOBS

umask 077
scratch="$(mktemp -d)"
key_path="$scratch/id_ed25519"
cleanup() {
  if [[ -f "$key_path" ]]; then shred -u -- "$key_path"; fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT
mkdir "$scratch/upstream"
git -C "$scratch/upstream" init --bare --template= --quiet
# Empty-repository verification refuses a bundle with missing prerequisites.
git -C "$scratch/upstream" bundle verify "$UPSTREAM_SOURCE_BUNDLE"

# Build the gate from the separately verified public archive, before reading any
# private source. Retain a job-owned copy while the persistent build lock is held.
# shellcheck source=.agent/scripts/rust-toolchain.sh
source .agent/scripts/rust-toolchain.sh
ccvl_select_rust_toolchain
"${CCVL_CARGO_COMMAND[@]}" build --locked
gate="$scratch/ccvl"
cp "$CARGO_TARGET_DIR/debug/ccvl" "$gate"

install -m 600 /dev/null "$key_path"
printf '%s\n' "$DOWNSTREAM_PUSH_KEY" > "$key_path"
unset DOWNSTREAM_PUSH_KEY
export GIT_TERMINAL_PROMPT=0
export GIT_SSH_COMMAND="ssh -i $key_path -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=$scratch/known_hosts"
git clone --quiet --single-branch --branch main "$DOWNSTREAM_SSH_URL" "$scratch/downstream"
cd "$scratch/downstream"
git remote add upstream "${CCVL_UPSTREAM_URL:?Public ccvl repository URL (https://<forge>/<owner>/ccvl.git) required}"
git fetch --quiet --no-tags "$UPSTREAM_SOURCE_BUNDLE" refs/heads/source
test "$(git rev-parse FETCH_HEAD)" = "$source_commit"
git update-ref refs/remotes/upstream/main "$source_commit"

if git merge-base --is-ancestor "$source_commit" HEAD; then
  printf 'Private downstream already contains %s.\n' "$source_commit"
  exit 0
fi

git config user.name "ccvl downstream sync"
git config user.email "ccvl-downstream-sync@noreply.invalid"
git merge --no-edit refs/remotes/upstream/main
"$gate" downstream-check --policy ccvl-downstream.json --upstream-ref refs/remotes/upstream/main
"$gate" check

# Re-read the authoritative public branch immediately before publication. A
# changed branch or unavailable forge authentication fails without a push.
upstream_head="$(git ls-remote --exit-code upstream refs/heads/main | awk '{print $1}')"
if [[ "$upstream_head" != "$source_commit" ]]; then
  echo 'Public main changed during verification; refusing a stale downstream push.' >&2
  exit 2
fi
git push --porcelain origin HEAD:main
expected_head="$(git rev-parse HEAD)"
remote_head="$(git ls-remote --exit-code origin refs/heads/main | awk '{print $1}')"
test "$remote_head" = "$expected_head"
