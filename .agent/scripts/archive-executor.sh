#!/usr/bin/env bash
# Minimal executor for the archive gate: "bash SCRIPT DIST" runs in a new user
# and mount namespace whose root holds the closure of a few ordinary tools and
# nothing else: no Git, no Rust, no compiler. It needs no privilege, only user
# namespaces and Nix (the tools come from a pinned nixpkgs, substituted, never built).
# Absence is a property of the root, not a masked PATH.
set -euo pipefail
script="${1:?Missing check script}"
dist="$(cd "${2:?Missing package directory}" && pwd)"
source_ref="${CCVL_ARCHIVE_TOOLS:-github:NixOS/nixpkgs/b7c2ada94fe99c15b0dbcf4d11fd7850b957a436?narHash=sha256-IItrdb7Puk05RqOBWZYFC5X6Wl1sJmCfh5MWVHw5iMM=}"
tools=(bash coreutils gnutar gzip gnused gnugrep gawk findutils diffutils)
refs=()
for tool in "${tools[@]}"; do refs+=("$source_ref#$tool"); done
mapfile -t outputs < <(nix build --no-link --print-out-paths "${refs[@]}")
mapfile -t closure < <(nix path-info --recursive "${outputs[@]}")
for store in "${closure[@]}"; do
  for forbidden in git cargo rustc rustup gcc cc clang; do
    if [[ -e "$store/bin/$forbidden" ]]; then
      echo "The minimal root would contain $forbidden." >&2; exit 2
    fi
  done
done
glibc="$(printf '%s\n' "${closure[@]}" | grep -E -- '-glibc-[0-9][^/]*$' | head -n 1)"
[[ -x "$glibc/lib/ld-linux-x86-64.so.2" ]] || { echo 'No dynamic loader in the minimal root.' >&2; exit 2; }
path=
for output in "${outputs[@]}"; do [[ -d "$output/bin" ]] && path="$path${path:+:}$output/bin"; done
root="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-archive-root.XXXXXXXX")"
trap 'rmdir "$root" 2>/dev/null || true' EXIT
# The inner script is deliberately single-quoted: it expands inside the new root.
# shellcheck disable=SC2016
unshare --user --map-root-user --mount -- bash -c '
  set -euo pipefail
  root=$1 dist=$2 script=$3 glibc=$4 path=$5
  shift 5
  mount --make-rprivate /
  mount -t tmpfs -o mode=0755 tmpfs "$root"
  mkdir -p "$root/tmp" "$root/dev" "$root/proc" "$root/work" "$root/lib64" "$root/usr/bin"
  for store in "$@"; do
    mkdir -p "$root$store"
    mount --bind "$store" "$root$store"
    mount -o remount,bind,ro "$root$store"
  done
  for node in null zero urandom; do : > "$root/dev/$node"; mount --bind "/dev/$node" "$root/dev/$node"; done
  # A fresh proc is refused inside a container (too revealing); the host view serves /proc/self.
  mount --rbind /proc "$root/proc"
  mount --bind "$dist" "$root/work"
  cp -- "$script" "$root/tmp/check.sh"
  ln -s "$glibc/lib/ld-linux-x86-64.so.2" "$root/lib64/ld-linux-x86-64.so.2"
  for bin in ${path//:/ }; do [[ -x "$bin/env" ]] && ln -s "$bin/env" "$root/usr/bin/env"; done
  exec chroot "$root" /usr/bin/env -i PATH="$path" HOME=/tmp TMPDIR=/tmp \
    CI_COMMIT_SHA="${CI_COMMIT_SHA:-}" GITHUB_SHA="${GITHUB_SHA:-}" GITHUB_RUN_ID="${GITHUB_RUN_ID:-}" \
    FORGEJO_ACTIONS="${FORGEJO_ACTIONS:-}" bash /tmp/check.sh /work
' bash "$root" "$dist" "$script" "$glibc" "$path" "${closure[@]}"
