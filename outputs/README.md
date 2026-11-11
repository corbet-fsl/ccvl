# Outputs

The documents a workspace produces, by type. `cvl/` holds the CV and cover
letter styles, their general sources and the checked-in showcase. Tailored
documents for one vacancy live with that vacancy in `opportunities/`.

The other output documents each own one folder with the same grid of four
style and five substyle positions. Their first style, `scaffold`, prints only
the approved profile header until a design is defined:

| Folder | Document | Page |
| --- | --- | --- |
| [`flyers/`](flyers/README.md) | `flyers` | portrait A4 |
| [`web/`](web/README.md) | `web` | 16:9 landscape first screen |
| [`videos/`](videos/README.md) | `videos` | 16:9 landscape still frame |
| [`presentations/`](presentations/README.md) | `presentations` | 16:9 landscape slide |
| [`portfolios/`](portfolios/README.md) | `portfolios` | portrait A4 |

`ccvl.json` declares every document under `documents`, keyed by the document
name that also names its folder in `workspace_groups.outputs`.
