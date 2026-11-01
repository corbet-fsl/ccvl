# Third-party notices

ccvl bundles Archivo, EB Garamond, IBM Plex Serif, and Source Serif 4 font
files. They are separate works distributed under the SIL Open Font License 1.1.
Detailed copyright and reserved-font-name notices are in
`.agent/typst/fonts/NOTICE.md`; the license text is available in both
`.agent/typst/fonts/OFL-1.1.txt` and `LICENSES/OFL-1.1.txt`.

The native ccvl binary links the Typst and Typstyle engines from locked Rust
dependencies and embeds the bundled font files. Rust and Cargo are required to
build that binary. Poppler, QPDF, Git, curl, and ripgrep are external tools
and are not redistributed by this repository. The bootstrap installs the exact
Rust toolchain only after the user requests setup, then builds ccvl from the
frozen `Cargo.lock` dependency graph.

The binary also links the correspondence libraries cletter, cnice, cdate and
cink, licensed LGPL-3.0-only WITH LGPL-3.0-linking-exception, as locked crates.
Those crates embed the Typst packages behind letter openings, closings, dates
and signatures; ccvl registers them in memory and keeps no copy of their
sources in this repository.
