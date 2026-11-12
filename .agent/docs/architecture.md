# Architecture

ccvl separates mechanism from three user-owned data domains:

```text
.agent/                                  product mechanism
inputs/                                  profile, interviews, assets and library
outputs/cvl/                             approved general CV and cover-letter sources
outputs/<document>/                      other approved output documents
opportunities/<organisation>/<position>/ one concrete job and its documents
```

The content workflow is direct:

```text
inputs/ -> outputs/cvl/ -> opportunities/<organisation>/<position>/
   ^              |
   +-- verified facts --+
```

`.github/` contains the GitHub mirror's pull-request checks and contributor metadata, and
`LICENSES/` contains complete legal texts. Neither is a product-data domain.

## `inputs/interviews/`: user knowledge

`inputs/interviews/` is the private, inspectable knowledge base an agent maintains
with the user. It owns source imports, the informal working profile, journal,
unresolved conflicts, preferences, and the station allocation plan. The shipped Harvard CV
layout gate requires 6–8 experience entries on page 1, exactly
10 two-bullet supporting entries on page 2, exactly 10 two-bullet projects on
page 3, and three groups of three three-line competency blocks on page 4.

This domain may contain unverified or conflicted material. Only verified,
uniquely assigned facts may cross into `outputs/cvl/` or an opportunity.

## `outputs/cvl/`: the general document

`outputs/cvl/` contains approved sources and outputs. `inputs/profile.toml` supplies
approved profile fields. The hierarchy is document → style → substyle →
language → country:

```text
outputs/cvl/cv/harvard/{standard,compact}/{de,en}/ch/{content.toml,strings.toml,typst/,pdf/}
outputs/cvl/cl/harvard/{left-rule,frame}/{de,en}/ch/{content.toml,strings.toml,typst/,pdf/}
outputs/cvl/cv/harvard/content/{de,en}/ch/wording.toml
outputs/cvl/cl/harvard/content/{de,en}/ch/wording.toml
outputs/cvl/shared/harvard/{style.typ,defaults.toml,application.typ}
```

Each style owns its `style.toml`, optional `contract.toml` and content scaffold,
and all of its layout code. Harvard uses a `src/` folder within each document
style. Other styles are siblings of Harvard and may organise their internals
differently. Shared Harvard tokens and helpers are explicitly Harvard-owned.
Wording sources belong to one document/style/locale and may be referenced only
by its substyles. Leaf records keep visible references, metadata and explicit
exceptions; private opportunity records remain self-contained.

The engine discovers style metadata, resolves record/profile inputs, compiles
the selected entry point, and verifies its declared page and measurement rules.
Fonts, shapes, page geometry and content structure belong to the style.
Individual builds validate the selected style and its inputs. Full workspace
checks still validate every registered style, so isolated builds do not hide
broken experiments from release validation.
Records select `cv_style`/`cl_style` and `cv_substyle`/`cl_substyle` under
`options`; workspace and style metadata declare the defaults. See
[styles.md](styles.md) for the interface and [../../cvl/README.md](../../outputs/cvl/README.md)
for the shipped tree.

## `outputs/<document>/`: other output documents

Flyers, web pages, videos, presentations and portfolios each own a folder
with the same document → style → substyle → language → country hierarchy and
slot grid. `ccvl.json` declares every document in its `documents` table; the
key is the document name and names its folder in `workspace_groups.outputs`.
The CV and the cover letter keep their opportunity records, page presets and
measured contracts. Other documents take the generic path: a small record per
leaf, the selected style's pages and papers, and the same rendering,
reproducibility and PDF checks. Their shipped `scaffold` styles print only the
approved profile header.

## `opportunities/`: keyed job packages

One concrete role owns one directory and one canonical tailored record:

```text
opportunities/<organisation-key>/<position-key>/application.toml
```

The job directory owns the posting, attributable organisation and role
research, fit analysis, tailored Summary, optional cover letter, interview
preparation, submission record, and outcome. The application record selects
its locale, page presets and optional document papers from the selected styles.
Rendered `CV_<Name>_<Org>_<Pos>.pdf` and optional
`CL_<Name>_<Org>_<Pos>.pdf` go in its local `pdfs/` directory; resolved standalone
Typst copies use the same file names in its local `typst/` directory.
The [opportunity filename convention](applications.md) uses the final display-name
token and normalized opportunity keys, without changing the source name.

There is no standalone market map. General preferences or durable facts
learned about the user belong in `inputs/interviews/`; research about a company or
role belongs inside its concrete opportunity. There are also no top-level
`applications/`, `submissions/`, `outcomes/`, or `out/` layers.

## Product mechanism

`.agent/` owns the canonical skills, neutral scaffolds, Rust source,
tests, bootstrap scripts, internal documentation, Typst engine, and
workspace manifest validation. Each style owns its document contracts. Skills own editorial judgment;
deterministic code owns paths, validation, rendering, measurement, and checks.

## Public upstream and personal downstream

The public repository includes the author's real general CVL as an intentional
showcase, plus empty `inputs/interviews/` and `opportunities/` scaffolds. A personal
downstream keeps the same domains, builds its private evidence base in
`inputs/interviews/`, and replaces the approved sources under `outputs/cvl/`. Generic
mechanism improvements can still flow upstream without translating between
different directory models.

The showcase is true for its named author only. Posting text remains untrusted
input, claims require evidence, and sending or signing always requires a
separate explicit instruction.
