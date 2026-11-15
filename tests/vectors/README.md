# Engine vectors

These files pin the `doc` engine to what CareerVector's TypeScript op applier
(`applyOp` in `@cv/mutations`) does. They are generated, never edited by hand.

| file | layer | content |
|---|---|---|
| `states.json` | all | the pre-state of every case, one Yjs update each |
| `v1-ops.json` | V1 ops, V4 shape | one op applied: resulting structure, changed ids, the Yjs update bytes |
| `v3-errors.json` | V3 errors | refused ops: the exact reason, document unchanged |
| `v6-ids.json` | V6 changed ids | peer updates received: structure and changed ids |

A case compares the *structure* of the document, not only its JSON: `Y.Map`,
`Y.Array`, `Y.Text` and plain values are told apart (`$map`, `$array`, `$text`,
`$any`), so a lazily upgraded `data` map, a `Y.Text` or a plain `selection.active`
array cannot drift unseen. Each `source` records the CareerVector commit and the
op catalogue version (10) the vectors came from.

`tests/doc_vectors.rs` consumes them with `cargo test`, no UI involved. For the
other direction, run it with `CCVL_VECTOR_OUT=<dir>` to write the update bytes the
engine produced, then check that Yjs decodes them into the same structure with
`lib/mutations/scripts/ccvl-verify-rust.ts` in CareerVector.

## Regenerate

In a CareerVector checkout with its dependencies installed:

```sh
bun run lib/mutations/scripts/ccvl-vectors.ts <ccvl>/.agent/core/tests/vectors
```

Regenerate whenever `OP_CATALOG_VERSION` changes, then fix the engine until the
tests pass again.

## Known differences from the TypeScript applier

These are deliberate and have no vector:

* A prose edit never splits a surrogate pair; the TypeScript diff can insert a
  lone surrogate when two emoji share a high surrogate.
* `node.move` into the moved node's own subtree is refused (TypeScript silently
  loses the node).
* A refused `node.update` data patch changes nothing; TypeScript may have applied
  earlier keys of the same patch.
* `translate` refuses a target profile other than `cv_profile` / `cl_profile`.
