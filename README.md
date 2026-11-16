# ccvl

<!-- ccid:security:begin (rendered by ccid render; do not edit by hand) -->
**Security and dependencies:** watched by Renovate (with OSV vulnerability alerts); an update is merged only when its tests pass. See the [security policy](SECURITY.md).
<!-- ccid:security:end -->

Portable CCVL style contracts, versioned asset bundles and in-memory document rendering.
Styles remain upstream in CCVL; consumers provide application and profile records separately.

Enable the `render` feature to use `render::StyleRenderer`. A bundle selects an exact
style, substyle, locale, page count and paper size. `StyleBundle::with_record` binds
user data to its asset hash. Rendering uses only bundled assets and explicit inputs.

CCVL exports bundles with `export-style` and renders them with `render-style`.
A compatible style contribution changes the bundle, without copying templates into
consumer applications. Consumers must explicitly select a new bundle version.

The core has no workspace database, filesystem adapter, shell runner or chat provider.
Draft validation checks the style contract and selected variant; CCVL's editorial
readiness and application-submission policy remain separate responsibilities.

## Session API

With the `doc` feature, `ccvl::session::Workspace` keeps one Yrs document per profile
sub-document and offers `open`, `apply`, `receive`, `take_outgoing` and `view`. It does no
I/O and never interprets document content.

## Storage and remote sessions

- `storage` (default on native targets, ignored on wasm32): `ccvl::session::storage::Stored`
  persists a `Workspace` through any `cvws::Store` (files, `SQLite`, light server): cold load,
  one compare-and-swap write per sub-document, `refresh` for what other writers committed.
- `remote`: `ccvl::session::remote::RemoteSession` is a `Stored` on `cvws::RemoteStore`, with
  the durable outbox (`flush`, `queued`), live frames (`apply_live`) and foreign routing shadows.
  Only the append write path is carried.
