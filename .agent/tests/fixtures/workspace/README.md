# Synthetic engine-test workspace

Rust tests copy this small workspace into a temporary directory, together with
the engine's `.agent/typst` library, scaffolds and schemas. It exercises style
discovery, selection, settings, paper, wording, rendering, measurement and
contract logic without reading the checked-in showcase under `outputs/cvl/`.

Every name, record and line here is synthetic test data. None of it describes
a real person or reuses showcase wording; do not copy it into an application.

| Style | Purpose |
| --- | --- |
| `outputs/cvl/cv/ledger` | Default CV: shared-family renderer, shared wording, two substyles, A4, the frozen station and summary contract |
| `outputs/cvl/cv/grid` | Independent one-page CV with its own `layout.typ`; exportable as a portable bundle |
| `outputs/cvl/cl/ledger` | Default letter: the frozen AIDA paragraph contract |
| `outputs/flyers/sketch` | Generic output document: its own small record, fixed A6 geometry, one page |
| `slot-<n>` | Empty style and substyle slots |

The renderers emit fixed, contract-conforming line metrics instead of
measuring text, so measurement code can be tested without tuned wording.
