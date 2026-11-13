#!/usr/bin/env bash
# Put the Git LFS objects of a checkout in place without git-lfs.
#
#   lfs-fetch.sh [CHECKOUT]
#
# The macOS build clone has no git-lfs, and a plain clone leaves the committed pointer files
# where the objects belong, so the source it builds from would not be the source that the
# Linux release run staged. This script reads every pointer of the checkout (a tracked file
# whose `filter` attribute is `lfs`), asks the forge's LFS API for the objects, checks each
# one against the size and SHA-256 its pointer names and copies it over the pointer. Git
# then sees the files as modified, which is why the release evidence of the macOS leg is taken
# from an archive of the committed pointers (`SOURCE_ARCHIVE`), exactly as for the Linux run.
#
# Needs curl, jq, git and sha256sum or shasum; bash 3.2 and the BSD userland are enough.
# The repository is public, so every request goes out anonymously first: Forgejo's LFS
# endpoint answers 401 to an API token it does not take as an LFS credential, even for a public
# repository. Only when the anonymous request fails is the runner's read-only credential tried.
# Nothing is installed and nothing is written outside the checkout.
set -euo pipefail
cd "${1:-.}"
git rev-parse --is-inside-work-tree >/dev/null
for tool in curl jq git; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 2; }
done
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}
size_of() {
  local size
  size="$(wc -c < "$1")"
  printf '%s' "${size//[[:space:]]/}"
}
server="${GITHUB_SERVER_URL:-}"
server="${server%/}"
repository="${GITHUB_REPOSITORY:-}"
lfs_url="${CCVL_LFS_URL:-$server/$repository.git/info/lfs}"
[[ $lfs_url == https://* || $lfs_url == http://127.0.0.1:* ]] || {
  echo 'The forge address is required (GITHUB_SERVER_URL and GITHUB_REPOSITORY, or CCVL_LFS_URL).' >&2; exit 2
}
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-lfs.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT

# The runner's read-only credential, when there is one, through a curl configuration file;
# lfs_curl uses it only after an anonymous request has failed.
auth=()
read_file="${CCVL_FORGE_READ_TOKEN_FILE:-${CREDENTIALS_DIRECTORY:+$CREDENTIALS_DIRECTORY/forgejo-ci-read}}"
if [[ -n $read_file && -r $read_file ]]; then
  read_token="$(tr -d '\n' < "$read_file")"
  if [[ $read_token =~ ^[A-Za-z0-9_.-]{8,}$ ]]; then
    printf 'header = "Authorization: token %s"\n' "$read_token" > "$scratch/auth"
    chmod 0600 "$scratch/auth"
    auth=(-K "$scratch/auth")
  fi
  unset read_token
fi

# lfs_curl ARGS...: curl, anonymously; if that fails and there is a credential, once more with it.
lfs_curl() {
  if curl -q --silent --show-error --fail "$@" 2> "$scratch/curl.error"; then return 0; fi
  if [[ ${#auth[@]} -eq 0 ]]; then cat "$scratch/curl.error" >&2; return 1; fi
  curl -q --silent --show-error --fail "${auth[@]}" "$@"
}

# 1. Every pointer: "OID SIZE PATH" per line. A file that already holds its object is left alone.
git -c core.quotepath=off ls-files | git -c core.quotepath=off check-attr --stdin filter | sed -n 's/: filter: lfs$//p' > "$scratch/paths"
: > "$scratch/pointers"
have=0
while IFS= read -r path; do
  [[ -f $path ]] || continue
  if [[ $(size_of "$path") -le 300 ]] && [[ $(head -n 1 "$path") == 'version https://git-lfs.github.com/spec/v1' ]]; then
    oid="$(sed -n 's/^oid sha256:\([0-9a-f]\{64\}\)$/\1/p' "$path")"
    size="$(sed -n 's/^size \([0-9][0-9]*\)$/\1/p' "$path")"
    [[ -n $oid && -n $size ]] || { echo "Not a valid LFS pointer: $path" >&2; exit 2; }
    printf '%s %s %s\n' "$oid" "$size" "$path" >> "$scratch/pointers"
  else
    have=$((have + 1))
  fi
done < "$scratch/paths"
files="$(wc -l < "$scratch/pointers" | tr -d '[:space:]')"
if [[ $files == 0 ]]; then
  echo "LFS: every object is in place already ($have file(s))."
  exit 0
fi
sort -u -k1,1 "$scratch/pointers" | cut -d ' ' -f 1,2 > "$scratch/objects"
objects="$(wc -l < "$scratch/objects" | tr -d '[:space:]')"

# 2. The objects, a hundred to a request: ask where each one is, download it, check it.
split -l 100 "$scratch/objects" "$scratch/batch-"
fetched=0
for batch in "$scratch"/batch-*; do
  jq -Rn '{operation: "download", transfers: ["basic"],
           objects: [inputs | split(" ") | {oid: .[0], size: (.[1] | tonumber)}]}' < "$batch" > "$batch.request"
  lfs_curl --max-time 300 -X POST \
    -H 'Accept: application/vnd.git-lfs+json' -H 'Content-Type: application/vnd.git-lfs+json' \
    --data-binary "@$batch.request" -o "$batch.response" "$lfs_url/objects/batch" || {
    echo "The forge's LFS API did not answer ($lfs_url)." >&2; exit 2
  }
  while IFS=' ' read -r oid size href; do
    [[ -n $oid ]] || continue
    [[ -n $href ]] || { echo "The forge has no LFS object $oid." >&2; exit 2; }
    lfs_curl --max-time 600 -o "$scratch/object-$oid" "$href" || {
      echo "Cannot download the LFS object $oid." >&2; exit 2
    }
    [[ $(size_of "$scratch/object-$oid") == "$size" && $(sha256 "$scratch/object-$oid") == "$oid" ]] || {
      echo "The LFS object $oid is not the one its pointer names." >&2; exit 2
    }
    fetched=$((fetched + 1))
  done < <(jq -r '.objects[] | "\(.oid) \(.size) \(.actions.download.href // "")"' "$batch.response")
done
[[ $fetched == "$objects" ]] || { echo "Fetched $fetched of $objects LFS objects." >&2; exit 2; }

# 3. Over every pointer. cp keeps the mode the checkout gave the file.
while IFS=' ' read -r oid size path; do
  cp -- "$scratch/object-$oid" "$path"
done < "$scratch/pointers"
echo "LFS: $files file(s) from $objects object(s) in place; $have file(s) held their object already."
