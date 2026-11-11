# Comparing variants side by side

`ccvl compare` renders two or more variants of a CV or cover letter and
assembles them into one review PDF, so the same page of every variant sits
side by side. A variant can be a style, a substyle, a locale, a page preset,
an application record, one of its kept variants, or a PDF from an older
build. Comparisons are review material: they live in `.agent/cache/` and are
never committed.

```sh
bash ./ccvl compare [--doc <document>] [--locale <locale>] [--pages <preset>]
                    [--only <pages>] [--layout interleaved|sheet] [--name <name>]
                    [--png] [--metrics] [--json] <item> <item> [<item>...]
```

On Windows, use `.\ccvl.cmd compare` with the same arguments.
`bash ./ccvl compare --help` repeats the item forms and examples.

## Items

Each item names one variant, much like the operands of `diff a b c`:

| Item | Renders |
|---|---|
| `harvard/d-plus` | a style and substyle with the showcase content in `outputs/cvl/` |
| `harvard/compact@de-ch:2` | the same with its own locale and page preset |
| `default`, `cluster/default`, `harvard` | the workspace default style, or a style's first substyle |
| `opportunities/acme/lead` or `…/application.toml` | an application record with its own style, language and pages |
| `opportunities/acme/lead/variants/cluster/d-plus/values` | a kept variant record, rendered the same way |
| `old/cv-4.pdf` | an existing PDF, used as is |

`default` resolves exactly like `--style default` and `--substyle default` on
`build-cv`. Selecting an empty slot fails with the usual message, such as
`cv modern/slot-3 is an empty slot; it has no design yet`.

Per item suffixes win over the global options:

- `@<locale>` overrides `--locale`. Without either, a style item renders in
  `en-ch` when its style declares that locale, otherwise in the style's first
  declared locale. A language such as `@de` works when the style has exactly
  one matching locale.
- `:<preset>` overrides `--pages`. Without either, the record's
  `options.pages` (or `options.cl_pages` for letters) applies, then the
  style's default preset.

A record always renders in its own `options.language`, so `@<locale>` on a
record is rejected. A PDF is compared as is and takes no suffix; select its
pages with `--only`. An existing path always wins over the selection grammar.

`--doc cl` compares cover letters instead of CVs, for style items and records
alike.

## Rendering

Style and record items render through the same path as `build-cv` and
`build-cl`, but into `.agent/cache/comparisons/<name>/`; checked-in outputs
such as `outputs/cvl/cv/<style>/<substyle>/<language>/<region>/pdf/cv-4.pdf` are never
written. Every item and the requested pages are validated before the first
render, so a typo fails fast.

Line contracts are reported, not enforced, the way `check` exports its PDFs,
so a draft whose lines are out of bounds still renders. Add `--metrics` to see
the measurements side by side. `measure` and `check` remain the gates.

Variant records under `variants/` are never built or checked by the regular
commands. `compare` renders them only into the cache. A variant kept in an
older record format may no longer render; compare the PDF kept beside it
instead.

## Layouts

**interleaved** (the default) copies the original pages unchanged, vectors
and text layer included, in the order 1A 1B … 1N, 2A 2B … 2N. A variant with
fewer pages gets blank pages of its own page size so every row stays aligned.

- With two variants the PDF asks viewers for a two page spread without a
  separate cover page (`/PageLayout /TwoColumnLeft`), so each spread is one row.
- PDF has no standard layout for three or more columns. Set the viewer to show
  n pages per row, for example Okular's Overview view mode with n columns, or
  use `--layout sheet`.

**sheet** composes one wide page per row with every variant's page placed side
by side under a small caption (`B · cluster/d-plus@en-ch:4 · page 2`). It
works in every viewer, prints, and suits three or more variants. The embedded
pages stay vector graphics.

## Labels and navigation

Every variant gets a column letter and a label derived from its item:
`<style>/<substyle>@<locale>:<preset>` for styles,
`<record path>@<locale>:<preset>` for records and the file name for PDFs.
Duplicate labels gain their column letter.

Interleaved pages carry PDF page labels such as `2·cluster/d-plus@en-ch:4`
and an outline with one entry per row (`Page 2`) and one child per variant,
so the original pages remain untouched. Sheets print the label above each page.

## Output

| Path | Content |
|---|---|
| `.agent/cache/comparisons/<name>.pdf` | the comparison |
| `.agent/cache/comparisons/<name>/<letter>-<label>.pdf` | each freshly rendered variant |
| `.agent/cache/comparisons/<name>/page-<n>.png` | with `--png`, one sheet preview per row |

`--name` must use lowercase letters, digits and hyphens. Without it the name is
derived from the labels, stating a shared locale and preset once, for example
`cv-harvard-d-plus-vs-cluster-d-plus-en-ch-4`. Each run replaces the previous
comparison of the same name. The same inputs produce the same bytes: no
timestamps and no random identifiers.

`--only 1-2` or `--only 1,3` keeps those page rows. `--png` writes sheet
previews at two pixels per point for inspection, whatever the layout; very
many variants can exceed the preview size limit.

The command prints the output path, the page order, each variant with its page
count and any blank rows, and a viewer hint. `--json` prints the same summary
as JSON for agents: `output`, `rows`, `pages`, `page_layout`, `viewer`,
`previews` and per variant `column`, `label`, `kind` (`style`, `record` or
`pdf`), `style`, `substyle`, `locale`, `preset`, `pages`, `blank_rows`, `pdf`
and, with `--metrics`, `metrics`.

`--metrics` adds a table with one column per rendered variant: measured lines,
lines outside their contract and the fill of each Summary line, followed by
each finding as `measure` reports it. PDF items show `n/a`.

## Examples

Two styles, opening pages only:

```sh
bash ./ccvl compare harvard/d-plus cluster/d-plus --only 1
```

Three substyles at four pages, as sheets with previews to inspect:

```sh
bash ./ccvl compare harvard/d-plus harvard/compact harvard/standard --pages 4 --layout sheet --png
```

An application against its kept variants, with measurements:

```sh
bash ./ccvl compare opportunities/acme/lead \
  opportunities/acme/lead/variants/harvard/d-plus/values \
  opportunities/acme/lead/variants/cluster/d-plus/draft --metrics
```

German against English:

```sh
bash ./ccvl compare harvard/d-plus@de-ch harvard/d-plus@en-ch
```

Two page presets of one substyle:

```sh
bash ./ccvl compare harvard/d-plus:2 harvard/d-plus:4
```

The current rendering against a PDF from an older build:

```sh
mkdir -p .agent/cache/old
git show HEAD~5:outputs/cvl/cv/harvard/d-plus/en/ch/pdf/cv-4.pdf > .agent/cache/old/cv-4.pdf
bash ./ccvl compare .agent/cache/old/cv-4.pdf harvard/d-plus:4
```

Opportunity PDFs are stored with Git LFS, so `git show` returns a pointer for
them; check out the older revision in a separate worktree and pass its PDF.

Two cover letter substyles, summarized for an agent:

```sh
bash ./ccvl compare --doc cl harvard/frame harvard/left-rule --json
```
