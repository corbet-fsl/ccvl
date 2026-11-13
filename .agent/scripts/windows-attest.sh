#!/usr/bin/env bash
# Make the Windows test count: when the shipped Windows x86_64 executable and the test
# suite passed in the offline Windows clone (windows-test.sh), the cross-build receipt in
# the drafts of the release becomes a NATIVE receipt, and the manifests say `tested`.
#
#   windows-attest.sh REPORT.json      REPORT.json is what windows-test.sh wrote
#
# The release job left ONE draft for this commit, build-<sha>, holding every asset
# (executables, bundles, receipts, gates, manifests). This stage reads it (`ccid release fetch`), lets
# `ccvl release-evidence attest` re-issue the receipt (it checks the report against the
# receipt: the very same executable, the same source, every check and the whole suite
# passed), lets `release-evidence check` write the manifests exactly as the promote stage
# will write them again, and replaces the changed files in the draft (`ccid release put`):
#   ccvl-windows-x86_64.receipt.json      the native receipt
#   ccvl-windows-x86_64.native-result.json the clone's report, as recorded by the receipt
#   ccvl-windows-x86_64.cross-receipt.json the receipt it replaces (its checksum is in the new one)
#   manifest.json, runtime-manifest.json
# Nothing else is touched and nothing is published: a published release is final, so this
# refuses it (promotion renames the draft to v<version>, and states the platforms again). Run twice, it changes nothing the second time.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
report="${1:?Usage: windows-attest.sh REPORT.json}"
[[ -s $report ]] || { echo "No report: $report" >&2; exit 2; }
platform=windows-x86_64
: "${CCID_BIN:?Missing the ccid that runs this leg}"
: "${CI_COMMIT_SHA:?Missing the source commit}"
sha="$CI_COMMIT_SHA"
[[ -z ${GH_TOKEN:-} ]] || { echo 'Publication credentials must not reach the Windows attestation.' >&2; exit 2; }
for tool in jq sha256sum cmp; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 2; }
done
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-windows-attest.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT
# `ccid release leg windows` ran this only for a draft of the commit that is main now.

dist="$scratch/dist"
mkdir -p "$dist"
# Every asset of the draft (the runner's read credential reads a draft; the job token cannot).
"$CCID_BIN" release fetch --into "$dist"
receipt_name="ccvl-$platform.receipt.json"
cross_name="ccvl-$platform.cross-receipt.json"
# A run that died between removing the old receipt and uploading the new one: the copy
# of the old one is on the release.
if [[ ! -e $dist/$receipt_name && -e $dist/$cross_name ]]; then
  echo "The release has no $receipt_name; starting from $cross_name."
  cp -- "$dist/$cross_name" "$dist/$receipt_name"
fi
runtime_id="$(jq -er '.runtime_id' "$dist/$receipt_name")"
[[ "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || { echo 'The receipt carries no runtime identity.' >&2; exit 2; }

# The evidence command is the tested native executable of this very release, as the cross
# stage uses it: it must run here and carry the identity of this source. (An evidence
# binary named in the environment is used as it is: release-evidence.sh documents that.)
if [[ -z ${CCVL_RELEASE_EVIDENCE_BIN:-} ]]; then
  linux_exe="$dist/ccvl-linux-x86_64"
  (cd "$dist" && sha256sum --check --strict ccvl-linux-x86_64.sha256 >/dev/null)
  chmod 0755 "$linux_exe"
  [[ $(cd "$repo_root" && "$linux_exe" runtime-id) == "$runtime_id" ]] || {
    echo 'The native Linux executable of the release does not carry the runtime identity of this source.' >&2; exit 2
  }
  export CCVL_RELEASE_EVIDENCE_BIN="$linux_exe"
fi
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
ccvl_prepare_release_evidence

kind="$(jq -r '.kind' "$dist/$receipt_name")"
if [[ $kind == cross ]]; then
  cp -- "$dist/$receipt_name" "$dist/$cross_name"
  "${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" attest --platform "$platform" --result "$report" --output "$dist"
else
  echo "The receipt of $platform is already native (run $(jq -r '.run' "$dist/$receipt_name")); verifying it."
fi
# The manifests are derived from the receipts: write them again, as `check` does at promotion.
find "$dist/manifest.json" "$dist/runtime-manifest.json" -delete
"${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" check --output "$dist" >/dev/null
for file in manifest.json runtime-manifest.json; do
  [[ $(jq -r --arg p "$platform" '.platform_states[$p]' "$dist/$file") == tested ]] || {
    echo "$file does not say that $platform is tested." >&2; exit 2
  }
done

# put NAME: the draft ends up holding $dist/NAME under NAME, replacing what differs (a draft only).
put() { "$CCID_BIN" release put "$dist/$1"; }
echo "Draft of $sha:"
# First the new names, then the receipt (the old one is kept as $cross_name), then the
# manifests that describe them.
[[ ! -e $dist/$cross_name ]] || put "$cross_name"
put "ccvl-$platform.native-result.json"
put "$receipt_name"
put manifest.json
put runtime-manifest.json

# What the forge holds now is what promotion will check: read it back.
for file in manifest.json runtime-manifest.json "$receipt_name"; do
  "$CCID_BIN" release fetch "$file" --into "$scratch/read-back"
  cmp -s -- "$scratch/read-back/$file" "$dist/$file" || { echo "$file on the forge differs from the checked one." >&2; exit 2; }
done
for file in manifest.json runtime-manifest.json; do
  jq -e --arg p "$platform" '.platform_states[$p] == "tested"' "$dist/$file" >/dev/null
done
printf 'Platform states in the draft: %s\n' "$(jq -c '.platform_states' "$dist/manifest.json")"
printf '%s is tested: the shipped executable and the whole test suite passed in the offline Windows clone (run %s).\n' \
  "$platform" "$(jq -r '.run' "$dist/$receipt_name")"
