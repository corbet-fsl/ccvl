# Profile interview and station allocation

The profile interview solves layout underfill by finding truthful material
before the CV is written. Pagination cannot make a sparse career record look
full. The system therefore maintains a private working profile and journal,
then applies a deterministic station gate to the publishable CV plan.

## Working artifacts

| Artifact | Purpose | Publication status |
|---|---|---|
| `interview/imports/` | read-only source-document inbox | private and ignored |
| `interview/profile.md` | informal, source-linked working portrait | private and ignored |
| `interview/journal.md` | visible interview progress and deferred questions | private and ignored |
| `interview/stations.toml` | selected and unassigned station candidates | private working state |
| `cvl/cv/<style>/<substyle>/<lang>/ch/` and `cvl/cl/<style>/<substyle>/<lang>/ch/` | approved rendered wording and presentation | publishable candidate data |

The journal is written before the next question is asked. A user can inspect it
at any time, paste information directly into it, or provide more documents.

## Fixed layout contract (Harvard CV)

A station is a full visual CV entry with its own heading, context or period,
and supporting content. Individual bullets and compact standalone lines do not
count. The counts below are the Harvard family contract, shared by its
`standard` and `compact` CV substyles — not engine law for a future second
family.

| Page | Role | Fixed structure |
|---|---|---|
| 1 | experience | 6–8 stations; target 7 |
| 2 | supporting record across useful sections | exactly 10 stations × 2 bullets |
| 3 | projects and initiatives | exactly 10 entries × 2 bullets |
| 4 | competencies | exactly 3 groups × 3 blocks × 3 keyword lines |

Only verified stations with a page and one section count. Nine page-1 stations
are overcrowded; fewer than six are underfilled. Every other page count and
listed per-entry line count is exact. A failed bound triggers another interview
or allocation pass.

A selected station uses this neutral shape:

```json
{
  "id": "inventory-automation",
  "label": "Independent inventory automation",
  "anchor": "Independent project | 2024–present",
  "kind": "project",
  "status": "verified",
  "page": 1,
  "section": "experience",
  "experience_eligible": true,
  "facts": [
    { "id": "inventory-time", "text": "Reduced recurring manual inventory work" }
  ],
  "source_refs": ["user-confirmed:YYYY-MM-DD"]
}
```

An unresolved or surplus candidate uses `page: null` and an empty `section`.
It remains available for later tailoring without appearing twice.

Pages 3 and 4 have their own top-level tables, `[[projects]]` and
`[[competencies]]`, each entry with an `id`, a `label`, an optional `anchor`,
`facts` and `source_refs`. Fact IDs are unique across all three tables.

When the plan mirrors an approved CV, each entry uses the same ID as its CV
marker and these fact IDs: `<id>-role` for the subheading (`#cv-s[...]`) and
`<id>-1`, `<id>-2`, … for the bullets (`#cv-b[...]`) in order. Any other fact,
such as `<id>-scope`, is free context and is not compared with the CV. A
project without a `<id>-role` fact compares its `anchor` with the subheading.

### Source references

Each entry carries `source_refs`; a fact may carry its own, which then
replaces the entry's for that fact. The recognised forms are:

| Form | Meaning |
|---|---|
| `user-confirmed:<YYYY-MM-DD>` | the candidate confirmed it in the interview |
| `author-confirmed:<YYYY-MM-DD>` | the showcase author, who is the candidate, confirmed it |
| `confirmed-by:<person>:<YYYY-MM-DD>` | someone other than the candidate confirmed it |
| `cv:<path>` | it is printed in the approved CV source at that path |

Other references, such as an imported document path, are evidence too. Use
`confirmed-by` whenever the person recording a fact is not the candidate:
`author-confirmed` and `user-confirmed` always mean the candidate's own
word. A malformed `confirmed-by` reference is an error.

### Claims that are not true

The optional `[[not_true]]` list records things the candidate must never be
presented as having, typically after a correction:

```toml
[[not_true]]
term = "SAP"
note = "No professional SAP or ERP experience"

[[not_true]]
term = "ERP"
note = "No professional ERP experience; the own mini ERP is the only ERP work"
except = ["mini-ERP", "mini ERP"]
```

`term` matches as a whole word and ignores case unless `case_sensitive =
true`. `except` lists phrases inside which the term is legitimate; they ignore
case in the same way. `note` is the one-line reason every hint repeats.
Duplicate terms and unknown fields are errors.

## Deterministic state machine

1. **Protect:** determine whether personal data would be published and warn
   before writing to a public destination.
2. **Ingest:** inventory attached, pasted, and `interview/imports/` sources; preserve
   them read-only and cross-reference conflicts.
3. **Capture:** ask one broad question, save the answer, and create candidate
   stations and atomic facts without prematurely choosing sections.
4. **Deepen:** obtain enough detail for each promising station: period,
   context, responsibility, action, result or scale, tools, and provenance.
5. **Measure:** run `ccvl profile-status`; select the next question from the
   underfilled page and unresolved high-value candidates.
6. **Revisit:** treat “nothing else” as exhaustion of one prompt, not proof that
   no other experience exists. Change topic, then return once with specific
   memory cues. Respect an explicit request to stop and report the underfill.
7. **Allocate:** give every station one page and section; give every atomic fact
   one station owner. Rank surplus material and leave it unassigned for future
   tailoring.
8. **Verify:** compare the plan with every locale source in each selected
   substyle, render, measure, and run the complete workspace check.

## Question scheduler

The scheduler starts with “What experiences have you had?” and follows the
user's answer rather than presenting a long questionnaire. After each answer it
chooses one of four actions:

- deepen a promising station that lacks responsibility, result, or provenance;
- ask for another experience while page 1 is below six;
- change to page-2 material when the user is fatigued or page 1 is ready;
- revisit page 1 later with cues drawn from the working profile.

Recall cues cover paid and unpaid roles, internships, temporary work,
independent work, side ventures, repairs, products, open source, sustained
hobbies, research, teaching, mentoring, leadership, volunteering, communities,
events, and substantial personal responsibility. Page-2 prompts cover
education, professional development, credentials, publications, awards, and
other supporting milestones. Page-3 prompts identify ten distinct projects and
initiatives that can support two verified bullets each. Page-4 synthesis groups
evidenced tools, methods, and domains into exactly three MECE groups of three
blocks, each with three keyword lines.

The method is ambitious about finding value and conservative about factual
scope. Independent work may become an Experience station if it demonstrates
real responsibility and output, but its nature remains independent work. It is
never relabelled as employment, customers, formal business activity, adoption,
or revenue without evidence.

## MECE allocation

Facts are collected before presentation buckets are finalised. Each fact has a
stable ID, and duplicate ownership is a validation error. A station records a
heading label, a context-or-period anchor, and its truthful `kind` separately
from its selected `section`, allowing economical
placement without changing what happened. Moving is allowed; copying is not.

Run the gate at any time:

```sh
bash ./ccvl profile-status
bash ./ccvl profile-status --verify-sources
```

The first command reports station coverage and targeted next prompts. The
second proves that every selected locale source contains the planned page-1 and page-2
station IDs, exactly ten two-bullet projects, and the same 3×3 competency
structure. Entries use `// ccvl-station: <station-id>`, `// ccvl-project:
<project-id>`, or `// ccvl-competency: <competency-id>` directly before their
`#cv-h[...]`; compact lines have no marker.

To read the whole interview at once, print it as Markdown:

```sh
bash ./ccvl profile-status --list
```

It lists every station with its role line and facts, then the projects, the
competencies and the `not_true` list, in file order.

## Hints and acknowledgements

Hints work like a linter. They show what deserves attention; they never fail
a command, and a deviation is allowed. Each hint is one `WARN` line with a
precise location and a one-line reason. Only structural problems stay errors:
an unparsable file, a duplicate ID, the schema rules above, or a malformed
acknowledgement.

A deliberate deviation is acknowledged in the data with a short written
reason. The reason silences that hint and keeps the decision reviewable in
the file's history. An acknowledgement without a reason is an error.

| Hint | Printed by | Acknowledgement |
|---|---|---|
| interview fact differs from, is missing in, or has no counterpart in the CV | `profile-status --verify-sources`, `check` | `allow_drift = "<reason>"` on the fact, or on the entry for all of its lines |
| a CV entry has no interview entry | `profile-status --verify-sources`, `check` | add the entry |
| a `not_true` term occurs in application wording, `cvl/` sources or the plan | `check`; also `measure-opportunity` and `build-opportunity` for that record | `[lint] allow` in the application record; `except` phrases in the rule |
| an entry or fact is confirmed only by someone other than the candidate | `profile-status` | the candidate reviews it and adds `user-confirmed:<date>` |
| a figure in application wording occurs in no interview fact | `check --figures` only | `[lint] allow` in the application record |

The drift comparison reads the English source among the style's
`source_files` (a file named `en` or ending in `-en`, such as
`entries-en.typ`) and compares text, not markup: `#brand[X]` reads `X`, `\>`
reads `>`, `*bold*` loses its stars, and spacing and apostrophes are
normalised. It runs once the plan has stations. Plan entries that the CV does
not mark are not reported, because the plan may hold surplus material.

The `not_true` scan covers the `[cv]` and `[cl]` tables of
`opportunities/**/application.toml` (skipping `variants/` and `review/`
folders), the `[cv]` and `[cl]` tables of `cvl/` TOML files, every
non-comment line of `cvl/` Typst sources, and the plan's own labels, anchors
and facts. An application record acknowledges a match for the employer's own
context rather than the candidate's experience:

```toml
[lint]
allow = [
  { term = "SAP", reason = "names the employer's SAP environment, not my experience" },
]
```

An acknowledged term that no longer occurs in that record is reported as a
stale acknowledgement, so the list stays short.

Figures are opt-in because rephrased or converted figures can differ from
their source wording. `ccvl check --figures` reads quantified claims in the
`[cv]` and `[cl]` wording of application records: numbers with a currency,
magnitude, `%`, `+`, `×` or `years`, and written magnitudes such as
`eight-figure` or `achtstellig`. A figure is backed when its number occurs in
any interview fact or anchor (`1'000+` and `1,000` both read `1000`). Bare
numbers, years and grades are ignored. Acknowledge a figure taken from the
posting with `{ figure = "40%", reason = "the employer's own target" }`.
