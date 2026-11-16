# Engine vectors

These files pin the `doc` engine to what CareerVector's TypeScript op applier
(`applyOp` in `@cv/mutations`) does. They are generated, never edited by hand.

| file | layer | content |
|---|---|---|
| `states.json` | all | the pre-state of every case, one Yjs update each |
| `v1-ops.json` | V1 ops (V4 shape is asserted by every case) | one op or prose edit applied: resulting structure, changed nodes, the Yjs update bytes |
| `v3-errors.json` | V3 errors | refused ops: the exact reason, document unchanged, no bytes emitted |
| `v6-ids.json` | V6 changed nodes | peer frames received, singly or as a sequence (out of order, duplicate, no visible effect): structure and changed nodes per frame |

V4 (storage traps) is not a separate file: every case compares the structure, so a lazily
upgraded `data` map, a `Y.Text`, a plain `selection.active` array or a UTF-16 offset cannot
drift unseen; the cases tagged `shape` in the generator aim at those traps. V6 is a port
check: it pins the changed-node definition of the engine to what Yjs `observeDeep` reports,
it is not a property of the two engines against each other. Concurrency against Yjs (V2) is
not covered yet.

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
* `translate` also applies the server's rules (`validateTranslateOp`): same profile for source
  and target, different languages, a well-formed translated node. A target language without a
  tree is refused without creating an empty language map.
* Every op validates before it writes, so a refusal leaves no partial write and no bytes
  (`@cv/mutations` can leave an empty `children` array, a `selection` map or a language map behind).
* `node.create` and `selection.update` refuse non-string ids; a `children`, `selection` or language
  value of the wrong type is refused, never replaced.
* `tailored.*` is refused: the web keeps those ops in the `jobs` sub-document.
* Trees deeper than 128 levels are refused instead of walked.
