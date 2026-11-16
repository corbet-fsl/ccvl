# Agent instructions

`ccvl` is the shared CareerVector engine: style contracts and bundles, rendering
(feature `render`) and the Yrs document engine with its session API (feature `doc`,
modules `doc` and `session`). The products `careervector-cli`, `careervector-tui` and
`careervector-web` depend on it; it depends on none of them.

- No I/O, no storage backend, no shell runner, no chat provider. Storage belongs to
  `cvws` and its backends (`cvfs`, `cvdb`).
- Rendered output stays byte-identical; the vectors in `tests/vectors` prove it.
- Comments and documentation are English.

Checks (CI runs the same): `cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets --all-features -- -D warnings`,
`cargo test --locked --all-features`.
