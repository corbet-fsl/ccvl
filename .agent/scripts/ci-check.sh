#!/usr/bin/env bash
# Provider-independent checks using an already provisioned build worker.
set -euo pipefail
exec 2>&1
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
# shellcheck source=.agent/scripts/rust-toolchain.sh
source .agent/scripts/rust-toolchain.sh
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"

# One optimized build serves the documents and styles checks: same profile,
# same target directory, so a job that runs both compiles and links the graph
# once. The release flags are a cargo config file passed with --config; the
# manifest carries no [profile] table (build-efficiency rule 2).
ccvl_build_optimized() {
  "${CCVL_CARGO_COMMAND[@]}" build --locked --bin ccvl --release \
    --config .cargo/release-profile.toml
}
ccvl_optimized_binary="${CARGO_TARGET_DIR:-target}/release/ccvl"

if (($# == 0)); then
  set -- rust
fi
for check in "$@"; do
  case "$check" in
    rust)
      ccvl_select_rust_toolchain
      # Key the style-parity evidence on the checkout's own revision, never on a
      # per-run value, so the same tree reuses the same directory (rule 8).
      revision="$(git rev-parse --verify HEAD 2>/dev/null)" || revision=
      if [[ -n "$revision" ]]; then
        export CCVL_STYLE_EVIDENCE="${CARGO_TARGET_DIR:-target}/style-parity/$revision"
      fi
      "${CCVL_CARGO_COMMAND[@]}" fmt --all -- --check
      # Clippy's metadata-only check is the quicker gate on a cold cache, so
      # lint findings surface before the full test build and run.
      "${CCVL_CARGO_COMMAND[@]}" clippy --locked --workspace --all-targets --all-features -- -D warnings
      "${CCVL_CARGO_COMMAND[@]}" test --locked --workspace --all-features
      ;;
    core-package)
      ccvl_select_rust_toolchain
      "${CCVL_CARGO_COMMAND[@]}" package --locked -p ccvl-core
      ;;
    lint)
      # shellcheck source=.agent/scripts/existing-tool-path.sh
      source .agent/scripts/existing-tool-path.sh
      ccvl_use_existing_lint_tools
      # The GitHub workflows are the mirror's pull-request checks; the sync one stays disabled.
      actionlint -ignore '^constant expression "false" in condition\. remove the if: section$' \
        -shellcheck shellcheck .github/workflows/*.yml
      shellcheck .agent/scripts/*.sh .agent/tests/*.sh ccvl
      reuse lint
      bash .agent/tests/test_bootstrap.sh
      bash .agent/tests/test_ci_toolchain.sh
      bash .agent/tests/test_downstream_sync.sh
      bash .agent/tests/test_lfs_fetch.sh
      bash .agent/tests/test_skill_eval_ci.sh
      ;;
    documents)
      [[ $(uname -s) == Linux ]] || { echo 'documents requires Linux' >&2; exit 2; }
      ccvl_select_rust_toolchain
      for command in file qpdf pdfinfo pdftotext pdffonts pdfdetach pdfimages pdftoppm jq; do
        command -v "$command" >/dev/null || { echo "Missing existing tool: $command" >&2; exit 2; }
      done
      ccvl_build_optimized
      bash .agent/scripts/check-linux-deep.sh "$ccvl_optimized_binary"
      ;;
    styles)
      # Scoped document checks: every workspace-wide check, plus rendering and
      # PDF verification for the space-separated <doc>/<style> selections in
      # CCVL_CHECK_STYLES (every style when unset). It renders with the same
      # optimized release binary the documents check builds, so a job that runs
      # both links once instead of twice.
      ccvl_select_rust_toolchain
      ccvl_build_optimized
      read -r -a selected <<<"${CCVL_CHECK_STYLES:-}"
      filters=()
      for style in ${selected[@]+"${selected[@]}"}; do filters+=(--style "$style"); done
      "$ccvl_optimized_binary" public-check ${filters[@]+"${filters[@]}"}
      ;;
    downstream-check)
      # Private downstream route: the ccvl runtime built from the checked-out
      # private commit checks that tree once the upstream commit's own ci run
      # is proven green (no published runtime is needed between releases).
      ccvl_select_rust_toolchain >&2
      "${CCVL_CARGO_COMMAND[@]}" run --locked --quiet --bin ccvl -- private-check
      ;;
    skill-eval-build)
      bash .agent/scripts/skill-eval-ci.sh build
      ;;
    skill-eval)
      bash .agent/scripts/skill-eval-ci.sh evaluate
      ;;
    release-*)
      bash .agent/scripts/release-ci.sh "${check#release-}"
      ;;
    *) echo "Unknown check: $check (rust lint documents styles downstream-check skill-eval-build skill-eval release-*)" >&2; exit 2 ;;
  esac
done
