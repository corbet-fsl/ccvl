# Onepager CV style

**Scaffold only.** The design of the one-page CV has not been defined yet. The
folders and files exist so the engine discovers, builds and checks the style;
the renderer prints only the approved profile header.

| Path | State |
| --- | --- |
| `style.toml` | `de-ch` / `en-ch`, portrait A4, one page, substyle `standard` (default); `slot-2` to `slot-5` are empty slots |
| `contract.toml`, `scaffold.toml` | No content fields yet |
| `content/<language>/ch/wording.toml` | Empty `[cv]` table |
| `standard/substyle.toml` | No settings yet |
| `layout.typ` | Placeholder renderer: profile header only |
| `../../shared/onepager/defaults.toml` | `document-v1` adapter baseline copied from Modern |

Fonts, paper, content fields and how Onepager differs from Modern (also one
page) are open decisions.
