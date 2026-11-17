# Agent instructions

`ccvl` is the shared CareerVector engine: style contracts and bundles, rendering
(feature `render`) and the Yrs document engine with its session API (feature `doc`,
modules `doc` and `session`). The products `careervector-cli`, `careervector-tui` and
`careervector-web` depend on it; it depends on none of them.

- No I/O of its own: storage goes through `cvws` (feature `storage`, off on wasm32); no storage backend, no shell runner, no chat provider. Storage belongs to
  `cvws` and its backends (`cvfs`, `cvdb`).
- Rendered output stays byte-identical; the vectors in `tests/vectors` prove it.
- Comments and documentation are English.

The data sub-documents (`jobs`, `settings`, `layout`, `order-state`) are `Doc::scoped_data`; their ops are in `src/doc/data.rs` and pinned by `tests/vectors/v7-data.json`. `src/doc/jobid.rs` holds the frozen URL-to-job-id math of `@cv/jobcache/ids`; it must stay identical to the `jobcache` contract crate of the CareerVector monorepo (the vectors fail on drift).

`transfer` (jobs CSV export/import) reproduces the TypeScript reference byte for byte; its fixtures under
`tests/fixtures/transfer` come from `tests/transfer-ts/gen.ts` (see its README), never from hand edits.

Checks (CI runs the same): `cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets --all-features -- -D warnings`,
`cargo test --locked --all-features`.
