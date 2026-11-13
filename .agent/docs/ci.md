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
| `release` (jobs `release`, `release-windows`, `release-macos`, `release-promote`; rendered from `[release]`) | `release-build`; `release-windows`; `release-macos`; `release-seal` | one run, jobs ordered by `needs`: the draft, the platform legs of a minor release (the shipped Windows executable and the test suite in the offline Windows clone; macOS built, tested and packaged in the macOS clone), then the public release `v<version>` once every leg has passed; GitHub and GitLab pull it; see [Releases](releases.md) |

On a provisioned build worker, use `bash .agent/scripts/ci-check.sh <check>...`:

| Check | Coverage |
|---|---|
| `rust` (default) | Stable Rust formatting, Clippy and locked unit/document tests, including the release evidence, publisher and private downstream guard fixtures |
| `lint` | Actionlint, ShellCheck, REUSE and the CI script behavior tests with preinstalled tools |
| `documents` | Locked Linux release build and independent PDF/text/layout verification |
| `styles` | The same optimized release build as `documents` (thin LTO and one codegen unit, so the two checks share one graph build and target directory), then `public-check` limited to the space-separated `<doc>/<style>` list in `CCVL_CHECK_STYLES` (every style when unset) |
| `downstream-check` | Manual private downstream route: builds the checked-out source's `private-check` command and runs it against that checkout (see [private downstream](private-downstream.md)) |
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

These checks provide source validation. The `release` workflow provides the complete released path; see [Releases](releases.md). They
require the native evidence of the tested platform, the cross-build evidence of
the built ones and all four shared gates. Linux evidence does not prove macOS,
Windows or ARM behavior. A registry dependency that is not published still blocks
a locked build; a provisional path-patched lock is not release evidence.

The staged-archive Crow route is retired with Crow. A private downstream follows
the public upstream through the event-driven upstream sync (see
[private downstream](private-downstream.md)); the old push-model script and its
GitHub workflow are removed. Model evaluation is a local
command, never a CI workflow; see below.

## Skill evaluation

`skill-eval` is a local command for whoever holds a provider key, not a CI
workflow. The Forgejo and GitHub skill-eval workflows are removed: hosted
evaluation would need a provider key on someone else's runner, and Julian
declined a Groq key for it (ds-groqdrop, 2026-11-13), so no runner provides
`CCVL_GROQ_API_KEY_FILE` and the GitHub mirror holds no `GROQ_API_KEY` secret.
Run the evaluation locally as documented in
[testing](testing.md#small-model-skill-evaluation), with only the checked-in
public synthetic `.agent/tests/skill-cases.json` cases and `.agent/skills`
instructions as model inputs. Private profiles, opportunities and document
artifacts are never evaluator inputs. CLI exit codes remain `0` (pass),
`1` (semantic failure), `2` (configuration failure) and `75` (provider
unavailable). A timeout or absent report supplies no semantic proof. Do not
substitute response-file fixtures for provider evidence.
