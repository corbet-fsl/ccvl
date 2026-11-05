#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Isolated command fixtures: no network, builds, credentials or actual pushes.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/bin" "$fixture/source/.agent/scripts"
cp "$repo_root/.agent/scripts/downstream-sync.sh" "$repo_root/.agent/scripts/rust-toolchain.sh" \
  "$repo_root/.agent/scripts/memory-admission.sh" "$fixture/source/.agent/scripts/"
cp "$repo_root/Cargo.toml" "$fixture/source/Cargo.toml"
export TEST_SOURCE_COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export TEST_MERGED_COMMIT=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
export TEST_TRACE="$fixture/trace"
export SOURCE_ARCHIVE="$fixture/source.tar" UPSTREAM_SOURCE_BUNDLE="$fixture/source.bundle"
printf 'source fixture\n' > "$SOURCE_ARCHIVE"
printf 'bundle fixture\n' > "$UPSTREAM_SOURCE_BUNDLE"
SOURCE_SHA256="$(sha256sum "$SOURCE_ARCHIVE" | awk '{print $1}')"
UPSTREAM_SOURCE_SHA256="$(sha256sum "$UPSTREAM_SOURCE_BUNDLE" | awk '{print $1}')"
export SOURCE_SHA256 UPSTREAM_SOURCE_SHA256
export CARGO_HOME="$fixture/cargo" CI_CACHE_ROOT="$fixture/cargo"
export CI_JOBS=1 CI_MIN_AVAILABLE_MB=1 CI_MEMORY_PER_JOB_MB=1
export DOWNSTREAM_PUSH_KEY=fixture-only DOWNSTREAM_SSH_URL=fixture-only
export CCVL_UPSTREAM_URL=https://forge.example.invalid/corbet-libs/ccvl.git
export PATH="$fixture/bin:$PATH"
# Memory admission reads fixture kernel interfaces, never the host's.
mkdir -p "$fixture/proc/self" "$fixture/cgroup"
printf 'MemTotal: 33554432 kB\nMemAvailable: 16777216 kB\n' > "$fixture/proc/meminfo"
printf '0::/\n' > "$fixture/proc/self/cgroup"
export CCVL_PROC_ROOT="$fixture/proc" CCVL_CGROUP_ROOT="$fixture/cgroup"
unset RUST_TOOLCHAIN RUSTUP_TOOLCHAIN

cat > "$fixture/bin/git" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == -C ]]; then shift 2; fi
case "$1" in
  get-tar-commit-id) cat >/dev/null; printf '%s\n' "$TEST_SOURCE_COMMIT" ;;
  bundle)
    if [[ "$2" == list-heads ]]; then printf '%s refs/heads/source\n' "$TEST_SOURCE_COMMIT"; fi ;;
  init|remote|fetch|update-ref|config) ;;
  clone) mkdir -p "${@: -1}" ;;
  merge-base) exit 1 ;;
  merge) printf 'merge\n' >> "$TEST_TRACE" ;;
  rev-parse)
    if [[ "$2" == FETCH_HEAD ]]; then printf '%s\n' "$TEST_SOURCE_COMMIT"
    else printf '%s\n' "$TEST_MERGED_COMMIT"; fi ;;
  ls-remote)
    if [[ "$3" == upstream ]]; then
      printf 'upstream-freshness\n' >> "$TEST_TRACE"
      case "$TEST_MAIN_STATE" in
        unchanged) printf '%s\trefs/heads/main\n' "$TEST_SOURCE_COMMIT" ;;
        advanced) printf '%s\trefs/heads/main\n' "$TEST_MERGED_COMMIT" ;;
        unavailable) exit 128 ;;
      esac
    else printf '%s\trefs/heads/main\n' "$TEST_MERGED_COMMIT"; fi ;;
  push) printf 'push\n' >> "$TEST_TRACE" ;;
  *) printf 'Unexpected fixture Git command: %s\n' "$1" >&2; exit 99 ;;
esac
SH
cat > "$fixture/bin/rustup" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == run ]] || exit 99
shift 2
case "$1" in
  rustc) echo 'rustc 1.97.1 (fixture)' ;;
  cargo)
    if [[ "$2" == --version ]]; then echo 'cargo 1.97.1 (fixture)'; exit 0; fi
    [[ "$2" == build && "$3" == --locked ]] || exit 99
    mkdir -p "$CARGO_TARGET_DIR/debug"
    cat > "$CARGO_TARGET_DIR/debug/ccvl" <<'GATE'
#!/usr/bin/env bash
set -euo pipefail
printf 'gate:%s\n' "$1" >> "$TEST_TRACE"
if [[ "$1" == check && "$TEST_DOCUMENT_GATE" == failure ]]; then exit 1; fi
GATE
    chmod +x "$CARGO_TARGET_DIR/debug/ccvl"
    ;;
  *) exit 99 ;;
esac
SH
chmod +x "$fixture/bin/git" "$fixture/bin/rustup"

expect_jobs() {
  local expected="$1" actual status=0
  shift
  # shellcheck disable=SC2016 # The child shell expands its own positional argument.
  actual="$(env "$@" bash -c 'source "$1"; ccvl_memory_admission' _ "$repo_root/.agent/scripts/memory-admission.sh" 2>"$fixture/admission-error")" || status=$?
  if [[ "$expected" == refused ]]; then
    ((status != 0)) || { echo 'Expected memory admission refusal.' >&2; exit 1; }
  else
    [[ "$status" == 0 && "$actual" == "$expected" ]] || {
      cat "$fixture/admission-error"
      echo "Expected $expected build job(s), got ${actual:-none}." >&2
      exit 1
    }
  fi
}
cpus="$(nproc)"
jobs_for() { if (($1 <= cpus)); then echo "$1"; else echo "$cpus"; fi; }
expect_jobs "$(jobs_for 2)" CARGO_BUILD_JOBS=2 CI_MIN_AVAILABLE_MB=1 CI_MEMORY_PER_JOB_MB=1
expect_jobs "$(jobs_for 4)" CARGO_BUILD_JOBS=64 CI_MIN_AVAILABLE_MB=16380 CI_MEMORY_PER_JOB_MB=1
expect_jobs refused CARGO_BUILD_JOBS=0
expect_jobs refused CARGO_BUILD_JOBS=two
expect_jobs refused CARGO_BUILD_JOBS=2 CI_MIN_AVAILABLE_MB=16384 CI_MEMORY_PER_JOB_MB=1
# A cgroup limit lowers what the host reports as available.
printf '1073741824\n' > "$fixture/cgroup/memory.max"
printf '1073741824\n' > "$fixture/cgroup/memory.current"
expect_jobs refused CARGO_BUILD_JOBS=2 CI_MIN_AVAILABLE_MB=1 CI_MEMORY_PER_JOB_MB=1
printf '2147483648\n' > "$fixture/cgroup/memory.max"
expect_jobs "$(jobs_for 1)" CARGO_BUILD_JOBS=1 CI_MIN_AVAILABLE_MB=100 CI_MEMORY_PER_JOB_MB=100
printf 'max\n' > "$fixture/cgroup/memory.max"
# Sustained full memory pressure refuses admission.
mkdir -p "$fixture/proc/pressure"
printf 'some avg10=1.00 avg60=0.00\nfull avg10=7.50 avg60=0.00\n' > "$fixture/proc/pressure/memory"
expect_jobs refused CARGO_BUILD_JOBS=2 CI_MIN_AVAILABLE_MB=1 CI_MEMORY_PER_JOB_MB=1
printf 'some avg10=1.00 avg60=0.00\nfull avg10=4.99 avg60=0.00\n' > "$fixture/proc/pressure/memory"
expect_jobs "$(jobs_for 2)" CARGO_BUILD_JOBS=2 CI_MIN_AVAILABLE_MB=1 CI_MEMORY_PER_JOB_MB=1
rm -rf "$fixture/proc/pressure" "$fixture/cgroup/memory.max" "$fixture/cgroup/memory.current"

run_case() {
  export TEST_MAIN_STATE="$1" TEST_DOCUMENT_GATE="$2"
  : > "$TEST_TRACE"
  if bash "$fixture/source/.agent/scripts/downstream-sync.sh" "$TEST_SOURCE_COMMIT" > "$fixture/output" 2>&1; then
    [[ "$3" == success ]] || { cat "$fixture/output"; echo 'Expected sync failure.' >&2; exit 1; }
  else
    [[ "$3" == failure ]] || { cat "$fixture/output"; echo 'Unexpected sync failure.' >&2; exit 1; }
  fi
  if [[ "$3" == failure ]] && grep -qx push "$TEST_TRACE"; then
    echo 'Failed verification attempted a private push.' >&2
    exit 1
  fi
}
run_case unchanged success success
expected=$'merge\ngate:downstream-check\ngate:check\nupstream-freshness\npush'
[[ "$(cat "$TEST_TRACE")" == "$expected" ]] || { cat "$TEST_TRACE"; exit 1; }
run_case advanced success failure
run_case unavailable success failure
run_case unchanged failure failure
printf 'Downstream freshness and failed-gate publication guards passed.\n'
