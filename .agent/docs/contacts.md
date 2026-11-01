# Contacts and application status

Scope: the data format of `opportunities/kanban.toml` (the application status
list) and of `opportunities/<organisation-key>/contacts.toml` (people and
touchpoints of one organisation), their field mappings to CareerVector,
vCard, iCalendar and CSV, identifier and value rules, privacy, and the planned
commands. Schemas: [`kanban.schema.json`](../schemas/kanban.schema.json) and
[`contacts.schema.json`](../schemas/contacts.schema.json). Validation code
does not exist yet; the schemas and this page are the contract it will
implement.

```text
opportunities/
├── kanban.toml                status list for this workspace, editable
└── <organisation-key>/
    ├── contacts.toml          optional; the organisation, its people, touchpoints
    └── <position-key>/        optional; an organisation may have no positions
        └── application.toml   references a status key and contact ids
```

## Application status list

`opportunities/kanban.toml` defines phases, statuses and follow-up reminders
for every position of the workspace. ccvl ships the CareerVector v4 defaults;
a workspace may rename, recolour, reorder, add or remove entries. Numeric ids
and field semantics are identical to CareerVector so a workspace can be
exchanged 1:1. Each phase and status also carries a readable `key`, which is
what positions reference.

```toml
schema_version = 1

[[phases]]
id = 1
key = "discovery"
name = "Discovery"
order = 1
color = "#94a3b8"
requires_applied_date = false
is_terminal = false

[[statuses]]
id = 5
key = "to-apply"
name = "To Apply"
phase_id = 2
order = 3
# sets_applied_date = true   only where entering the status records applied_at

[reminders]
enabled = true
default_interval_days = 7
escalation = [7, 14, 28, 56]

[reminders.phase_rules.1]    # keyed by phase id
enabled = true
default_days = 14
```

### Field mapping to CareerVector

The TOML file uses snake_case throughout. `phases` and `statuses` map to
CareerVector `settings.kanban`, `reminders` to `settings.reminders`.

| ccvl TOML | CareerVector v4 | Meaning |
|---|---|---|
| `phases[].id` | `kanban.phases[].id` | numeric id, unique, stable once used |
| `phases[].key` | none (ccvl only) | readable slug, unique among phases |
| `phases[].name` | `kanban.phases[].name` | display name |
| `phases[].order` | `kanban.phases[].order` | column order |
| `phases[].color` | `kanban.phases[].color` | `#rrggbb` |
| `phases[].requires_applied_date` | `kanban.phases[].requiresAppliedDate` | positions in this phase must carry `applied_at` |
| `phases[].is_terminal` | `kanban.phases[].isTerminal` | the process has ended |
| `statuses[].id` | `kanban.statuses[].id` | numeric id, unique, stable once used |
| `statuses[].key` | none (ccvl only) | readable slug, unique among statuses |
| `statuses[].name` | `kanban.statuses[].name` | display name |
| `statuses[].phase_id` | `kanban.statuses[].phaseId` | id of the owning phase |
| `statuses[].order` | `kanban.statuses[].order` | order within the phase |
| `statuses[].sets_applied_date` | `kanban.statuses[].setsAppliedDate` | entering the status records `applied_at` when empty |
| `reminders.enabled` | `reminders.enabled` | follow-up reminders on or off |
| `reminders.default_interval_days` | `reminders.defaultIntervalDays` | interval for phases without a rule |
| `reminders.escalation` | `reminders.escalation` | days after the nth snooze; the last value repeats |
| `reminders.phase_rules.<phase id>` | `reminders.phaseRules[<phase id>]` | rule on entering that phase |
| `reminders.phase_rules.<id>.enabled` | `phaseRules[<id>].enabled` | rule on or off |
| `reminders.phase_rules.<id>.default_days` | `phaseRules[<id>].defaultDays` | days until `follow_up_at` |

Defaults where a field is omitted match CareerVector: booleans default to
`false` except `enabled` (`true`), `default_interval_days` and `default_days`
to 7, `escalation` to `[7, 14, 28, 56]`. ccvl is stricter in two places: ids,
orders and day counts must be integers (ids from 1, orders from 0), and
`escalation` must not be empty. `schema_version` and `key` have no
CareerVector counterpart; an import into CareerVector drops them, an export
from CareerVector derives keys from the names (lowercase, words joined by
hyphens).

The default keys are `discovery`, `due-diligence`, `application`,
`their-court` and `resolution` for phases, and `spotted`, `maybe`,
`researching`, `networking`, `to-apply`, `drafting`, `submitted`, `waiting`,
`following-up`, `interview`, `offer` and `closed` for statuses. Keys are
values, never folder names, so hyphens are allowed.

Runtime validation will check what JSON Schema cannot express: unique ids and
keys, that every `phase_id` and every `phase_rules` key names an existing
phase.

### Positions

A position records its state in `application.toml` with the CareerVector job
field names. These fields are the intended reference and are not yet part of
the record schema:

| ccvl `application.toml` | CareerVector job | Rule |
|---|---|---|
| `status = "<status key>"` | `status_id` (numeric) | key must exist in `kanban.toml`; exchange maps through the status id |
| `applied_at = "YYYY-MM-DD"` | `applied_at` | required while the status is in a phase with `requires_applied_date` |
| `follow_up_at = "YYYY-MM-DD"` | `follow_up_at` | next follow-up; computed from `phase_rules` or `escalation` unless set by hand |

CareerVector also keeps `snooze_count` and `follow_up_manual` on the job. ccvl
does not adopt them yet; an export writes `0` and `false`.

## Contacts

Each organisation may have one `opportunities/<organisation-key>/contacts.toml`.
The organisation folder may exist without any position, for example to track
a company met at a fair before a vacancy exists. The file describes the
organisation itself, the people known there, and the history of touchpoints
with them. Field names follow vCard 4.0 (RFC 6350) for the organisation and
people, and iCalendar (RFC 5545) for meetings and follow-ups.

Example with fictional data only (`example.invalid` is a reserved domain; no
real person or company is meant):

```toml
# FICTIONAL EXAMPLE. Not a real organisation or person.
schema_version = 1

[organisation]
name = "Example Analytics AG"
url = "https://example.invalid"
country = "CH"
notes = "Met at a fictional career fair."
source = { kind = "event", name = "Example Career Fair", date = "2026-10-08" }

[[contacts]]
id = "alex-sample"
name = "Alex Sample"
given = "Alex"
family = "Sample"
title = "Head of Data"
role = "Hiring manager"
email = ["alex.sample@example.invalid"]
phone = "+41000000000"
url = "https://example.invalid/people/alex-sample"
languages = ["de-ch", "en"]
notes = "Prefers short emails."
source = { kind = "event", name = "Example Career Fair", date = "2026-10-08" }

[[touchpoints]]
id = "fair-chat"
date = "2026-10-08"
contact = "alex-sample"
channel = "event"
direction = "outgoing"
summary = "Talked about the data platform team."
next = "Send CV and ask about open roles."
due = "2026-10-15"
source = { kind = "event", name = "Example Career Fair", date = "2026-10-08" }

[[touchpoints]]
date = "2026-10-20"
contact = "alex-sample"
channel = "meeting"
direction = "incoming"
summary = "Invited to an informal call."
start = "2026-10-22T10:00:00+02:00"
end = "2026-10-22T10:30:00+02:00"
```

### Fields

`[organisation]` (required): `name` (required), `url`, `country`, `notes`,
`source`. The organisation key is the folder name and is not repeated.

`[[contacts]]` (optional, one per person): `id` and `name` required;
`given`, `family`, `title`, `role`, `email`, `phone`, `url`, `languages`,
`country`, `notes`, `source` optional. `email`, `phone` and `url` take one
string or a list, preferred first.

`[[touchpoints]]` (optional, one per interaction): `date`, `channel`,
`direction` and `summary` required; `id`, `contact`, `next`, `due`, `done`,
`start`, `end`, `source` optional. `contact` is one contact id or a list;
absent means the organisation as a whole (for example a careers inbox).
`next` is the agreed next step, `due` its deadline and `done` the date it was
completed; `due` and `done` require `next`. `start` and `end` describe a
scheduled meeting; `end` requires `start`.

`source = { kind, name, date, url }` records where an organisation, a contact
or a touchpoint came from. `kind` is required and one of `event`, `linkedin`,
`email`, `referral`, `posting`, `website`, `other`. Events are never entities
of their own: a fair or meetup appears only as a `source` (with `kind =
"event"` and its name and date) or as `channel = "event"`. Material about an
event belongs to research, not to contacts.

### vCard, iCalendar and CSV mapping

| ccvl field | vCard 4.0 | iCalendar | CSV column |
|---|---|---|---|
| organisation key (folder) | `UID` of the org card | | `organisation` |
| `organisation.name` | `KIND:org`, `FN`, `ORG` | | `organisation_name` |
| `organisation.url` | `URL` | | `organisation_url` |
| `organisation.country` | `ADR` country component | | `organisation_country` |
| `organisation.notes` | `NOTE` | | |
| `contacts[].id` | `UID` (from the global id) | `ATTENDEE` lookup | `id` (global id) |
| `contacts[].name` | `KIND:individual`, `FN` | `ATTENDEE;CN=` | `name` |
| `contacts[].given`, `family` | `N` (family;given;;;) | | `given`, `family` |
| organisation of the contact | `ORG` | | `organisation_name` |
| `contacts[].title` | `TITLE` | | `title` |
| `contacts[].role` | `ROLE` | | `role` |
| `contacts[].email` | `EMAIL` (first gets `PREF=1`) | `ATTENDEE:mailto:` | `email` |
| `contacts[].phone` | `TEL;VALUE=uri:tel:+…` | | `phone` |
| `contacts[].url` | `URL` | | `url` |
| `contacts[].languages` | `LANG` (`PREF` by position) | | `languages` |
| `contacts[].country` | `ADR` country component | | `country` |
| `contacts[].notes` | `NOTE` | | `notes` |
| `source` | `X-CCVL-SOURCE` extension | `X-CCVL-SOURCE` extension | `source_kind`, `source_name`, `source_date`, `source_url` |
| `touchpoints[].id` | | `UID` | |
| `touchpoints[].date` | | `VJOURNAL` `DTSTART;VALUE=DATE` | |
| `touchpoints[].channel` | | `CATEGORIES` | |
| `touchpoints[].direction` | | `CATEGORIES` | |
| `touchpoints[].summary` | | `SUMMARY` (`DESCRIPTION` of a VTODO) | |
| `touchpoints[].next` | | `VTODO` `SUMMARY` | |
| `touchpoints[].due` | | `VTODO` `DUE;VALUE=DATE` | |
| `touchpoints[].done` | | `VTODO` `COMPLETED`, `STATUS:COMPLETED` | |
| `touchpoints[].start` | | `VEVENT` `DTSTART` | |
| `touchpoints[].end` | | `VEVENT` `DTEND` | |

Every touchpoint exports as a `VJOURNAL`. One with `start` also exports as a
`VEVENT`, one with `next` also as a `VTODO`; the three share `RELATED-TO`.
A position's `follow_up_at` exports as a `VTODO` with `DUE` as well. The CSV
export has one row per contact with a header row (RFC 4180, UTF-8); list
values are joined with `"; "`. vCard and iCalendar UIDs are
`urn:ccvl:<kind>:<global id>`; a touchpoint without `id` gets a UID derived
from its organisation key, date and position in the file, which changes if
earlier touchpoints are inserted, so set `id` where calendar updates must stay
stable.

### Identifiers

- A contact `id` uses the rule of organisation and position keys: lowercase
  ASCII letters and digits in words joined by single hyphens or underscores.
  It is unique within its file, for example `alex-sample`.
- The global id is `<organisation-key>/<id>`, for example
  `example-analytics/alex-sample`. It is the identity used across files and in
  exports.
- Ids are stable. Do not rename an id once anything references it; correct
  the `name` instead. When a person changes employer, add a new entry under
  the new organisation and mention the old global id in `notes`.
- Touchpoint `id` is optional and unique within its file.
- A position may reference a contact, for example as the cover letter
  recipient: `job.cl_recipient.contact = "<id>"`. A bare id resolves against
  the position's own organisation; a global id may point to another
  organisation (for example a recruiting agency). This is the intended
  reference; the record schema does not carry it yet, and the existing
  `cl_recipient` fields remain the printed source of truth.

### Value rules

- Dates are quoted ISO 8601 calendar dates, `"YYYY-MM-DD"`, matching
  `options.application_date`. Meeting times are quoted ISO 8601 date and time
  with seconds and an explicit offset (RFC 3339), for example
  `"2026-10-22T10:00:00+02:00"`.
- Phone numbers are E.164: a plus sign, country code and number, digits only,
  at most 15 digits, for example `"+41000000000"`. Keep formatting for display
  out of the data.
- Languages are BCP 47 tags in lowercase, like every locale in ccvl, for
  example `"de-ch"` or `"en"`.
- Countries are ISO 3166-1 alpha-2 codes in uppercase, for example `"CH"`.
- URLs are absolute `http` or `https` URLs; email addresses are plain
  addresses without display names.

## Privacy

Contacts are personal data about other people who never agreed to appear in a
public repository. `contacts.toml` files, touchpoints, recipient details and
any notes about people belong only in a private downstream (see
[Private downstreams](private-downstream.md) and the [privacy
model](privacy.md)). The public ccvl repository ships the schema, this page and
the default `kanban.toml`, never real contacts. Examples use the reserved
`example.invalid` domain and are marked fictional. Record only what the
application process needs, and delete entries that are no longer needed.

## Planned commands

- `ccvl contacts` prints an overview: open next steps from touchpoints (with
  `due` and without `done`) and positions whose `follow_up_at` is due, sorted
  by date, overdue first.
- `ccvl contacts export --format vcard|ics|csv` writes the mapping above:
  vCard 4.0 cards for organisations and people, an iCalendar file with
  journals, meetings and follow-ups, or a CSV table of contacts.

Both commands, and the validation of `kanban.toml`, `contacts.toml` and the
position fields, are not implemented yet.
