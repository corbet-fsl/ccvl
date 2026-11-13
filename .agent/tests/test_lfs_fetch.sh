#!/usr/bin/env bash
# lfs-fetch.sh puts the Git LFS objects of a checkout in place without git-lfs: the right bytes
# over every pointer, nothing changed when an object is wrong or missing, a repeat is a no-op.
# The forge is a fake `curl` that serves a directory of objects.
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-lfs-test.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}

store="$scratch/store"
mkdir -p "$store" "$scratch/bin"
# The fake forge: the batch API answers with a download link for every object the store holds
# (and an error object for the others); a link is a file of the store.
cat > "$scratch/bin/curl" <<'EOF'
#!/usr/bin/env bash
out=; data=; url=
credential=
while [ $# -gt 0 ]; do
  case $1 in
    -K) credential=1; shift 2 ;;
    -o) out=$2; shift 2 ;;
    --data-binary) data=${2#@}; shift 2 ;;
    -X | -H | --max-time) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
# A forge that refuses a request with a credential (as Forgejo's LFS endpoint does with an API
# token) or one without (a private repository).
[ -z "${FAKE_REFUSE_CREDENTIAL:-}" ] || [ -z "$credential" ] || { echo "curl: (22) The requested URL returned error: 401" >&2; exit 22; }
[ -z "${FAKE_REQUIRE_CREDENTIAL:-}" ] || [ -n "$credential" ] || { echo "curl: (22) The requested URL returned error: 401" >&2; exit 22; }
case $url in
  */objects/batch)
    {
      printf '{"objects":['
      sep=
      for oid in $(jq -r '.objects[].oid' "$data"); do
        size=$(jq -r --arg oid "$oid" '.objects[] | select(.oid == $oid) | .size' "$data")
        if [ -e "$FAKE_STORE/$oid" ]; then
          printf '%s{"oid":"%s","size":%s,"actions":{"download":{"href":"http://127.0.0.1:1/lfs/objects/%s"}}}' "$sep" "$oid" "$size" "$oid"
        else
          printf '%s{"oid":"%s","size":%s,"error":{"code":404,"message":"Not Found"}}' "$sep" "$oid" "$size"
        fi
        sep=,
      done
      printf ']}'
    } > "$out"
    ;;
  */objects/*) cp "$FAKE_STORE/${url##*/}" "$out" ;;
  *) echo "unexpected request: $url" >&2; exit 22 ;;
esac
EOF
chmod +x "$scratch/bin/curl"

# A checkout whose *.bin files are LFS pointers: two paths share one object.
repo="$scratch/repo"
git init -q "$repo"
cd "$repo"
git config user.name test
git config user.email test@example.invalid
printf '*.bin filter=lfs diff=lfs merge=lfs -text\n' > .gitattributes
mkdir -p dir
printf 'object one\n' > "$scratch/o1"
printf 'a second, longer object\n' > "$scratch/o2"
for object in o1 o2; do
  oid="$(sha256 "$scratch/$object")"
  cp "$scratch/$object" "$store/$oid"
  printf '%s' "$oid" > "$scratch/$object.oid"
done
pointer() { printf 'version https://git-lfs.github.com/spec/v1\noid sha256:%s\nsize %s\n' "$(cat "$scratch/$1.oid")" "$(wc -c < "$scratch/$1" | tr -d '[:space:]')"; }
pointer o1 > a.bin
pointer o1 > dir/b.bin
pointer o2 > "dir/c d.bin"
printf 'not lfs\n' > plain.txt
chmod 0755 dir/b.bin
git add -A
git commit -q -m pointers

# The committed pointers again (a checkout would run the host's git-lfs filter).
restore_pointers() {
  pointer o1 > a.bin
  pointer o1 > dir/b.bin
  pointer o2 > "dir/c d.bin"
}

run() {
  env PATH="$scratch/bin:$PATH" FAKE_STORE="$store" CCVL_LFS_URL=http://127.0.0.1:1/lfs \
    CCVL_FORGE_READ_TOKEN_FILE=/nonexistent bash "$repo_root/.agent/scripts/lfs-fetch.sh" "$repo" 2>&1
}
head_of() { head -n 1 "$1"; }

# 1. A missing object stops everything and changes nothing.
mv "$store/$(cat "$scratch/o2.oid")" "$scratch/held-back"
if out="$(run)"; then echo "A missing object was accepted: $out" >&2; exit 1; fi
grep -q 'has no LFS object' <<<"$out" || { echo "Unexpected message: $out" >&2; exit 1; }
[ "$(head_of a.bin)" = 'version https://git-lfs.github.com/spec/v1' ] || { echo 'A pointer was changed.' >&2; exit 1; }
mv "$scratch/held-back" "$store/$(cat "$scratch/o2.oid")"

# 2. An object that is not the one its pointer names is refused.
cp "$store/$(cat "$scratch/o1.oid")" "$scratch/good"
printf 'tampered!\n' > "$store/$(cat "$scratch/o1.oid")"
if out="$(run)"; then echo "A wrong object was accepted: $out" >&2; exit 1; fi
grep -q 'is not the one its pointer names' <<<"$out" || { echo "Unexpected message: $out" >&2; exit 1; }
[ "$(head_of a.bin)" = 'version https://git-lfs.github.com/spec/v1' ] || { echo 'A pointer was changed.' >&2; exit 1; }
cp "$scratch/good" "$store/$(cat "$scratch/o1.oid")"

# 3. The objects go over the pointers; modes stay; a file that is not LFS is left alone.
out="$(run)"
grep -q '3 file(s) from 2 object(s) in place' <<<"$out" || { echo "Unexpected summary: $out" >&2; exit 1; }
for pair in "a.bin o1" "dir/b.bin o1" "dir/c d.bin o2"; do
  cmp -s "${pair% *}" "$scratch/${pair##* }" || { echo "Wrong bytes in place: ${pair% *}" >&2; exit 1; }
done
[ "$(head_of plain.txt)" = 'not lfs' ] || { echo 'A plain file was changed.' >&2; exit 1; }
[ -x dir/b.bin ] && [ ! -x a.bin ] || { echo 'File modes were not kept.' >&2; exit 1; }

# 4. Anonymous first: a credential the forge would refuse is never sent when the repository is
# public; a private repository gets the runner's read credential after the anonymous request failed.
restore_pointers
printf 'fixture-read-token\n' > "$scratch/read-token"
out="$(env PATH="$scratch/bin:$PATH" FAKE_STORE="$store" FAKE_REFUSE_CREDENTIAL=1 CCVL_LFS_URL=http://127.0.0.1:1/lfs \
  CCVL_FORGE_READ_TOKEN_FILE="$scratch/read-token" bash "$repo_root/.agent/scripts/lfs-fetch.sh" "$repo" 2>&1)"
grep -q '3 file(s) from 2 object(s) in place' <<<"$out" || { echo "The anonymous request was not tried first: $out" >&2; exit 1; }
restore_pointers
if out="$(env PATH="$scratch/bin:$PATH" FAKE_STORE="$store" FAKE_REQUIRE_CREDENTIAL=1 CCVL_LFS_URL=http://127.0.0.1:1/lfs \
  CCVL_FORGE_READ_TOKEN_FILE=/nonexistent bash "$repo_root/.agent/scripts/lfs-fetch.sh" "$repo" 2>&1)"; then
  echo "A private repository was read without a credential: $out" >&2; exit 1
fi
grep -q 'returned error: 401' <<<"$out" || { echo "Unexpected message: $out" >&2; exit 1; }
out="$(env PATH="$scratch/bin:$PATH" FAKE_STORE="$store" FAKE_REQUIRE_CREDENTIAL=1 CCVL_LFS_URL=http://127.0.0.1:1/lfs \
  CCVL_FORGE_READ_TOKEN_FILE="$scratch/read-token" bash "$repo_root/.agent/scripts/lfs-fetch.sh" "$repo" 2>&1)"
grep -q '3 file(s) from 2 object(s) in place' <<<"$out" || { echo "The read credential was not used: $out" >&2; exit 1; }

# 5. A repeat finds every object in place and asks nobody.
out="$(env PATH="$scratch/bin:$PATH" FAKE_STORE=/nonexistent CCVL_LFS_URL=http://127.0.0.1:1/lfs bash "$repo_root/.agent/scripts/lfs-fetch.sh" "$repo" 2>&1)"
grep -q 'every object is in place already' <<<"$out" || { echo "Unexpected repeat: $out" >&2; exit 1; }
echo 'lfs-fetch: pointers replaced by verified objects; wrong and missing objects refused; repeat is a no-op.'
