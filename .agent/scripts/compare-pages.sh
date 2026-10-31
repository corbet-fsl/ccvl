#!/usr/bin/env bash
# Assemble selected pages of built PDFs into one comparison PDF with previews,
# for reviewing styles, substyles, locales or presets side by side.
#
#   bash .agent/scripts/compare-pages.sh <name> <pdf>[:<pages>] <pdf>[:<pages>] ...
#
# <pages> is a qpdf page range such as 1, 2-4 or 1,3 (default: every page).
# Build the inputs first; the result is review material in
# .agent/cache/comparisons/ and is never committed. Examples:
#
#   # Two styles: cluster opening against the chronological Harvard opening.
#   bash .agent/scripts/compare-pages.sh openings-en \
#     cvl/cv/cluster/d-plus/en/ch/pdf/cv-4.pdf:1 cvl/cv/harvard/d-plus/en/ch/pdf/cv-4.pdf:1
#   # Two substyles: Harvard D+ against compact, all four pages each.
#   bash .agent/scripts/compare-pages.sh harvard-dplus-compact-en \
#     cvl/cv/harvard/d-plus/en/ch/pdf/cv-4.pdf cvl/cv/harvard/compact/en/ch/pdf/cv-4.pdf
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
for tool in qpdf pdftoppm; do
  command -v "$tool" >/dev/null || { echo "$tool is required for page comparisons." >&2; exit 2; }
done
if (($# < 3)); then
  echo "Usage: compare-pages.sh <name> <pdf>[:<pages>] <pdf>[:<pages>] ..." >&2
  exit 2
fi
name="$1"
shift
[[ "$name" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "Name must be lowercase letters, digits and hyphens: $name" >&2; exit 2; }
selection=()
for spec in "$@"; do
  pdf="${spec%%:*}"
  pages="1-z"
  if [[ "$spec" == *:* ]]; then pages="${spec#*:}"; fi
  [[ -f "$pdf" && "$pdf" == *.pdf ]] || { echo "Not a built PDF: $pdf" >&2; exit 2; }
  [[ -n "$pages" ]] || { echo "Empty page range in $spec" >&2; exit 2; }
  selection+=("$pdf" "$pages")
done
output=".agent/cache/comparisons"
mkdir -p "$output"
qpdf --deterministic-id --empty --pages "${selection[@]}" -- "$output/$name.pdf"
qpdf --check "$output/$name.pdf" >/dev/null
rm -f -- "$output/$name"-*.png
pdftoppm -scale-to 1600 -png "$output/$name.pdf" "$output/$name"
printf 'Comparison: %s (%s pages)\n' "$output/$name.pdf" "$(qpdf --show-npages "$output/$name.pdf")"
