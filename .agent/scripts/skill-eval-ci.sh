#!/usr/bin/env bash
# Trusted manual evaluation of committed public synthetic cases and skills.
set +x
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

fail() { printf '%s\n' "$1" >&2; exit 2; }
[[ ${FORGEJO_ACTIONS:-} == true && ${GITHUB_REPOSITORY:-} == corbet-fsl/ccvl &&
   ${GITHUB_REF_NAME:-} == main ]] || fail 'Skill evaluation requires public ccvl main on Forgejo.'
[[ ${CI_COMMIT_SHA:-} =~ ^[0-9a-f]{40}$ ]] || fail 'Missing verified source commit.'
[[ -n ${CARGO_TARGET_DIR:-} && ${CCID_TARGET_LOCK_HELD:-} == "$CARGO_TARGET_DIR" ]] ||
  fail 'Run skill evaluation through the verified ccid adapter and its target lock.'

binary="$CARGO_TARGET_DIR/debug/ccvl"
evidence="$CARGO_TARGET_DIR/ccvl-skill-eval/$CI_COMMIT_SHA"
case "${1:-}" in
  build)
    [[ -z ${GROQ_API_KEY:-} ]] || fail 'The skill evaluator build must not receive GROQ_API_KEY.'
    # Repository code runs in the build: it never sees where the key lives either.
    unset CCVL_GROQ_API_KEY_FILE
    # shellcheck source=.agent/scripts/rust-toolchain.sh
    source .agent/scripts/rust-toolchain.sh
    ccvl_select_rust_toolchain
    "${CCVL_CARGO_COMMAND[@]}" build --locked --bin ccvl
    mkdir -p "$evidence"
    sha256sum "$binary" > "$evidence/binary.sha256"
    "$binary" runtime-id > "$evidence/runtime-id"
    ;;
  evaluate)
    [[ -s "$evidence/binary.sha256" && -s "$evidence/runtime-id" ]] ||
      fail 'The exact source evaluator must be built first without provider credentials.'
    sha256sum --check --strict "$evidence/binary.sha256"
    [[ $("$binary" runtime-id) == "$(cat "$evidence/runtime-id")" ]] ||
      fail 'The evaluator runtime changed after its build.'
    report_dir="$(mktemp -d "$evidence/run.XXXXXXXX")"
    {
      printf 'source_commit=%s\nmodel=%s\n' "$CI_COMMIT_SHA" openai/gpt-oss-20b
      cat "$evidence/binary.sha256" "$evidence/runtime-id"
      sha256sum Cargo.lock .agent/tests/skill-cases.json
    } > "$report_dir/identity.txt"
    # The runner provides the key as a credential file; only this stage reads it.
    if [[ -z ${GROQ_API_KEY:-} && -r ${CCVL_GROQ_API_KEY_FILE:-/nonexistent} ]]; then
      GROQ_API_KEY="$(cat "$CCVL_GROQ_API_KEY_FILE")"
      export GROQ_API_KEY
    fi
    unset CCVL_GROQ_API_KEY_FILE
    if [[ -z ${GROQ_API_KEY:-} ]]; then
      printf '%s\n' 'The runner credential CCVL_GROQ_API_KEY_FILE (a Groq API key file) is required; no provider call can run.' >&2
    fi
    # Fixed repository inputs only; never accept private cases or a response file.
    # The CLI checks its runtime against this extracted source before evaluation.
    set +e
    timeout --signal=TERM --kill-after=30s 1800 \
      "$binary" skill-eval \
        --cases .agent/tests/skill-cases.json \
        --skills-root .agent/skills \
        --model openai/gpt-oss-20b \
        --output "$report_dir/report.json" \
        --summary "$report_dir/summary.md"
    status=$?
    set -e
    printf 'Skill evaluation evidence: %s\nEvaluation exit code: %s\n' "$report_dir" "$status"
    cat "$report_dir/identity.txt"
    # Reports contain only the public synthetic evaluation; retain failed reports too.
    if [[ -f "$report_dir/report.json" ]]; then cat "$report_dir/report.json"; fi
    if [[ $status == 0 && ( ! -s "$report_dir/report.json" || ! -s "$report_dir/summary.md" ) ]]; then
      fail 'Successful evaluation requires a nonempty report.json and summary.md.'
    fi
    exit "$status"
    ;;
  *) fail 'Usage: skill-eval-ci.sh build|evaluate' ;;
esac
