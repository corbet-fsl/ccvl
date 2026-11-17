# Transfer parity fixtures

`gen.ts` runs the CareerVector reference (`lib/domain/src/export.ts` and `import.ts`, which call
SheetJS) over the cases in `cases.ts` and writes the fixtures that `tests/transfer.rs` compares the
Rust `ccvl::transfer` module with. The job-creation helpers of `Dashboard.svelte`
(`jobCreateAttributes` and its helpers) and `normalizeSalary` are extracted verbatim from the checkout,
so the oracle cannot drift from them.

```sh
cd tests/transfer-ts
bun install
# SheetJS resolves `@cv/schemas` from node_modules: link the checkout's package once
mkdir -p node_modules/@cv && ln -s "$CV_ROOT/lib/schemas" node_modules/@cv/schemas
ln -s "$PWD/node_modules" "$CV_ROOT/node_modules"      # so the reference finds nanoid, zod, xlsx
CV_ROOT=/path/to/careervector bun --no-install gen.ts           # rewrite fixtures
CV_ROOT=/path/to/careervector bun --no-install gen.ts --check   # fail on drift
```

Fixtures never contain date-like text for import: SheetJS turns it into serial numbers and the engine
keeps the text (`dates.json` records the divergence).
