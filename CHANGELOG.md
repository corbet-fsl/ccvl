# Changelog

## 0.1.13

- **URL rule version 3** (`ccvl::URL_RULE_VERSION`, `ccvl::url_rules`). `normalize_url` now also drops query parameters and path
  parameters that carry personal data or credentials, before a job or ad id is computed:
  - a value that contains an email address, or that is base64 (16+ characters, printable result) of text containing one;
  - names that contain `token`, `secret`, `passw`, `session`, `signature`, `credential`, `apikey` or `api_key`, equal `auth` or
    `authorization`, end in `sig` or `hash`, or start with `otp`;
  - a small table of host-scoped alert parameters (LinkedIn, Xing, Indeed, StepStone);
  - `;` separates query parameters like `&`, and path parameters (`/job/1;jsessionid=X`) follow the same rules.
  A job-id allow-list (`jk`, `vjk`, `id`, `jid`, `key`, `uid`, `sid`, `currentJobId`, `jobId`, `gh_jid`, `jobListingId`) is never
  dropped by name. The lists are data at the top of `src/url_rules.rs` and are pinned to `tests/fixtures/normalize-url.json`,
  which the TypeScript twin (`@cv/jobcache/ids`) reads too.
- New: `is_personal_url` (an email in a path segment or in the userinfo makes a URL personal as a whole; nothing is rewritten) and
  `contains_email`.
- Ids change for URLs that carry such parameters. That is the intent.

## 0.1.12 (yanked)

- Shipped a first, narrow personal-data rule (URL rule version 2). **Superseded by rule version 3 in 0.1.13 and must not be
  used.** It was yanked on 2026-11-21; no id was ever computed with it in a product.

## 0.1.11

- `job.update` carries an optional `unset` list; op catalogue version 11; vectors regenerated.
