#!/usr/bin/env bash
# Copy the verified, public Forgejo releases of this commit to the downstream forges.
#
# One runner-level credential file (CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE) holds the
# upload tokens as JSON: {"github": "...", "gitlab": "..."}. A downstream with no
# token, or one this runner has no route to, is reported as skipped and is not a
# failure; a copy that began and did not complete is. Existing assets are never replaced.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
dist="$(cd "${1:?Missing verified release directory}" && pwd)"
targets=.agent/release-downstreams.json
tokens_file="${CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE:-}"
sha="$(jq -er '.source_commit' "$dist/manifest.json")"
runtime_id="$(jq -er '.runtime_id' "$dist/manifest.json")"
[[ "$sha" =~ ^[0-9a-f]{40}$ && "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || exit 2
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-downstream.XXXXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT

# The same file sets the Forgejo releases carry: every platform that has a build.
mapfile -t platforms < <(jq -r '.platform_states | to_entries[] | select(.value != "pending") | .key' "$dist/manifest.json")
runtime_files=("$dist/runtime-manifest.json")
bundle_files=("$dist/manifest.json" "$dist"/gate-*.json)
for platform in "${platforms[@]}"; do
  name="ccvl-$platform"
  if [[ "$platform" == windows-* ]]; then name="$name.exe"; fi
  runtime_files+=("$dist/$name" "$dist/$name.sha256" "$dist/$name.runtime-id")
  bundle_files+=("$dist/ccvl-$platform.tar.gz" "$dist/ccvl-$platform.tar.gz.sha256" "$dist/ccvl-$platform.receipt.json")
done
releases=("runtime-$runtime_id|runtime|true" "build-$sha|workspace|false")
notes="Verified runtime $runtime_id from source $sha, copied from the Forgejo release of the same name."

status=0
report() { printf 'downstream %s: %s\n' "$1" "$2"; }
files_of() { if [[ $1 == runtime ]]; then printf '%s\n' "${runtime_files[@]}"; else printf '%s\n' "${bundle_files[@]}"; fi; }

# token NAME: the upload token of one downstream, empty when there is none.
token() {
  [[ -r $tokens_file ]] || return 0
  jq -er --arg name "$1" '.[$name] // empty | select(type == "string" and test("^[A-Za-z0-9_.-]{8,}$"))' "$tokens_file" 2>/dev/null || true
}
reachable() { curl -q --silent --max-time 20 -o /dev/null -I "$1"; }
# req AUTHFILE METHOD URL [curl arguments]: body in $scratch/response; 0 for 2xx, 4 for 404, 2 otherwise.
req() {
  local auth="$1" method="$2" url="$3" code
  shift 3
  code="$(curl -q --silent --show-error --max-time 600 -K "$auth" -X "$method" -o "$scratch/response" -w '%{http_code}' "$@" "$url")" || return 2
  case "$code" in 2??) return 0 ;; 404) return 4 ;; *) echo "The downstream answered $code to $method ${url##*/}" >&2; return 2 ;; esac
}

copy_github() {
  local value api uploads repo entry tag kind prerelease path name id page found
  value="$(token github)"
  [[ -n $value ]] || { report github 'skipped: no upload token (CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE has no github entry)'; return 0; }
  api="$(jq -er '.github.api' "$targets")"; uploads="$(jq -er '.github.uploads' "$targets")"; repo="$(jq -er '.github.repository' "$targets")"
  if ! reachable "$api" || ! reachable "$uploads"; then
    report github "skipped: $api or $uploads is not reachable from this runner"; return 0
  fi
  printf 'header = "Authorization: Bearer %s"\nheader = "Accept: application/vnd.github+json"\n' "$value" > "$scratch/github-auth"
  for entry in "${releases[@]}"; do
    IFS='|' read -r tag kind prerelease <<<"$entry"
    found=
    for page in 1 2 3 4 5; do
      req "$scratch/github-auth" GET "$api/repos/$repo/releases?per_page=100&page=$page" || return 1
      if jq -e --arg tag "$tag" 'any(.[]; .tag_name == $tag)' "$scratch/response" >/dev/null; then
        jq -ce --arg tag "$tag" '[.[] | select(.tag_name == $tag)][0]' "$scratch/response" > "$scratch/gh-release.json"; found=1; break
      fi
      jq -e 'length == 0' "$scratch/response" >/dev/null && break
    done
    if [[ -z $found ]]; then
      req "$scratch/github-auth" POST "$api/repos/$repo/releases" -H 'Content-Type: application/json' --data "$(jq -cn \
        --arg tag "$tag" --arg sha "$sha" --arg notes "$notes" --argjson prerelease "$prerelease" \
        '{tag_name: $tag, target_commitish: $sha, name: $tag, body: $notes, draft: true, prerelease: $prerelease}')" || return 1
      cp "$scratch/response" "$scratch/gh-release.json"
    fi
    id="$(jq -er '.id' "$scratch/gh-release.json")"
    while IFS= read -r path; do
      name="$(basename "$path")"
      if jq -e --arg name "$name" --argjson size "$(stat -c %s "$path")" '.assets | any(.name == $name and .size == $size)' "$scratch/gh-release.json" >/dev/null; then continue; fi
      if jq -e --arg name "$name" '.assets | any(.name == $name)' "$scratch/gh-release.json" >/dev/null; then
        echo "Downstream asset differs: $name. No overwrite attempted." >&2; return 1
      fi
      [[ $(jq -r '.draft' "$scratch/gh-release.json") == true ]] || { echo "Downstream release $tag is published and incomplete." >&2; return 1; }
      req "$scratch/github-auth" POST "$uploads/repos/$repo/releases/$id/assets?name=$name" -H 'Content-Type: application/octet-stream' --data-binary "@$path" || return 1
    done < <(files_of "$kind")
    if [[ $(jq -r '.draft' "$scratch/gh-release.json") == true ]]; then
      req "$scratch/github-auth" PATCH "$api/repos/$repo/releases/$id" -H 'Content-Type: application/json' --data "$(jq -cn \
        --argjson prerelease "$prerelease" '{draft: false, prerelease: $prerelease, make_latest: (if $prerelease then "false" else "true" end)}')" || return 1
    fi
  done
  report github "copied ${#releases[@]} release(s) to $repo"
}

copy_gitlab() {
  local value api project encoded entry tag kind prerelease path name base links
  value="$(token gitlab)"
  [[ -n $value ]] || { report gitlab 'skipped: no upload token (CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE has no gitlab entry)'; return 0; }
  api="$(jq -er '.gitlab.api' "$targets")"; project="$(jq -er '.gitlab.project' "$targets")"
  reachable "$api/version" || { report gitlab "skipped: $api is not reachable from this runner"; return 0; }
  printf 'header = "PRIVATE-TOKEN: %s"\n' "$value" > "$scratch/gitlab-auth"
  encoded="$(jq -rn --arg project "$project" '$project | @uri')"
  for entry in "${releases[@]}"; do
    IFS='|' read -r tag kind prerelease <<<"$entry"
    base="$api/projects/$encoded/packages/generic/ccvl/$tag"
    if req "$scratch/gitlab-auth" GET "$api/projects/$encoded/releases/$tag"; then
      # Already there: it must hold exactly our file names.
      [[ $(jq -r '[.assets.links[].name] | sort | join(",")' "$scratch/response") == "$(files_of "$kind" | xargs -n1 basename | LC_ALL=C sort | paste -sd,)" ]] || {
        echo "Downstream release $tag exists and differs." >&2; return 1
      }
      continue
    elif (($? != 4)); then
      return 1
    fi
    links='[]'
    while IFS= read -r path; do
      name="$(basename "$path")"
      req "$scratch/gitlab-auth" PUT "$base/$name" --upload-file "$path" || return 1
      links="$(jq -c --arg name "$name" --arg url "$base/$name" '. + [{name: $name, url: $url, link_type: "package"}]' <<<"$links")"
    done < <(files_of "$kind")
    req "$scratch/gitlab-auth" POST "$api/projects/$encoded/releases" -H 'Content-Type: application/json' --data "$(jq -cn \
      --arg tag "$tag" --arg sha "$sha" --arg notes "$notes" --argjson links "$links" \
      '{name: $tag, tag_name: $tag, ref: $sha, description: $notes, assets: {links: $links}}')" || return 1
  done
  report gitlab "copied ${#releases[@]} release(s) to $project"
}

copy_github || { report github 'FAILED'; status=1; }
copy_gitlab || { report gitlab 'FAILED'; status=1; }
exit "$status"
