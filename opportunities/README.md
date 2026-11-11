# Opportunities

Every concrete job lives directly under a stable two-part key:

```text
opportunities/<organisation-key>/<position-key>/
├── application.toml
├── posting.md                 required human-readable position reference
├── research.md                optional attributable company and role research
├── interview-<stage>.md       optional preparation
├── submission.md              optional observed submission
├── outcome.md                 optional observed outcome
├── typst/
│   ├── CV_<Last>_<org>_<pos>.typ   resolved standalone copy (generated, do not edit)
│   └── CL_<Last>_<org>_<pos>.typ   only when the cover letter is enabled (generated)
├── pdfs/
│   ├── CV_<Last>_<org>_<pos>.pdf
│   └── CL_<Last>_<org>_<pos>.pdf   only when the cover letter is enabled
└── variants/                  optional variants that are not current
    └── <style>/<substyle>/    the variant's CV style, e.g. harvard/d-plus or cluster/d-plus
        └── <label>/           free label, e.g. values
        ├── application.toml   the record as it was; never built or checked
        └── *.pdf              its rendered documents
```

MECE split: `application.toml` holds the document build (options, tailored
wording, recipient) plus the role identity. Everything else about the
position — source, tasks, requirements, company context, preparation notes —
lives in `posting.md`. The build refuses records whose sibling `posting.md`
is missing.

The `typst/` copies are the locale templates with their `sys.inputs`
defaults resolved for the record (application and profile paths, page
count, style), so each compiles standalone and reproduces its neighbour
PDF. They are build artifacts: editing `application.toml` regenerates
both `typst/` and `pdfs/`; editing a locale template regenerates both as
well. Do not hand-edit `typst/` — re-run `build-opportunity`. `pdfs/` and
`typst/` are tracked in git (PDFs via LFS); commit whenever you feel like it.

Both keys use lowercase ASCII letters, numbers, hyphens, or underscores. The
single `application.toml` owns the options (language, pages, cover-letter
switch), the flowing Summary paragraph, optional six-paragraph cover letter,
and five highlights. Its
directory is the key; do not add generic `companies/`, `positions/`,
`applications/`, or market-map layers.

## Status list and contacts

`.agent/kanban.toml` lists the application phases and statuses with their
follow-up reminders. It ships with the CareerVector v4 defaults; a position
names its status by key in `application.toml`.

An organisation folder may also hold `contacts.toml`: the organisation, the
people known there, and the touchpoints with them, with field names taken from
vCard and iCalendar. An organisation folder may exist without any position.
Contacts are personal data about other people and belong only in a private
downstream. See [Contacts and application status](../.agent/docs/contacts.md)
for both formats.

Company, role, and fit research belongs here with its sources. General facts
or preferences learned about the user belong in `inputs/interviews/`.

Create the record deterministically, then ask the `ccvl-apply` skill to
research, draft, review, measure, and render it:

```sh
bash ./ccvl new-opportunity <organisation-key> <position-key>
bash ./ccvl build-opportunity <organisation-key> <position-key>
```

The public ccvl repository intentionally contains only this scaffold. Real
postings, research, recipient details, tailored documents, and outcomes belong
in a private downstream.
