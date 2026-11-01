# Reusable Typst helpers

This directory contains neutral profile, application, measurement and explicit-settings
adapters. Each style chooses which helpers to import. Document geometry and
presentation code live under `cvl/<document>/<style>/`.

`application.typ` resolves a leaf's explicit shared wording and local overrides.
It makes no assumptions about a style's content fields. Its `load-application`
function also accepts a self-contained opportunity record.

`paper.typ` resolves a style-declared paper preset from a requested ID, the
record's document selection or the locale default. Its `paper-settings` helper
validates and merges adapter layers with the selected preset. Renderers use
the resolved label when printing a paper name.

The shipped Harvard styles share helpers in `cvl/shared/harvard/` and keep their
CV and letter renderers in their own `src/` folders. Harvard's application
adapter lives in `cvl/shared/harvard/application.typ`, because its summary and
paragraph rules are specific to that style.

The engine supplies the selected record, profile, locale, page count, strings and
substyle configuration as inputs. A style may also declare defaults and extra
fonts. See [../docs/styles.md](../docs/styles.md) for the complete interface and
[../../cvl/README.md](../../cvl/README.md) for the shipped folder structure.

Harvard's `standard`/`compact` CVs share a contract; `compact` changes vertical
spacing. Its `left-rule`/`frame` letters share a contract; `frame` changes the
highlight border. Other styles own different layouts and contracts.

`document.typ` optionally applies style-owned page, text and paragraph values.
It chooses no paper, font or locale. Shipped styles merge family defaults,
substyle overrides and leaf `layout.toml`; independent styles can use a wholly
different setup. See [the defaults audit](../docs/typst-defaults.md) for the
pinned Typst baseline, deliberate automatic behavior and export limitations.

## Correspondence

Letters import the cletter facade from the engine's generated module:
`#import "/.agent/typst/correspondence.typ": opening, salutation`. That file is
not in the repository. Every ccvl Typst engine registers it together with the
cletter, cnice, cdate and cink Typst packages embedded in the crates linked
into ccvl (`ccvl_core::render::with_correspondence`). Its single line glob
imports `@preview/cletter:<linked version>`; Typst exports imported names, so
styles never name a library version, and no library source is copied here.
Rendering stays offline.

The libraries own greeting and title parsing, closings, dates, signature
helpers, lowercase locale IDs and explicit orthography conventions. Harvard
owns its subject wording, English greeting overrides and composition. Change
correspondence behavior in its owning library first, then rerun ccvl's full
render gate.

Orthography conversion is explicit and limited to caller-selected prose.
Diagnostics never mutate input. Preserve names, exact quotes, URLs, source
material and explicit choices; a possible spelling match is not proof of error.

### Staying on the latest release

`Cargo.toml` and `.agent/core/Cargo.toml` require cletter and ctypst as semver
ranges (`0.4.2` means at least 0.4.2 and below 0.5). cletter in turn requires
cnice, cdate and cink as ranges, and `cletter::typst::packages()` pins its
Typst imports to whichever compatible releases Cargo resolved. To move to the
latest compatible releases, run `cargo update` (or
`cargo update -p cletter -p cnice -p cdate -p cink -p ctypst`), rebuild with
`bash ./ccvl setup --from-source`, run `bash ./ccvl check`, and commit
`Cargo.lock`. No Typst source changes. A new incompatible release line, such
as cletter 0.5, needs a deliberate requirement change.

While a required release is not yet on crates.io, the root `Cargo.toml` lists
its source repository under `[patch.crates-io]`. Cargo ignores that section
when ccvl-core is packaged, so its published manifest keeps plain ranges.
Remove each entry once its version is published and update the lock again.
