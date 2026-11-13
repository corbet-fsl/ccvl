#!/usr/bin/env bash
# The seal check of the release (`release-seal` of [release]): promotion has downloaded the complete
# draft into $CCID_RELEASE_DIR; this derives the two manifests from the receipts of every platform
# afresh (the legs that ran after the draft was made are in them now) and refuses a release whose
# legs have not left their final evidence (`check --complete`). ccid then replaces the manifests in
# the draft where they differ, writes SHA256SUMS and publishes. Nothing else may change here: ccid
# refuses a seal that touches any other file.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
dir="${CCID_RELEASE_DIR:?Missing the release directory}"
[[ -z ${GH_TOKEN:-} ]] || { echo 'Publication credentials must not reach the seal.' >&2; exit 2; }
# The evidence command is the tested native Linux executable of this very release: it must run here
# and carry the identity of this source (the same rule the Windows attestation applies).
runtime_id="$(jq -er '.runtime_id' "$dir/ccvl-linux-x86_64.receipt.json")"
[[ "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || { echo 'The receipt carries no runtime identity.' >&2; exit 2; }
(cd "$dir" && sha256sum --check --strict ccvl-linux-x86_64.sha256 >/dev/null)
chmod 0755 "$dir/ccvl-linux-x86_64"
[[ $("$dir/ccvl-linux-x86_64" runtime-id) == "$runtime_id" ]] || {
  echo 'The native Linux executable of the release does not carry the runtime identity of this source.' >&2; exit 2
}
export CCVL_RELEASE_EVIDENCE_BIN="$dir/ccvl-linux-x86_64"
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
ccvl_prepare_release_evidence
# The checksum file and the manifests of the draft are derived again, never trusted as they are.
find "$dir/manifest.json" "$dir/runtime-manifest.json" "$dir/SHA256SUMS" -delete 2>/dev/null || true
"${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" check --complete --output "$dir" >/dev/null
printf 'The manifests are derived from the receipts of every platform; the legs are complete.\n'
