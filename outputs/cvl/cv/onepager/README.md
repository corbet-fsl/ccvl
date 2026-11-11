# Onepager

One A4 page in Archivo with D+ typography. The header has two columns: name,
headline, two contact rows and the summary on the left; an optional
application photo on the right in the classic 3:4 ratio, spanning exactly from
the top of the name to the baseline of the last summary line. Below follow
optional sections, each between two fine red rules: core competencies (rows
with a bold label), experience (groups with a title, an
"organisations | period" line and stations with an "organisation, place |
role" heading and one bullet), education and independent projects.

Every field is optional; content a leaf leaves empty is not rendered. The
public showcase leaves are empty, so they print the profile header only. The
fields are listed in [`scaffold.toml`](scaffold.toml); the design settings live
in [`../../shared/onepager/defaults.toml`](../../shared/onepager/defaults.toml)
under `[one_pager]`, and the line-count bounds in [`contract.toml`](contract.toml).
The summary uses exactly `summary_lines` lines; `allow_thin` admits lines
below 95 % fill. The page never shrinks type: content that does not fit fails
the one-page check.
