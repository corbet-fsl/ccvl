#!/usr/bin/env bash
# Every feature must compile alone (consumers take `default-features = false` with one feature):
# the TUI takes `render`, crates/jobcache takes `jobid`, the editor's wasm takes `doc`, the sheet module `sheet`.
# A module gated by the wrong feature (0.1.8: `session` reached crate::doc without `doc`) shows up here.
set -euo pipefail
cd "$(dirname "$0")/.."
args=(--locked)
[[ ${CCVL_OFFLINE:-} = 1 ]] && args+=(--offline)
echo "== --no-default-features"
cargo check "${args[@]}" --no-default-features
features=$(sed -n '/^\[features\]/,/^\[/p' Cargo.toml | sed -n 's/^\([a-z][a-z-]*\) = .*/\1/p' | grep -vx default)
for f in $features; do
  echo "== --no-default-features --features $f"
  cargo check "${args[@]}" --no-default-features --features "$f"
done
# Combinations consumers use.
for combo in "render,jobid" "doc,locale" "core,jobid"; do
  echo "== --no-default-features --features $combo"
  cargo check "${args[@]}" --no-default-features --features "$combo"
done
