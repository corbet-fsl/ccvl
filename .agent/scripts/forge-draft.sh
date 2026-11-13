#!/usr/bin/env bash
# Sourced, not run: the Windows stages' access to the releases of the commit under test
# on the forge (the DRAFTS a release run leaves, and public releases alike).
#
# The job's own token lists releases and writes assets, but it can not read the assets of
# a draft back; the runner's read-only credential can. That credential is used for
# reading and for nothing else, and tokens reach curl through configuration files, never
# through arguments.
#
# The caller sets `scratch` (a private directory it removes) before forge_setup. Afterwards
# these are set: sha, repository, server, api, and the curl configuration files
# $scratch/auth (the job token) and the list `reads` (read credential first).

# shellcheck disable=SC2034,SC2154 # sha, repository, server, api and reads are read by the sourcing script; scratch is its.
# forge_setup: validate the environment of a job on public ccvl main and prepare curl.
forge_setup() {
  local read_file read_token
  sha="${CI_COMMIT_SHA:-}"
  repository="${GITHUB_REPOSITORY:-}"
  server="${GITHUB_SERVER_URL:-}"
  server="${server%/}"
  api="${GITHUB_API_URL:-$server/api/v1}"
  token="${CCID_FORGE_TOKEN:-${FORGEJO_TOKEN:-${GITHUB_TOKEN:-}}}"
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'The exact source commit (CI_COMMIT_SHA) is required.' >&2; return 2; }
  # The loopback form exists for the offline fixtures only.
  [[ "$repository" == corbet-fsl/ccvl && ( "$server" == https://* || "$server" == http://127.0.0.1:* ) ]] || {
    echo 'Not the ccvl repository on the forge.' >&2; return 2
  }
  [[ "$token" =~ ^[A-Za-z0-9_.-]{8,}$ ]] || { echo 'The Forgejo job token is required.' >&2; return 2; }
  printf 'header = "Authorization: token %s"\n' "$token" > "$scratch/auth"
  chmod 0600 "$scratch/auth"
  unset token
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
}

# forge_call METHOD URL [curl arguments]: the body lands in $scratch/response. Returns 0 for
# 2xx, 4 for a definite 404, 2 for anything else (network, authorisation, rate limit).
forge_call() {
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

# forge_release TAG FILE: the one release carrying the tag (draft or public), as JSON in FILE.
# Returns 4 when there is none; only a completed scan of the list says so.
forge_release() {
  local tag="$1" file="$2" page
  for page in $(seq 1 20); do
    forge_call GET "$api/repos/$repository/releases?limit=50&page=$page" || return 2
    jq -e 'type == "array"' "$scratch/response" >/dev/null || return 2
    if jq -e --arg tag "$tag" 'any(.[]; .tag_name == $tag)' "$scratch/response" >/dev/null; then
      jq -e --arg tag "$tag" '[.[] | select(.tag_name == $tag)] | length == 1' "$scratch/response" >/dev/null || return 2
      jq -ce --arg tag "$tag" '.[] | select(.tag_name == $tag)' "$scratch/response" > "$file"
      jq -e '(.id | type == "number" and . > 0) and (.draft | type == "boolean") and (.assets | type == "array")' "$file" >/dev/null || return 2
      return 0
    fi
    if jq -e 'length == 0' "$scratch/response" >/dev/null; then break; fi
  done
  echo "No release $tag on the forge." >&2
  return 4
}

# forge_fetch_asset RELEASE_FILE NAME DESTINATION: the bytes of one asset of that release.
forge_fetch_asset() {
  local release="$1" name="$2" destination="$3" path auth code=000
  path="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .browser_download_url | sub("^https?://[^/]+"; "")' "$release")" || {
    echo "The release has no asset $name." >&2; return 2
  }
  for auth in "${reads[@]}"; do
    code="$(curl -q --silent --show-error --max-time 600 -K "$auth" -o "$destination" -w '%{http_code}' "$server$path")" || code=000
    [[ $code == 200 ]] && return 0
  done
  echo "The forge answered $code reading the asset $name." >&2
  return 2
}
