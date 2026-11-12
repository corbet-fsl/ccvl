#!/usr/bin/env bash
# Publish complete, source-bound results as Forgejo releases; retries never replace asset bytes.
#
#   publish-release.sh DIR              create or complete the DRAFT releases of this commit
#   publish-release.sh --promote [DIR]  make the verified drafts public, then copy them downstream
#
# The drafts hold every built platform; promoting is a separate, explicit run.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
ccvl_prepare_release_evidence
mode=draft
if [[ ${1:-} == --promote ]]; then mode=promote; shift; fi
repository="${GITHUB_REPOSITORY:-}"
server="${GITHUB_SERVER_URL:-}"
server="${server%/}"
api="${GITHUB_API_URL:-$server/api/v1}"
token="${CCID_FORGE_TOKEN:-${FORGEJO_TOKEN:-${GITHUB_TOKEN:-}}}"
# The loopback form exists for the offline publisher fixtures only.
[[ "$repository" == corbet-fsl/ccvl ]] && [[ "$server" == https://* || "$server" == http://127.0.0.1:* ]] || exit 2
[[ "$token" =~ ^[A-Za-z0-9_.-]{8,}$ ]] || { echo 'The Forgejo job token is required.' >&2; exit 2; }
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-publication.XXXXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
# The token reaches curl through a configuration file, never through arguments.
printf 'header = "Authorization: token %s"\n' "$token" > "$scratch/auth"
chmod 0600 "$scratch/auth"
# The job token writes releases but is not allowed to read the assets of a draft back.
# The runner's read-only credential is; it is used for reading and for nothing else.
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

if [[ $mode == draft ]]; then
  dist="$(cd "${1:?Missing verified release directory}" && pwd)"
  identity="$("${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" check --output "$dist")"
  read -r sha runtime_id <<<"$identity"
else
  # The drafts of this commit are the evidence; they are fetched and re-checked below.
  dist="$scratch/verified"
  mkdir -p "$dist"
  sha="${CI_COMMIT_SHA:-}"
  runtime_id="$("${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" source | jq -er '.runtime_id')"
fi
[[ "$sha" =~ ^[0-9a-f]{40}$ && "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || exit 2

# call METHOD URL [curl arguments]: the body lands in $scratch/response.
# Returns 0 for 2xx, 4 for a definite 404, 2 for anything else (network, auth, rate limit).
call() {
  local method="$1" url="$2" code
  shift 2
  code="$(curl -q --silent --show-error --max-time 600 -K "$scratch/auth" -X "$method" \
    -o "$scratch/response" -w '%{http_code}' "$@" "$url")" || { echo "Request failed: $method ${url#"$api"}" >&2; return 2; }
  case "$code" in
    2??) return 0 ;;
    404) return 4 ;;
    *) echo "The forge answered $code to $method ${url#"$api"}" >&2; return 2 ;;
  esac
}
is_current() {
  call GET "$api/repos/$repository/branches/main" || return 2
  [[ $(jq -r '.commit.id' "$scratch/response") == "$sha" ]]
}
is_current || { printf 'Source %s is no longer main; publication refused.\n' "$sha" >&2; exit 2; }

# Leaves the one release carrying the tag in release.json. Only a definite absence
# (4) permits creation; a lookup that can not be completed stops everything.
release_state() {
  local tag="$1" page
  for page in $(seq 1 20); do
    call GET "$api/repos/$repository/releases?limit=50&page=$page" || return 2
    jq -e 'type == "array"' "$scratch/response" >/dev/null || return 2
    if jq -e --arg tag "$tag" 'any(.[]; .tag_name == $tag)' "$scratch/response" >/dev/null; then
      jq -e --arg tag "$tag" '[.[] | select(.tag_name == $tag)] | length == 1' "$scratch/response" >/dev/null || return 2
      jq -ce --arg tag "$tag" '.[] | select(.tag_name == $tag)' "$scratch/response" > "$scratch/release.json"
      jq -e '(.id | type == "number" and . > 0) and (.draft | type == "boolean") and (.assets | type == "array")' "$scratch/release.json" >/dev/null || return 2
      return 0
    fi
    jq -e 'length == 0' "$scratch/response" >/dev/null && return 4
  done
  echo "Too many releases to scan for $tag." >&2
  return 2
}

# A published release's tag is immutable: it must name the exact source commit.
validate_tag() {
  local tag="$1" expected="$2" draft="$3" status=0
  [[ "$expected" =~ ^[0-9a-f]{40}$ ]] || { echo "Release $tag has no exact target commit." >&2; return 2; }
  call GET "$api/repos/$repository/tags/$tag" || status=$?
  if [[ $status == 4 && "$draft" == true ]]; then return 0; fi
  [[ $status == 0 ]] || { echo "Release $tag has no verifiable immutable tag." >&2; return 2; }
  [[ $(jq -r '.commit.sha' "$scratch/response") == "$expected" ]] || {
    echo "Existing tag $tag targets a different source commit." >&2; return 2
  }
}

# fetch_asset NAME DESTINATION: the bytes of one asset of the release in release.json.
fetch_asset() {
  local name="$1" destination="$2" path auth code=000
  path="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .browser_download_url | sub("^https?://[^/]+"; "")' "$scratch/release.json")" || return 2
  for auth in "${reads[@]}"; do
    code="$(curl -q --silent --show-error --max-time 600 -K "$auth" -o "$destination" -w '%{http_code}' "$server$path")" || code=000
    [[ "$code" == 200 ]] && return 0
  done
  echo "The forge answered $code reading the asset $name (the job token can not read the assets of a draft; the runner's read credential can)." >&2
  return 1
}

# Every asset already on the release must be one of ours with exactly our bytes.
verify_existing() {
  local path name expected actual downloaded
  expected="$(for path in "$@"; do basename "$path"; done | LC_ALL=C sort)"
  actual="$(jq -r '.assets[].name' "$scratch/release.json" | LC_ALL=C sort)"
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    grep -Fxq "$name" <<<"$expected" || { echo "Unexpected immutable asset: $name" >&2; return 2; }
    downloaded="$scratch/download-$name"
    [[ -e "$downloaded" ]] || fetch_asset "$name" "$downloaded" || { echo "Cannot read release asset: $name" >&2; return 2; }
    [[ $(sha256sum "$downloaded" | cut -d ' ' -f 1) == "$(sha256sum "$dist/$name" | cut -d ' ' -f 1)" ]] || {
      echo "Existing release asset differs: $name. No overwrite attempted." >&2; return 2
    }
  done <<<"$actual"
}

# drop_truncated TAG FILE...: the forge keeps the part of an interrupted upload it
# received. On a DRAFT, an asset whose size differs from our file of that name can
# serve nobody, so it is removed; an asset of the right size is never touched
# (differing bytes are refused by verify_existing, never replaced).
drop_truncated() {
  local tag="$1" path name held id size
  shift
  [[ $(jq -r '.draft' "$scratch/release.json") == true ]] || return 0
  for path in "$@"; do
    name="$(basename "$path")"
    held="$(jq -r --arg name "$name" '[.assets[] | select(.name == $name)][0] | if . == null then "" else "\(.id) \(.size)" end' "$scratch/release.json")"
    [[ -n $held ]] || continue
    read -r id size <<<"$held"
    if [[ $size != "$(stat -c %s "$path")" ]]; then
      echo "Removing the truncated draft asset $name ($size of $(stat -c %s "$path") bytes)." >&2
      call DELETE "$api/repos/$repository/releases/$(jq -r '.id' "$scratch/release.json")/assets/$id" || return 2
    fi
  done
  release_state "$tag"
}

# upload_asset RELEASE_ID TAG FILE: one upload. A reset or timeout (the home uplink is
# slow and the forge sits behind a proxy) stores nothing, a truncated file, or the
# whole file, and the forge may finish storing it after the client gave up; so wait,
# look at the release before every retry and never upload a name twice.
upload_asset() {
  local id="$1" tag="$2" path="$3" name attempt
  name="$(basename "$path")"
  for attempt in 1 2 3 4; do
    if call POST "$api/repos/$repository/releases/$id/assets?name=$name" \
      -H 'Content-Type: application/octet-stream' --data-binary "@$path"; then
      return 0
    fi
    echo "Upload of $name failed (attempt $attempt of 4)." >&2
    ((attempt == 4)) || sleep $((attempt * ${CCVL_UPLOAD_RETRY_DELAY:-20}))
    release_state "$tag" || return 2
    drop_truncated "$tag" "$path" || return 2
    if jq -e --arg name "$name" '.assets | any(.name == $name)' "$scratch/release.json" >/dev/null; then
      return 0
    fi
  done
  return 2
}

# publish TAG TITLE KIND FILE...: create or complete the draft, leave it a draft.
publish() {
  local tag="$1" title="$2" kind="$3" status=0 path name prerelease=false
  shift 3
  [[ "$kind" == runtime ]] && prerelease=true
  release_state "$tag" || status=$?
  if [[ $status == 4 ]]; then
    validate_tag "$tag" "$sha" true
    call POST "$api/repos/$repository/releases" -H 'Content-Type: application/json' --data "$(jq -cn \
      --arg tag "$tag" --arg sha "$sha" --arg title "$title" --arg notes "$notes" --argjson prerelease "$prerelease" \
      '{tag_name: $tag, target_commitish: $sha, name: $title, body: $notes, draft: true, prerelease: $prerelease}')"
    release_state "$tag"
  elif [[ $status != 0 ]]; then
    return "$status"
  fi
  if [[ "$kind" == runtime && $(jq -r '.draft' "$scratch/release.json") == true && $(jq -r '.target_commitish' "$scratch/release.json") != "$sha" ]]; then
    # The draft of an interrupted run on an earlier commit with the same runtime
    # identity: it has no tag yet, so it follows this commit.
    call PATCH "$api/repos/$repository/releases/$(jq -r '.id' "$scratch/release.json")" -H 'Content-Type: application/json' \
      --data "$(jq -cn --arg sha "$sha" '{target_commitish: $sha}')"
    release_state "$tag"
  fi
  [[ $(jq -r '.target_commitish' "$scratch/release.json") == "$sha" || ( "$kind" == runtime && $(jq -r '.draft' "$scratch/release.json") == false ) ]] || {
    echo "Release $tag targets a different source commit." >&2; return 2
  }
  validate_tag "$tag" "$(jq -r '.target_commitish' "$scratch/release.json")" "$(jq -r '.draft' "$scratch/release.json")"
  drop_truncated "$tag" "$@"
  verify_existing "$@"
  for path in "$@"; do
    name="$(basename "$path")"
    if ! jq -e --arg name "$name" '.assets | any(.name == $name)' "$scratch/release.json" >/dev/null; then
      [[ $(jq -r '.draft' "$scratch/release.json") == true ]] || { echo "Published release $tag is incomplete." >&2; return 2; }
      upload_asset "$(jq -r '.id' "$scratch/release.json")" "$tag" "$path"
    fi
  done
  release_state "$tag"
  verify_existing "$@"
  [[ $(jq -r '.assets | length' "$scratch/release.json") == "$#" ]] || return 2
}

# make_public TAG KIND: the verified draft becomes a release; the tag appears.
make_public() {
  local tag="$1" kind="$2" prerelease=false
  [[ "$kind" == runtime ]] && prerelease=true
  release_state "$tag"
  if [[ $(jq -r '.draft' "$scratch/release.json") == false ]]; then
    validate_tag "$tag" "$(jq -r '.target_commitish' "$scratch/release.json")" false
    return 0
  fi
  if [[ "$kind" == workspace ]]; then
    is_current || { echo "Source advanced; leaving $tag as a draft." >&2; return 2; }
  fi
  call PATCH "$api/repos/$repository/releases/$(jq -r '.id' "$scratch/release.json")" -H 'Content-Type: application/json' \
    --data "$(jq -cn --argjson prerelease "$prerelease" '{draft: false, prerelease: $prerelease}')"
  release_state "$tag"
  [[ $(jq -r '.draft' "$scratch/release.json") == false ]]
  validate_tag "$tag" "$(jq -r '.target_commitish' "$scratch/release.json")" false
}

runtime_tag="runtime-$runtime_id"
workspace_tag="build-$sha"

if [[ $mode == promote ]]; then
  # Fetch both drafts of this commit into one directory and check the whole set again.
  for tag in "$runtime_tag" "$workspace_tag"; do
    release_state "$tag" || { echo "No release $tag to promote." >&2; exit 2; }
    while IFS= read -r name; do
      fetch_asset "$name" "$dist/$name" || { echo "Cannot read release asset: $name" >&2; exit 2; }
    done < <(jq -r '.assets[].name' "$scratch/release.json")
  done
  "${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" check --output "$dist" >/dev/null
  make_public "$runtime_tag" runtime
  make_public "$workspace_tag" workspace
  printf 'Promoted %s and %s.\n' "$runtime_tag" "$workspace_tag"
  # Downstream copies follow the Forgejo release and can not undo it.
  bash .agent/scripts/downstream-release.sh "$dist"
  exit
fi

# What is released: every platform the release holds a build of, never a pending one.
mapfile -t platforms < <(jq -r '.platform_states | to_entries[] | select(.value != "pending") | .key' "$dist/manifest.json")
states="$(jq -r '.platform_states | to_entries | map(.key + " " + (if .value == "built" then "built, not tested" else .value end)) | join("; ")' "$dist/manifest.json")"
notes="Verified runtime $runtime_id from source $sha. Platforms: $states. Receipts and source/dependency identities are included."
runtime_files=()
bundle_files=()
evidence_files=("$dist/manifest.json" "$dist"/gate-*.json)
for platform in "${platforms[@]}"; do
  name="ccvl-$platform"
  if [[ "$platform" == windows-* ]]; then name="$name.exe"; fi
  runtime_files+=("$dist/$name" "$dist/$name.sha256" "$dist/$name.runtime-id")
  bundle_files+=("$dist/ccvl-$platform.tar.gz" "$dist/ccvl-$platform.tar.gz.sha256")
  evidence_files+=("$dist/ccvl-$platform.receipt.json")
done
publish "$runtime_tag" "Compiled runtime ${runtime_id:0:12}" runtime "${runtime_files[@]}" "$dist/runtime-manifest.json"
publish "$workspace_tag" "ccvl ${sha:0:12}" workspace "${bundle_files[@]}" "${evidence_files[@]}"
printf 'Drafted %s platform build(s) and the matching workspace bundle(s) for %s: %s.\n' "${#platforms[@]}" "$sha" "$states"
