# Selecting CI checks

Forgejo Actions on our own `linux` runners is the only place ccvl is built,
tested and released; the workflows are rendered from `.ci/ccid.toml` by `ccid`
(edit the manifest, never `.forgejo/workflows/*.yml`). Every workflow is a manual
dispatch of one exact commit. The GitHub mirror runs `.github/workflows/ci.yml`
for pull requests only, on GitHub's free public runners, and never on our own
pushes. Private downstream data and credential-bearing jobs stay on trusted
internal workers. A release is a deliberate and rare dispatch
(semantic versioning, see the [release policy](releases.md#release-policy-semantic-versioning-released-sparingly));
per-commit CI is the Linux `verify` job alone.

| Workflow | Checks | Notes |
|---|---|---|
| `ci` (job `verify`) | `rust`, `documents`, `lint`, `styles` | content-keyed result reuse through `ccid cached` |
| `downstream-check` | `downstream-check` | a private downstream repository only; see [private downstream](private-downstream.md) |
| `skill-eval` | `skill-eval-build`, `skill-eval` | public `main` only; credential-bearing |
| `release` | `release-*` stages | see [Releases](releases.md) |
| `release-promote` | `release-promote` | makes the draft the public release `v<version>`; GitHub and GitLab pull it |

On a provisioned build worker, use `bash .agent/scripts/ci-check.sh <check>...`:

| Check | Coverage |
|---|---|
| `rust` (default) | Stable Rust formatting, Clippy and locked unit/document tests, including the release evidence, publisher and private downstream guard fixtures |
| `lint` | Actionlint, ShellCheck, REUSE and the CI script behavior tests with preinstalled tools |
| `documents` | Locked Linux release build and independent PDF/text/layout verification |
| `styles` | The same optimized release build as `documents` (thin LTO and one codegen unit, so the two checks share one graph build and target directory), then `public-check` limited to the space-separated `<doc>/<style>` list in `CCVL_CHECK_STYLES` (every style when unset) |
| `downstream-check` | Manual private downstream route: builds the checked-out source's `private-check` command and runs it against that checkout (see [private downstream](private-downstream.md)) |
| `skill-eval-build`, then `skill-eval` | Explicit trusted small-model evaluation; separate credential-free build and credential-bearing evaluation |
| `release-<stage>` | One release stage; see [Releases](releases.md) |

The command uses existing tools. The shared `ccid` adapter supplies a
memory-bounded parallel job and test-thread budget; a direct invocation without
those environment settings retains conservative script defaults. Run only checks
whose inputs changed or whose results are missing. The runner reuses a suitable
preinstalled Rust, preferring an installed stable channel and otherwise the
provisioned rustup default or standalone compiler. `Cargo.toml` declares the
minimum supported version, not an exact compiler requirement. `RUST_TOOLCHAIN`
can explicitly select another installed stable compiler; a missing selection
fails visibly. No worker toolchain installation occurs in the checks themselves
(the `verify` and `release` jobs provision tools through their own scripts). Lint
checks likewise reuse existing tools, including installed Nix store packages
omitted from the worker's PATH. The lightweight bootstrap behavior tests run with
lint. A future advertised Windows binary must also pass bootstrap checks on its
actual native platform.

## Which checks a change needs

`ci` always runs all four checks: a content-keyed result is reused when its
inputs are unchanged, so a change that touches only prose or one style re-runs
only the checks whose declared inputs changed. A false skip is worse than a slow
run; there is no path-based selector. The GitHub mirror's pull-request workflow
runs the Rust, document and lint jobs for every pull request.

Engine tests run on the synthetic workspace in
`.agent/tests/fixtures/workspace` and never read the showcase styles, shared
families, profile or assets under `outputs/cvl/`; see
[Rust test fixtures](testing.md#rust-test-fixtures). With every directory of
`outputs/cvl/` except its README deleted, the suite still passes.

The real-data invariants these tests used to pin are enforced by `check`,
which the `documents` and `styles` checks run (`public-check` for the selected styles keeps every
workspace-wide check):

| Invariant | Enforced by |
|---|---|
| Harvard and opted-in contracts keep the frozen station, Summary, compact-delta and AIDA budgets | workspace-wide frozen-contract check |
| No style vendors measurement code | workspace-wide ownership scan of `outputs/cvl/` |
| Manifest, slots, leaf inventory and referenced files | workspace-wide manifest and style checks |
| Every leaf compiles at every preset and passes its measurement gates; tracked outputs exist and match | document checks of the selected styles |
| Letter metrics carry the contract's fill bounds | measurement of the selected styles |
| A build never reads the other document's tree | document checks of the selected styles |
| Resolved customization copies and portable bundles (`portable_bundle`, Cluster) reproduce the build | document checks of the selected styles |
| The interview station plan fits the station protocol and the entry sources | Harvard CV document checks; `inputs/interviews/stations.toml` selects them |

`styles` renders with the same optimized release binary the `documents` check
builds (thin LTO and one codegen unit from `.cargo/release-profile.toml`, passed
with `--config`; `Cargo.toml` carries no `[profile]` table), so a job that runs
both checks compiles and links the graph once and the second build is a cache
hit. Only the build steps that need the release flags pass them; a plain
`cargo build --release` uses Cargo's defaults. A debug build compiles faster but
checks every document about 15 times slower. Clippy runs before the tests so lint
findings surface before the longer test build.

Compiled targets live in persistent dedicated Cargo storage, namespaced by
canonical repository identity. An explicit `CARGO_TARGET_DIR` is honored. The
shared tool locks the actual target directory, retains unchanged source
freshness, and cleans its owned source scratch. Worker package-cache settings
remain authoritative. `CI_JOBS`, `CI_TEST_THREADS`, `CI_MEMORY_MB` and
`CI_TIMEOUT` permit explicit bounded overrides; memory admission still applies.

These checks provide source validation. The `release` and `release-promote`
workflows provide the complete released path; see [Releases](releases.md). They
require the native evidence of the tested platform, the cross-build evidence of
the built ones and all four shared gates. Linux evidence does not prove macOS,
Windows or ARM behavior. A registry dependency that is not published still blocks
a locked build; a provisional path-patched lock is not release evidence.

The staged-archive Crow route that fed `.agent/scripts/downstream-sync.sh` is
retired with Crow and has no Forgejo workflow yet; until one exists, a private
downstream merges the public upstream by hand (see
[private downstream](private-downstream.md)). External model evaluation has its
own manually selected trusted workflow and never runs with routine Rust or lint
checks.

## Skill evaluation

The manual `skill-eval` workflow runs only for public ccvl `main`. Its first step
compiles the evaluator with locked dependencies and existing Rust, without
provider credentials. Its second step reads the Groq API key from the runner
credential file named by `CCVL_GROQ_API_KEY_FILE` and exports it as
`GROQ_API_KEY` for the evaluator alone. That credential must be provided by an
authorized operator before dispatch; adding the workflow does not provision it or
prove the provider is available. A missing key is a configuration failure, never
a passing evaluation. The legacy hosted skill job in `.github/workflows` stays
disabled.

Both steps verify the source and the shared ccid binary, use the same dedicated
target storage and acquire its lock. The evaluator's recorded SHA-256 and runtime
identity must still match when the second step starts; its CLI also checks the
runtime against the extracted source. A concurrent different build fails closed.
Memory admission and the declared `CI_TIMEOUT` apply through ccid; provider
execution has an additional 30-minute limit. This workflow does not install
compilers or run a native release matrix.

Only checked-in public synthetic `.agent/tests/skill-cases.json` cases and
`.agent/skills` instructions are sent to Groq, with the existing
`openai/gpt-oss-20b` model. Private profiles, opportunities and document artifacts
are not evaluator inputs. Verify that the configured account permits this
evaluation without paid usage before dispatch. The evaluator bounds requests,
retries and completion tokens; dispatch only when the exact source lacks valid
provider results. Do not substitute response-file fixtures for provider evidence.

The script retains JSON, Markdown and source/binary/lock identities under
`$CARGO_TARGET_DIR/ccvl-skill-eval/<commit>/run.<unique>/`, outside disposable
source scratch. It prints the evidence path and public JSON report to the Forgejo
run log, including failed or unavailable results. CLI exit codes remain `0` (pass),
`1` (semantic failure), `2` (configuration failure) and `75` (provider unavailable);
the ccid adapter reports any nonzero result as a failed check. A timeout or absent
report also fails and supplies no semantic proof. Lint runs isolated workflow
guard fixtures only; it neither requires a credential nor calls the model.
