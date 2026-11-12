# Create a private downstream

Some forges do not allow private forks of public repositories, so create a
separate private repository with shared Git history and add ccvl as the
`upstream` remote.

```sh
git clone <ccvl repository URL> applications
cd applications
git remote rename origin upstream
# Create an empty private repository on your forge, then:
git remote add origin <private applications repository URL>
git push --set-upstream origin main
```

This is a private downstream in Git terms even though the forge may not display
it as a fork.

The clone temporarily contains the reference-only personal showcase under the
narrow replacement permission in
`LICENSES/LicenseRef-CCVL-Personal-Content.txt`. Keep the working repository
non-public while replacing the approved sources under `outputs/cvl/`, the signature
asset, public identifier manifest, and generated personal PDFs. The reusable
mechanism and neutral scaffolds may be modified under their separate licenses;
the showcase author's claims and wording may not be carried into another
person's application.

Keep the same domains as upstream: private user knowledge in `inputs/interviews/`, the
approved general document in `outputs/cvl/`, and one keyed package at
`opportunities/<organisation-key>/<position-key>/` for every concrete job.
Create an opportunity with `ccvl new-opportunity`; do not maintain a second
Markdown copy of the tailored fields in `application.toml`.

## Strict ownership policy

Keep a `ccvl-downstream.json` file in the downstream. It names the exact
upstream remote and the only paths the downstream may own. A personalised
downstream normally owns its policy, sync workflow, `inputs/interviews/`, `outputs/cvl/`, and
`opportunities/`; `.agent/`, the root launchers, schemas, skills, tests, and
fixed layout contracts remain upstream-owned. Narrow this list when a
downstream intentionally keeps the public showcase unchanged. `upstream.url` is
optional: when omitted, the check only requires the named remote (e.g.
`upstream`) and its branch to exist; when present, it must be an
`https://…/….git` URL on any forge and must match the remote.

Run the boundary gate after fetching upstream:

```sh
cargo run --locked -- downstream-check \
  --upstream-ref refs/remotes/upstream/main
```

The command fails if the downstream does not contain the fetched upstream
commit, if the named remote or its branch is missing or differs from a configured
`upstream.url`, or if even one unlisted path differs.
There is no implicit exception.

## Updating from ccvl

Keep personal changes in private commits. To import a new upstream release:

```sh
git fetch upstream
git merge --no-edit upstream/main
```

Do not poll for unattended updates. A trusted workflow may merge the exact
triggering commit, enforce the ownership policy, run the complete deterministic
document gate, and push only the verified merge. A superseded event is a no-op;
a conflict, structural deviation, document failure, or ownership breach leaves
the private remote unchanged. The GitHub Actions workflow in
`.github/workflows/downstream-sync.yml` is the generic reusable workflow; the
maintainer's own Forgejo runner keeps private interview and opportunity data off
hosted runners. There is no Forgejo sync workflow yet, so the maintainer imports
upstream releases with the commands above and verifies them with the route
below. The sync script reads the public repository from `CCVL_UPSTREAM_URL`
(`https://<forge>/<owner>/ccvl.git`), so it works against any mirror.

Keep credentials in the workflow runner's repository-scoped secret store and
restrict them to the intended event. Never expose personal downstream content
or its derived logs to an untrusted runner.

If private commits are already ahead, use a normal merge. Reserve a deliberate
history rewrite for a one-time conversion, protect the former tip with a remote
tag first, and review the exact replacement tree before using
`--force-with-lease`.

## Checking private changes on Forgejo

The inherited `.forgejo/workflows/downstream-check.yml` is a manual, check-only
route for a private repository. Dispatch it on the private repository's `main`
(or any exact commit) so it runs on a trusted `linux` runner of the owner. It
never merges or pushes. The public repository, a repository the forge does not
report as private, and runs outside a Forgejo runner (including GitHub-hosted
CI) are refused before the check runner starts.

The Forgejo runner exports `FORGEJO_ACTIONS=true` and the usual job environment
(`GITHUB_REPOSITORY`, `GITHUB_EVENT_NAME=workflow_dispatch`, `GITHUB_SHA`,
`GITHUB_SERVER_URL`, `GITHUB_API_URL`, and the job token under `GITHUB_TOKEN`,
`FORGEJO_TOKEN` and `CCID_FORGE_TOKEN`). The command refuses unless it runs on a
Forgejo runner, the event is a manual dispatch and the repository is not the
public one; it then asks the forge's own API, with the job token, whether the
repository is private and refuses a public answer. The token reaches only the API
call and the history fetch, never a published-runtime download or a child process
that does not need it. Publication credentials are refused outright, and the
runner-level release credential location is never passed on.

The job's checkout is the staged source. The command compares its
file/content/executable-mode inventory with the committed tree, completes a
shallow history, clones it without hooks or configuration and compares the
inventories again. Unsupported links, incomplete history, changed source, or an
invalid upstream policy fail closed. CI files remain upstream-owned under
`ccvl-downstream.json`.

The route runs `ccvl private-check` (hidden maintainer command, built from the
checked-out source by `ci-check.sh downstream-check`). The public upstream is
named by the policy's `url` (`CCVL_UPSTREAM_URL` may override it to the same
value); it is read from the forge that runs the job, because jobs have no route
to the public host (`CCVL_UPSTREAM_FETCH_URL` names another place to fetch from).
An explicit 40-character `UPSTREAM_COMMIT` selects the upstream commit the
private history must contain; without one, the newest public commit the private
history already contains is used. The command uses the runtime identity
algorithm of the compiled source, which equals the bootstrap one, and the
matching public `runtime-<identity>/ccvl-linux-x86_64` release and checksum.
Optional `CCVL_CHECK_RUNTIME` points to an existing worker artifact; its bytes
must still match the downloaded official checksum and its executable runtime
identity. Without that input, the binary is downloaded into disposable job
storage. No compiler, packages, worker tools, or user runtime cache are
installed; the checking command itself uses the worker's existing Rust
toolchain, and the published runtime is never built from source. Missing
published runtime assets or an unreachable release host produce a failed
prerequisite, not a source build or a successful validation result.

The published runtime runs `downstream-check --policy ccvl-downstream.json
--upstream-ref <upstream commit>` followed by the complete `check` gate. Private
command output is captured and discarded; the run log receives only a fixed
failure stage or a success receipt containing source, upstream and runtime
identities. This route declares no secrets and rejects publication tokens.

## Publishing improvements

Reimplement or cherry-pick only generic changes into a clean ccvl checkout.
Run `public-check`, inspect every staged path, and confirm that the change
contains no interview evidence, company research, application, recipient, or
outcome data.
