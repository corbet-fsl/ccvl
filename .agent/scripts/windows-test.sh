#!/usr/bin/env bash
# Test the exact Windows x86_64 executable that ships, in the offline Windows clone.
#
# The release job cross-builds ccvl-windows-x86_64.exe and leaves it in the draft
# release of its commit. This stage fetches THAT file (it builds nothing), proves it is
# the file the receipt names, hands it to the clone with `vm-win` and runs it there: the
# identity compiled into it, then `public-check` on the source of the same commit. The
# job of the label `windows` owns the clone for its length (woken before, rolled back
# after); the clone has no network and holds nothing of Julian's.
#
#   windows-test.sh                    assets from the forge release of CI_COMMIT_SHA
#   CCVL_WINDOWS_ASSET_DIR=DIR         assets from a directory instead (a rehearsal; the
#                                      same checks, the same clone)
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
platform=windows-x86_64
exe_name="ccvl-$platform.exe"
# Must equal the Rust target of this platform in release_evidence.rs (PLATFORMS).
expected_host=x86_64-pc-windows-gnullvm
[[ -z ${GH_TOKEN:-} ]] || { echo 'Publication credentials must not reach the Windows test.' >&2; exit 2; }
unset CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE
for tool in jq curl sha256sum tar vm-win; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool (a job of the label windows has it)." >&2; exit 2; }
done
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-windows-test.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT
assets="$scratch/assets"
mkdir -p "$assets"

if [[ -n ${CCVL_WINDOWS_ASSET_DIR:-} ]]; then
  for name in "$exe_name" "$exe_name.sha256" "$exe_name.runtime-id" "ccvl-$platform.receipt.json"; do
    cp -- "$CCVL_WINDOWS_ASSET_DIR/$name" "$assets/$name"
  done
  printf 'Assets from %s (rehearsal).\n' "$CCVL_WINDOWS_ASSET_DIR"
else
  sha="${CI_COMMIT_SHA:-}"
  repository="${GITHUB_REPOSITORY:-}"
  server="${GITHUB_SERVER_URL:-}"
  server="${server%/}"
  api="${GITHUB_API_URL:-$server/api/v1}"
  token="${CCID_FORGE_TOKEN:-${FORGEJO_TOKEN:-${GITHUB_TOKEN:-}}}"
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'The exact source commit (CI_COMMIT_SHA) is required.' >&2; exit 2; }
  [[ "$repository" == corbet-fsl/ccvl && "$server" == https://* ]] || { echo 'Not the ccvl repository on the forge.' >&2; exit 2; }
  [[ "$token" =~ ^[A-Za-z0-9_.-]{8,}$ ]] || { echo 'The Forgejo job token is required.' >&2; exit 2; }
  # Tokens reach curl through configuration files, never through arguments.
  printf 'header = "Authorization: token %s"\n' "$token" > "$scratch/auth"
  chmod 0600 "$scratch/auth"
  # The job token lists releases but can not read the assets of a draft; the runner's
  # read-only credential can. It is used for reading and for nothing else.
  read_file="${CCVL_FORGE_READ_TOKEN_FILE:-${CREDENTIALS_DIRECTORY:+$CREDENTIALS_DIRECTORY/forgejo-ci-read}}"
  reads=("$scratch/auth")
  if [[ -n $read_file && -r $read_file ]]; then
    read_token="$(tr -d '\n' < "$read_file")"
    if [[ "$read_token" =~ ^[A-Za-z0-9_.-]{8,}$ ]]; then
      printf 'header = "Authorization: token %s"\n' "$read_token" > "$scratch/read-auth"
      chmod 0600 "$scratch/read-auth"
      reads=("$scratch/read-auth" "$scratch/auth")
    fi
    unset read_token
  fi
  # release_json TAG: the one release carrying the tag, in $scratch/release.json (draft or public).
  release_json() {
    local tag="$1" page code
    for page in $(seq 1 20); do
      code="$(curl -q --silent --show-error --max-time 120 -K "$scratch/auth" -o "$scratch/response" -w '%{http_code}' \
        "$api/repos/$repository/releases?limit=50&page=$page")" || code=000
      [[ $code == 200 ]] || { echo "The forge answered $code listing releases." >&2; return 2; }
      jq -e 'type == "array"' "$scratch/response" >/dev/null || return 2
      if jq -e --arg tag "$tag" 'any(.[]; .tag_name == $tag)' "$scratch/response" >/dev/null; then
        jq -e --arg tag "$tag" '[.[] | select(.tag_name == $tag)] | length == 1' "$scratch/response" >/dev/null || return 2
        jq -ce --arg tag "$tag" '.[] | select(.tag_name == $tag)' "$scratch/response" > "$scratch/release.json"
        return 0
      fi
      if jq -e 'length == 0' "$scratch/response" >/dev/null; then break; fi
    done
    echo "No release $tag on the forge." >&2
    return 4
  }
  # fetch_asset NAME: that asset of the release in release.json, into $assets.
  fetch_asset() {
    local name="$1" path auth code=000
    path="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .browser_download_url | sub("^https?://[^/]+"; "")' "$scratch/release.json")" || {
      echo "The release has no asset $name." >&2; return 2
    }
    for auth in "${reads[@]}"; do
      code="$(curl -q --silent --show-error --max-time 600 -K "$auth" -o "$assets/$name" -w '%{http_code}' "$server$path")" || code=000
      [[ $code == 200 ]] && return 0
    done
    echo "The forge answered $code reading the asset $name." >&2
    return 2
  }
  # The workspace release names the runtime release through the receipt's identity.
  release_json "build-$sha"
  fetch_asset "ccvl-$platform.receipt.json"
  runtime_id="$(jq -er '.runtime_id' "$assets/ccvl-$platform.receipt.json")"
  [[ "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || { echo 'The receipt carries no runtime identity.' >&2; exit 2; }
  release_json "runtime-$runtime_id"
  for name in "$exe_name" "$exe_name.sha256" "$exe_name.runtime-id"; do fetch_asset "$name"; done
  printf 'Assets of release build-%s and runtime-%s (draft or public).\n' "${sha:0:12}" "${runtime_id:0:12}"
fi

# The file that runs must be the file the receipt names: receipt, checksum file and bytes agree.
receipt="$assets/ccvl-$platform.receipt.json"
[[ $(jq -r '.platform' "$receipt") == "$platform" && $(jq -r '.rust_host' "$receipt") == "$expected_host" ]] || {
  echo "The receipt is not that of $platform built for $expected_host." >&2; exit 2
}
receipt_sha="$(jq -er --arg name "$exe_name" '.assets[$name]' "$receipt")"
recorded_sha="$(cut -d ' ' -f 1 "$assets/$exe_name.sha256")"
actual_sha="$(sha256sum "$assets/$exe_name" | cut -d ' ' -f 1)"
runtime_id="$(jq -er '.runtime_id' "$receipt")"
[[ "$receipt_sha" == "$recorded_sha" && "$recorded_sha" == "$actual_sha" ]] || {
  echo "The executable does not match its receipt: receipt $receipt_sha, checksum file $recorded_sha, bytes $actual_sha." >&2; exit 2
}
[[ $(tr -d '\r\n' < "$assets/$exe_name.runtime-id") == "$runtime_id" ]] || {
  echo 'The runtime-id asset differs from the receipt.' >&2; exit 2
}
printf '%s\n  sha256 %s\n  runtime %s\n  built by %s\n' "$exe_name" "$actual_sha" "$runtime_id" "$(jq -r '.tool' "$receipt")"

# What the clone gets: the executable, what to expect of it, and the source of the same
# commit (the file is run against the documents it will render; no Git is needed there).
mkdir -p "$scratch/src"
tar -C "$repo_root" --exclude=./.git --exclude=./target -cf - . | tar -C "$scratch/src" -xf -
jq -n --arg sha256 "$actual_sha" --arg runtime_id "$runtime_id" '{sha256: $sha256, runtime_id: $runtime_id}' > "$scratch/expect.json"

cat > "$scratch/run.ps1" <<'PS'
$ErrorActionPreference = 'Continue'
$expect = Get-Content -Raw -LiteralPath W:\job\expect.json | ConvertFrom-Json
$work = 'C:\ccvl-windows-test'
New-Item -ItemType Directory -Path $work -Force | Out-Null
$checks = @()
function Add-Check([string]$Name, [bool]$Ok, [string]$Detail) {
    $script:checks += [pscustomobject]@{ name = $Name; ok = $Ok; detail = $Detail }
    '{0,-5}{1}: {2}' -f $(if ($Ok) { 'ok' } else { 'FAIL' }), $Name, $Detail
}
# The executable runs from the clone's own disk: memory-mapped reads from the share fail.
$exe = Join-Path $work 'ccvl.exe'
Copy-Item -LiteralPath 'W:\job\ccvl-windows-x86_64.exe' -Destination $exe -Force
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLowerInvariant()
Add-Check 'the executable that runs is the release asset (sha256)' ($hash -ceq $expect.sha256) $hash
$arch = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture
Add-Check 'the clone is x86_64 Windows' ($arch -eq 'X64') "$arch"
$version = (& $exe --version 2>&1 | Out-String).Trim()
Add-Check 'the executable starts (--version)' ($LASTEXITCODE -eq 0) $version
$runtime = (& $exe runtime-id 2>&1 | Out-String).Trim()
Add-Check 'the identity compiled into it is the release identity (runtime-id)' (($LASTEXITCODE -eq 0) -and ($runtime -ceq $expect.runtime_id)) $runtime
# The source of the same commit, on the clone's own disk, is the working directory of the check.
robocopy W:\job\src "$work\src" /E /NFL /NDL /NJH /NJS /NP | Out-Null
Set-Location "$work\src"
$clock = [Diagnostics.Stopwatch]::StartNew()
& $exe public-check *>&1 | Out-File -FilePath W:\job\public-check.txt -Encoding utf8
$code = $LASTEXITCODE
Add-Check 'public-check on the source of the commit (every document rendered and verified)' ($code -eq 0) ('exit {0} in {1:N0} s' -f $code, $clock.Elapsed.TotalSeconds)
$checks | ConvertTo-Json -Depth 3 | Out-File -FilePath W:\job\result.json -Encoding ascii
$failed = @($checks | Where-Object { -not $_.ok }).Count
"checks failed in the clone: $failed"
exit $failed
PS

vm-win spp
vm-win put "$assets/$exe_name" "$scratch/expect.json" "$scratch/src"
rc=0
vm-win run "$scratch/run.ps1" || rc=$?
share_job="${VM_WIN_SHARE:?}/job"
if [[ -r $share_job/public-check.txt ]]; then
  printf -- '--- public-check output (tail)\n'
  sed '1s/^\xEF\xBB\xBF//' "$share_job/public-check.txt" | tail -n 12
fi
if [[ -r $share_job/result.json ]]; then
  printf -- '--- result.json\n'
  sed '1s/^\xEF\xBB\xBF//' "$share_job/result.json" | jq -r '(if type == "array" then . else [.] end)[] | "\(if .ok then "ok  " else "FAIL" end) \(.name): \(.detail)"'
fi
if ((rc == 0)); then
  printf 'The shipped %s (sha256 %s) passed in the offline Windows clone.\n' "$exe_name" "$actual_sha"
else
  printf 'The shipped %s (sha256 %s) FAILED in the offline Windows clone (exit %s).\n' "$exe_name" "$actual_sha" "$rc" >&2
fi
exit "$rc"
