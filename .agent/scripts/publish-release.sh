#!/usr/bin/env bash
# Publish a complete, source-bound release on Forgejo; retries never replace asset bytes.
#
#   publish-release.sh --preflight      refuse a version that names no release or is released already
#   publish-release.sh DIR              create or complete the DRAFT release of this commit
#   publish-release.sh --promote [DIR]  make the verified draft the public release v<version>
#
# One release per version. The draft is named after its commit (`build-<commit>`, an
# internal identity: the drafts of different commits never meet) and holds every
# asset: the executables with their checksums and runtime identities, the workspace
# bundles, the receipts and the manifests. Promoting is a separate, explicit run: it
# checks the whole set again, COMPLETE (a minor release whose Windows or macOS leg has
# not left its final evidence is refused), derives the two manifests from the receipts
# afresh (the legs that ran after the draft was made are in them now), adds SHA256SUMS
# over every asset and renames the draft to the version, `v<version>`. Nothing is copied
# anywhere: the mirrors on GitHub and GitLab pull the release.
# The version is the one in Cargo.toml; `ccvl release-evidence version` refuses a
# version that names no release (anything but 0.MINOR.PATCH), and a version already
# released (release or tag `v<version>`, draft or public) is never released again.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
# shellcheck source=.agent/scripts/release-evidence.sh
source .agent/scripts/release-evidence.sh
ccvl_prepare_release_evidence
mode=draft
case ${1:-} in
  --promote) mode=promote; shift ;;
  --preflight) mode=preflight; shift ;;
esac
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
  # The draft of this commit is the evidence; it is fetched and re-checked below.
  dist="$scratch/verified"
  mkdir -p "$dist"
  sha="${CI_COMMIT_SHA:-}"
  runtime_id="$("${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" source | jq -er '.runtime_id')"
fi
[[ "$sha" =~ ^[0-9a-f]{40}$ && "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || exit 2
# The version decides what is released (see `ccvl release-evidence platforms`); a version
# that names no release stops here, before anything is asked of the forge.
version_line="$("${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" version)" || exit 2
read -r version kind <<<"$version_line"
public_tag="v$version"
draft_tag="build-$sha"

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

# A version is released once: no release (draft or public) and no tag may carry it yet.
# Returns 3 for a version that is taken, 2 when the forge could not tell.
version_is_free() {
  local status=0
  release_state "$public_tag" || status=$?
  case $status in
    4) ;;
    0) printf 'Release %s exists. A version is released once; raise the version in Cargo.toml for a new release.\n' "$public_tag" >&2; return 3 ;;
    *) return 2 ;;
  esac
  status=0
  call GET "$api/repos/$repository/tags/$public_tag" || status=$?
  case $status in
    4) return 0 ;;
    0) printf 'A tag %s exists already; a version is released once.\n' "$public_tag" >&2; return 3 ;;
    *) return 2 ;;
  esac
}

# release_notes: one text for the draft and the public release, from the evidence in $dist.
release_notes() {
  local states
  states="$(jq -r '.platform_states | to_entries | map(.key + " " + (if .value == "built" then "built, not tested" else .value end)) | join("; ")' "$dist/manifest.json")"
  printf 'ccvl %s (%s release). Verified runtime %s from source %s. Platforms: %s. Receipts and source/dependency identities are included.' \
    "$version" "$kind" "$runtime_id" "$sha" "$states"
}

# publish TAG TITLE FILE...: create or complete the draft, leave it a draft.
publish() {
  local tag="$1" title="$2" status=0 path name
  shift 2
  release_state "$tag" || status=$?
  if [[ $status == 4 ]]; then
    validate_tag "$tag" "$sha" true
    call POST "$api/repos/$repository/releases" -H 'Content-Type: application/json' --data "$(jq -cn \
      --arg tag "$tag" --arg sha "$sha" --arg title "$title" --arg notes "$notes" \
      '{tag_name: $tag, target_commitish: $sha, name: $title, body: $notes, draft: true, prerelease: false}')"
    release_state "$tag"
  elif [[ $status != 0 ]]; then
    return "$status"
  fi
  [[ $(jq -r '.target_commitish' "$scratch/release.json") == "$sha" ]] || {
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

# fetch_release TAG: every asset of that release into $dist.
fetch_release() {
  local name
  while IFS= read -r name; do
    fetch_asset "$name" "$dist/$name" || { echo "Cannot read release asset: $name" >&2; return 2; }
  done < <(jq -r '.assets[].name' "$scratch/release.json")
}

# replace_in_draft NAME: the draft ends up holding $dist/NAME under that name; an asset of the
# name that is there is removed first (a draft only, never a public release).
replace_in_draft() {
  local name="$1" held
  held="$(jq -r --arg name "$name" '[.assets[] | select(.name == $name)][0] | if . == null then "" else "\(.id)" end' "$scratch/release.json")"
  if [[ -n $held ]]; then
    call DELETE "$api/repos/$repository/releases/$(jq -r '.id' "$scratch/release.json")/assets/$held" || return 2
  fi
  upload_asset "$(jq -r '.id' "$scratch/release.json")" "$draft_tag" "$dist/$name" || return 2
  release_state "$draft_tag" || return 2
}

# write_sums FILE: the checksums of every asset of the release in $dist but the checksum file
# itself, in coreutils format and in byte order. The mirrors on GitHub and GitLab pull the
# release and fail closed on an asset that has no entry in a checksum file.
write_sums() {
  (cd "$dist" && find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%f\0' | LC_ALL=C sort -z | xargs -0 sha256sum --) > "$1"
}

# seal_draft: what a promotion publishes is derived from the complete set of receipts and
# never trusted as the draft holds it. The legs of the Windows and macOS clones ran after the
# draft was made, so the two manifests are derived again (`check --complete` refuses a set
# whose legs have not left their final evidence) and, with the checksum file of every asset,
# replaced in the draft wherever they differ from what it holds.
seal_draft() {
  local name fetched="$scratch/fetched-derived"
  mkdir -p "$fetched"
  for name in manifest.json runtime-manifest.json SHA256SUMS; do
    if [[ -e $dist/$name ]]; then mv -- "$dist/$name" "$fetched/$name"; fi
  done
  "${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" check --complete --output "$dist" >/dev/null
  for name in manifest.json runtime-manifest.json; do
    if [[ -e $fetched/$name ]] && cmp -s -- "$fetched/$name" "$dist/$name"; then continue; fi
    echo "The draft's $name is not what its receipts say; replacing it."
    replace_in_draft "$name" || return 2
  done
  # The checksum file covers the manifests as they are now, so it comes last.
  write_sums "$dist/SHA256SUMS"
  if [[ -e $fetched/SHA256SUMS ]] && cmp -s -- "$fetched/SHA256SUMS" "$dist/SHA256SUMS"; then return 0; fi
  echo "The draft's SHA256SUMS does not cover its assets; writing it."
  replace_in_draft SHA256SUMS || return 2
}

# make_public: the verified draft of this commit becomes the release of the version;
# the tag appears, on this commit.
make_public() {
  is_current || { echo "Source advanced; leaving $draft_tag as a draft." >&2; return 2; }
  call PATCH "$api/repos/$repository/releases/$(jq -r '.id' "$scratch/release.json")" -H 'Content-Type: application/json' \
    --data "$(jq -cn --arg tag "$public_tag" --arg name "$version" --arg notes "$(release_notes)" \
      '{tag_name: $tag, name: $name, body: $notes, draft: false, prerelease: false}')"
  release_state "$public_tag"
  [[ $(jq -r '.draft' "$scratch/release.json") == false ]]
  validate_tag "$public_tag" "$sha" false
}

if [[ $mode == preflight ]]; then
  version_is_free
  printf 'Release %s (%s release) of %s may proceed. Platforms: %s.\n' "$public_tag" "$kind" "${sha:0:12}" \
    "$("${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" platforms | paste -sd ' ')"
  exit
fi

if [[ $mode == promote ]]; then
  status=0
  release_state "$public_tag" || status=$?
  if [[ $status == 0 ]]; then
    # Promoted already (a repeat): it must be this commit's, public and complete.
    [[ $(jq -r '.draft' "$scratch/release.json") == false ]] || { echo "Release $public_tag is a draft of another run; nothing is promoted." >&2; exit 2; }
    validate_tag "$public_tag" "$sha" false
    fetch_release
    "${CCVL_RELEASE_EVIDENCE_COMMAND[@]}" check --complete --output "$dist" >/dev/null
    write_sums "$scratch/SHA256SUMS.expected"
    cmp -s -- "$scratch/SHA256SUMS.expected" "$dist/SHA256SUMS" || {
      echo "The SHA256SUMS of $public_tag does not cover its assets." >&2; exit 2
    }
    printf 'Release %s is public already.\n' "$public_tag"
  elif [[ $status == 4 ]]; then
    # Fetch the draft of this commit and check the whole set again.
    release_state "$draft_tag" || { echo "No release $draft_tag to promote." >&2; exit 2; }
    fetch_release
    seal_draft
    version_is_free
    release_state "$draft_tag"
    make_public
    printf 'Promoted %s as %s.\n' "$draft_tag" "$public_tag"
  else
    exit "$status"
  fi
  # Promotion publishes on Forgejo and nothing else: GitHub and GitLab fetch the release
  # themselves (the mirror workflows rendered into the repository pull the tag, verify it
  # against SHA256SUMS and publish the same bytes).
  exit
fi

# What is released: every platform the release holds a build of, never a pending one.
mapfile -t platforms < <(jq -r '.platform_states | to_entries[] | select(.value != "pending") | .key' "$dist/manifest.json")
states="$(jq -r '.platform_states | to_entries | map(.key + " " + (if .value == "built" then "built, not tested" else .value end)) | join("; ")' "$dist/manifest.json")"
notes="$(release_notes)"
files=("$dist/runtime-manifest.json" "$dist/manifest.json" "$dist"/gate-*.json)
for platform in "${platforms[@]}"; do
  name="ccvl-$platform"
  if [[ "$platform" == windows-* ]]; then name="$name.exe"; fi
  files+=("$dist/$name" "$dist/$name.sha256" "$dist/$name.runtime-id" \
    "$dist/ccvl-$platform.tar.gz" "$dist/ccvl-$platform.tar.gz.sha256" "$dist/ccvl-$platform.receipt.json")
done
version_is_free
publish "$draft_tag" "$version (draft of ${sha:0:12})" "${files[@]}"
printf 'Drafted %s platform build(s) for %s as %s: %s. Promoting makes it %s.\n' "${#platforms[@]}" "$sha" "$draft_tag" "$states" "$public_tag"
