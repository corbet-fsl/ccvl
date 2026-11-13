#!/usr/bin/env bash
# Test the Windows x86_64 platform of ccvl in the offline Windows clone: the executable
# that ships, and the test suite, both for the one Windows ABI (gnullvm, llvm-mingw).
#
# 1. The release job cross-builds ccvl-windows-x86_64.exe and leaves it in the draft
#    release of its commit. This stage fetches THAT file (it does not build it), proves it
#    is the file the receipt names, hands it to the clone with `vm-win` and runs it there:
#    the identity compiled into it, then `public-check` on the source of the same commit.
# 2. It builds the test executables of the workspace for the same target, with the same
#    llvm-mingw tools (llvm-mingw-env.sh) and the toolchain of the job, hands them to the
#    clone with the source tree at the path they were built at, and runs them there.
#
# The job of the label `windows` owns the clone for its length (woken before, rolled back
# after); the clone has no network and holds nothing of Julian's. The result is a JSON
# report of the clone (checks, test counts, the activation facts): windows-attest.sh
# turns it into the native receipt of the platform.
#
#   windows-test.sh [REPORT.json]      assets from the forge release of CI_COMMIT_SHA
#   CCVL_WINDOWS_ASSET_DIR=DIR         assets from a directory instead (a rehearsal; the
#                                      same checks, the same clone)
# Exit status: 0 only when the executable and every test passed in the clone.
set +x
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
report="${1:-}"
platform=windows-x86_64
exe_name="ccvl-$platform.exe"
# Must equal the Rust target of this platform in release_evidence.rs (PLATFORMS).
expected_host=x86_64-pc-windows-gnullvm
[[ -z ${GH_TOKEN:-} ]] || { echo 'Publication credentials must not reach the Windows test.' >&2; exit 2; }
unset CCVL_RELEASE_DOWNSTREAM_TOKENS_FILE
for tool in jq curl sha256sum tar vm-win; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool (a job of the label windows has it)." >&2; exit 2; }
done
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ccvl-windows-test.XXXXXXXX")"
trap 'find "$scratch" -delete' EXIT
assets="$scratch/assets"
mkdir -p "$assets"

if [[ -n ${CCVL_WINDOWS_ASSET_DIR:-} ]]; then
  for name in "$exe_name" "$exe_name.sha256" "$exe_name.runtime-id" "ccvl-$platform.receipt.json"; do
    cp -- "$CCVL_WINDOWS_ASSET_DIR/$name" "$assets/$name"
  done
  printf 'Assets from %s (rehearsal).\n' "$CCVL_WINDOWS_ASSET_DIR"
else
  # shellcheck source=.agent/scripts/forge-draft.sh
  source .agent/scripts/forge-draft.sh
  forge_setup
  # The one draft of this commit (`build-<commit>`) holds the receipt and the executable alike.
  forge_release "build-$sha" "$scratch/build-release.json"
  if ! jq -e --arg name "ccvl-$platform.receipt.json" 'any(.assets[]; .name == $name)' "$scratch/build-release.json" >/dev/null; then
    # The platform set follows the version: a patch release holds Linux x86_64 alone.
    printf 'The release build-%s does not hold %s (%s); there is nothing to test.\n' "${sha:0:12}" "$platform" \
      "$(jq -r '[.assets[].name | select(test("^ccvl-.*\\.receipt\\.json$")) | sub("^ccvl-"; "") | sub("\\.receipt\\.json$"; "")] | join(", ")' "$scratch/build-release.json")"
    exit 0
  fi
  for name in "ccvl-$platform.receipt.json" "$exe_name" "$exe_name.sha256" "$exe_name.runtime-id"; do
    forge_fetch_asset "$scratch/build-release.json" "$name" "$assets/$name"
  done
  printf 'Assets of release build-%s (draft).\n' "${sha:0:12}"
fi

# The file that runs must be the file the receipt names: receipt, checksum file and bytes agree.
# A receipt that is already native names the same file; only the identity of the bytes matters.
receipt="$assets/ccvl-$platform.receipt.json"
[[ $(jq -r '.platform' "$receipt") == "$platform" && $(jq -r '.rust_host' "$receipt") == "$expected_host" ]] || {
  echo "The receipt is not that of $platform built for $expected_host." >&2; exit 2
}
receipt_sha="$(jq -er --arg name "$exe_name" '.assets[$name]' "$receipt")"
recorded_sha="$(cut -d ' ' -f 1 "$assets/$exe_name.sha256")"
actual_sha="$(sha256sum "$assets/$exe_name" | cut -d ' ' -f 1)"
runtime_id="$(jq -er '.runtime_id' "$receipt")"
[[ "$runtime_id" =~ ^[0-9a-f]{64}$ ]] || { echo 'The receipt carries no runtime identity.' >&2; exit 2; }
[[ "$receipt_sha" == "$recorded_sha" && "$recorded_sha" == "$actual_sha" ]] || {
  echo "The executable does not match its receipt: receipt $receipt_sha, checksum file $recorded_sha, bytes $actual_sha." >&2; exit 2
}
[[ $(tr -d '\r\n' < "$assets/$exe_name.runtime-id") == "$runtime_id" ]] || {
  echo 'The runtime-id asset differs from the receipt.' >&2; exit 2
}
printf '%s\n  sha256 %s\n  runtime %s\n  built by %s\n' "$exe_name" "$actual_sha" "$runtime_id" "$(jq -r '.tool' "$receipt")"

# What the clone gets of the executable: the file, what to expect of it, and the source of
# the same commit (the file is run against the documents it will render; no Git is needed there).
mkdir -p "$scratch/src"
tar -C "$repo_root" --exclude=./.git --exclude=./target -cf - . | tar -C "$scratch/src" -xf -
jq -n --arg sha256 "$actual_sha" --arg runtime_id "$runtime_id" '{sha256: $sha256, runtime_id: $runtime_id}' > "$scratch/expect.json"


spp_text="$(vm-win spp)"
printf '%s\n' "$spp_text"
vm-win put "$assets/$exe_name" "$scratch/expect.json" "$scratch/src"
exe_rc=0
vm-win run .agent/scripts/windows-clone-exe.ps1 || exe_rc=$?
share_job="${VM_WIN_SHARE:?}/job"
if [[ -r $share_job/public-check.txt ]]; then
  printf -- '--- public-check output (tail)\n'
  sed '1s/^\xEF\xBB\xBF//' "$share_job/public-check.txt" | tail -n 12
fi
exe_checks='[]'
if [[ -r $share_job/result.json ]]; then
  sed '1s/^\xEF\xBB\xBF//' "$share_job/result.json" | jq -c '(if type == "array" then . else [.] end)' > "$scratch/exe-checks.json"
  exe_checks="$(cat "$scratch/exe-checks.json")"
  printf -- '--- result.json\n'
  jq -r '.[] | "\(if .ok then "ok  " else "FAIL" end) \(.name): \(.detail)"' <<<"$exe_checks"
fi
if ((exe_rc == 0)); then
  printf 'The shipped %s (sha256 %s) passed in the offline Windows clone.\n' "$exe_name" "$actual_sha"
else
  printf 'The shipped %s (sha256 %s) FAILED in the offline Windows clone (exit %s).\n' "$exe_name" "$actual_sha" "$exe_rc" >&2
fi

# ---- The test suite, for the same ABI --------------------------------------------------
# Built here, with the tools that built the executable; a test executable of this target
# embeds the absolute paths of this checkout (CARGO_MANIFEST_DIR, fixtures), so the clone
# gets the checkout at the same absolute path below the share (`vm-win mirror`) and the
# executables run on its own disk, from the root of that tree.
# shellcheck source=.agent/scripts/rust-toolchain.sh
source .agent/scripts/rust-toolchain.sh
# shellcheck source=.agent/scripts/llvm-mingw-env.sh
source .agent/scripts/llvm-mingw-env.sh
ccvl_select_rust_toolchain
test_rustc="$("${CCVL_RUSTC_COMMAND[@]}" --version)"
sysroot="$("${CCVL_RUSTC_COMMAND[@]}" --print sysroot)"
# Toolchains are provisioned explicitly by the job, never installed implicitly here.
[[ -d $sysroot/lib/rustlib/$expected_host/lib ]] || {
  echo "The Rust toolchain has no $expected_host standard library (the job provisions it)." >&2; exit 2
}
ccvl_llvm_mingw_env "$expected_host"
test_tool="$(ccvl_llvm_mingw_identity "$expected_host")"
built_rustc="$(jq -r '.rustc' "$receipt")"
printf 'Test suite: %s with %s (the executable: %s).\n' "$test_rustc" "$test_tool" "$built_rustc"
[[ $test_rustc == "$built_rustc" ]] || echo "Note: the test suite is built with another compiler than the executable; the ABI is the same."
build_json="$scratch/test-build.json"
suite_started=$SECONDS
# ctypst declares crate-type ["cdylib", "rlib"], and cargo builds the DLL for every target
# of a dependency. A cdylib that exports no C symbol gets an empty export list from rustc,
# and ld.lld (MinGW mode) then exports EVERY symbol, 108,000 of them at opt-level 0, past the
# 65,535 a PE file holds ("too many exported symbols"). The release profile escapes by
# optimising most of them away; the dev profile of a test build does not. Nothing here uses
# that DLL, so the linker is told not to export anything automatically.
"${CCVL_CARGO_COMMAND[@]}" test --locked --workspace --all-features --no-run \
  --target "$expected_host" \
  --config "target.$expected_host.rustflags=[\"-C\", \"link-arg=-Wl,--exclude-all-symbols\"]" \
  --message-format=json-render-diagnostics > "$build_json"
jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) | .executable' "$build_json" | sort -u > "$scratch/tests.txt"
test_count="$(wc -l < "$scratch/tests.txt")"
((test_count > 0)) || { echo 'The build produced no test executable.' >&2; exit 2; }
printf '%s test executables built in %s s.\n' "$test_count" "$((SECONDS - suite_started))"
: > "$scratch/tests.list"
while IFS= read -r test_exe; do
  vm-win put "$test_exe"
  printf '%s\n' "$(basename -- "$test_exe")" >> "$scratch/tests.list"
done < "$scratch/tests.txt"
vm-win put "$scratch/tests.list"
vm-win mirror "$repo_root"
top="${repo_root#/}"
top="${top%%/*}"
# Where the checkout is, for the clone: the first path component below the share root and the path itself.
jq -n --arg top "$top" --arg src "C:$repo_root" '{top: $top, src: $src}' > "$scratch/suite.json"
vm-win put "$scratch/suite.json"
suite_rc=0
vm-win run .agent/scripts/windows-clone-suite.ps1 || suite_rc=$?
passed=0 failed=0 ignored=0 results=0
for log in "$share_job"/full-*.txt; do
  [[ -r $log ]] || continue
  while read -r p f i; do
    passed=$((passed + p)); failed=$((failed + f)); ignored=$((ignored + i)); results=$((results + 1))
  done < <(sed '1s/^\xEF\xBB\xBF//' "$log" | tr -d '\r' \
    | sed -n 's/^test result: [A-Za-z]*\. \([0-9][0-9]*\) passed; \([0-9][0-9]*\) failed; \([0-9][0-9]*\) ignored.*/\1 \2 \3/p')
done
if ((failed > 0 || suite_rc != 0)); then
  for log in "$share_job"/full-*.txt; do
    [[ -r $log ]] || continue
    printf -- '--- why: %s\n' "$(basename -- "$log")"
    sed '1s/^\xEF\xBB\xBF//' "$log" | tr -d '\r' | grep -E 'panicked at|os error|No such file|cannot find|The system cannot|not found|^test .* FAILED' | cut -c1-200 | sort | uniq -c | sort -rn | head -8 || true
  done
fi
suite_ok=false
((suite_rc == 0 && failed == 0 && passed > 0 && results >= test_count)) && suite_ok=true
suite_detail="$passed passed, $failed failed, $ignored ignored in $test_count test executables (clone exit $suite_rc)"
printf 'Windows test suite in the offline clone: %s\n' "$suite_detail"

jq -n \
  --arg commit "${CI_COMMIT_SHA:-}" --arg platform "$platform" --arg host "$expected_host" \
  --arg sha "$actual_sha" --arg runtime "$runtime_id" --arg rustc "$test_rustc" --arg tool "$test_tool" \
  --arg spp "$spp_text" --argjson exe "$exe_checks" --argjson suite_ok "$suite_ok" --arg detail "$suite_detail" \
  --argjson executables "$test_count" --argjson passed "$passed" --argjson failed "$failed" --argjson ignored "$ignored" \
  '{schema: 1, platform: $platform, rust_host: $host, source_commit: $commit,
    executable_sha256: $sha, runtime_id: $runtime,
    test_build: {rustc: $rustc, tool: $tool},
    clone: {activation: ($spp | split("\n"))},
    tests: {executables: $executables, passed: $passed, failed: $failed, ignored: $ignored},
    checks: ($exe + [{id: "test-suite", name: "the workspace test suite built for the same ABI", ok: $suite_ok, detail: $detail}])}' \
  > "$scratch/report.json"
if [[ -n $report ]]; then cp -- "$scratch/report.json" "$report"; printf 'Report: %s\n' "$report"; fi
((exe_rc == 0)) && [[ $suite_ok == true ]]
