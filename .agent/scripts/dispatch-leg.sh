#!/usr/bin/env bash
# Start the next platform leg of the release of this commit, if one is due.
#
# A minor release holds platforms that only their own executors can run, so it is not
# complete until the legs have run: the Windows clone tests the shipped executable
# (`release-windows`), the macOS clone builds, tests and packages the two macOS platforms
# (`release-macos`). Nobody starts a leg by hand and nothing waits for one: the release run
# ends by calling this script and every leg ends by calling it again, so the next leg
# starts when the last one has finished (a chain of events; no polling, no input, no knob).
# A patch release holds Linux x86_64 alone and has no legs.
#
#   dispatch-leg.sh
#
# The script reads the DRAFT of the commit (`build-<commit>`): its manifest says which
# release it is (`release_kind`) and which platforms it holds, and its assets say which legs
# have left their evidence. It starts the first leg whose evidence the draft lacks:
#   release-windows   until the draft holds ccvl-windows-x86_64.native-result.json
#   release-macos     until it holds the receipts of macos-x86_64 and macos-arm64
# With nothing due it says so, so a leg re-run for a repair ends the chain at once. It only
# dispatches: the evidence a leg leaves is what `release-promote` checks, so a leg that
# failed is a draft that can not be promoted, not a state anybody has to track.
#
# Bash 3.2, the BSD userland, curl and jq are enough: the macOS leg calls this script too,
# and it needs no ccvl binary and no toolchain.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
[[ -z ${GH_TOKEN:-} ]] || { echo 'Publication credentials must not reach the release legs.' >&2; exit 2; }
for tool in jq curl; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 2; }
done
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-legs.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT
# shellcheck source=.agent/scripts/forge-draft.sh
source .agent/scripts/forge-draft.sh
forge_setup

# The legs belong to the draft of the commit that is main now.
forge_call GET "$api/repos/$repository/branches/main"
[[ $(jq -r '.commit.id' "$scratch/response") == "$sha" ]] || {
  printf 'Source %s is no longer main; no leg is started.\n' "$sha" >&2; exit 2
}
status=0
forge_release "build-$sha" "$scratch/draft.json" || status=$?
[[ $status == 0 ]] || { echo "There is no draft build-$sha: the release run has not left one." >&2; exit 2; }
[[ $(jq -r '.draft' "$scratch/draft.json") == true ]] || {
  printf 'build-%s is public: nothing is left to run.\n' "${sha:0:12}"; exit 0
}
forge_fetch_asset "$scratch/draft.json" manifest.json "$scratch/manifest.json" || {
  echo "The draft build-$sha has no manifest.json to read the release from." >&2; exit 2
}
version="$(jq -er '.version' "$scratch/manifest.json")"
kind="$(jq -er '.release_kind' "$scratch/manifest.json")"
if [[ $kind != minor ]]; then
  printf 'ccvl %s is a %s release: it holds Linux x86_64 alone and has no platform legs.\n' "$version" "$kind"
  exit 0
fi

# held NAME: the draft holds that asset.
held() { jq -e --arg name "$1" '.assets | any(.name == $name)' "$scratch/draft.json" >/dev/null; }
# wants PLATFORM: this release holds that platform.
wants() { jq -e --arg platform "$1" '.platforms | any(. == $platform)' "$scratch/manifest.json" >/dev/null; }

next=
if wants windows-x86_64 && ! held ccvl-windows-x86_64.native-result.json; then
  next='release-windows'
elif wants macos-x86_64 && ! { held ccvl-macos-x86_64.receipt.json && held ccvl-macos-arm64.receipt.json; }; then
  next='release-macos'
fi
if [[ -z $next ]]; then
  printf 'Every platform leg of ccvl %s has left its evidence in build-%s: release-promote can check the draft.\n' \
    "$version" "${sha:0:12}"
  exit 0
fi
forge_call POST "$api/repos/$repository/actions/workflows/$next.yml/dispatches" \
  -H 'Content-Type: application/json' --data '{"ref":"main"}' || {
  echo "The forge did not start the workflow $next." >&2; exit 2
}
printf 'Started %s for %s (ccvl %s, a %s release). It starts the leg after it.\n' "$next" "$sha" "$version" "$kind"
