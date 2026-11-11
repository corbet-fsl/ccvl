# Web outputs

**Scaffold only.** The web page design has not been defined yet. The folders
exist so the engine discovers, builds and checks this document type with the
same 4 × 5 style and substyle grid as the CV and the cover letter.

| Path | State |
| --- | --- |
| `scaffold/style.toml` | `de-ch` / `en-ch`, a 16:9 landscape page, one page, substyle `standard` (default); `slot-2` to `slot-5` are empty substyle slots |
| `scaffold/contract.toml`, `scaffold/scaffold.toml` | No content fields yet |
| `scaffold/layout.typ` | Placeholder renderer: profile name and contact line only |
| `scaffold/standard/<language>/ch/` | Leaf record, strings, layout, Typst entry and checked-in [PDF](scaffold/standard/en/ch/pdf/web.pdf) |
| `slot-2` to `slot-4` | Empty style slots |

The intended purpose is a static first screen of a web page. Fonts, composition, content fields and
any further pages are open decisions. Build one leaf with
`bash ./ccvl build-output web en-ch`.
