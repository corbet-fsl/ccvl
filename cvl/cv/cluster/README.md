# Cluster CV — D+

The cluster CV opens with a clustered first page and continues with the
unchanged Harvard D+ pages 2–4. It ships in the Harvard presets: two, three
and four pages. Pages 1 and 2 are identical across all three presets.

The first page groups seven approved Harvard stations into three categories
with **2–3–2 stations**, each with three bullets: Investments & Corporate
Finance; Cloud, AI & Transformation; and Strategy & Innovation Consulting.
Stations follow reverse chronological order within each category, except that
professional NEXGEN work leads the student consulting entry. Their headings,
roles and bullets are copied unchanged from the Harvard showcase; the
five-line summary leads with recent finance and AI results from those
stations. The summary is justified; bullets retain natural word spacing and a
ragged right edge.

The opening uses Archivo and the Harvard hierarchy. Approximately 3.3 mm of
visible space surrounds each category heading; smaller gaps distinguish roles,
bullets and stations. Text is never shortened to fit.

| Locale | 2 pages | 3 pages | 4 pages |
| --- | --- | --- | --- |
| de-ch | [D+](d-plus/de/ch/pdf/cv-2.pdf) | [D+](d-plus/de/ch/pdf/cv-3.pdf) | [D+](d-plus/de/ch/pdf/cv-4.pdf) |
| en-ch | [D+](d-plus/en/ch/pdf/cv-2.pdf) | [D+](d-plus/en/ch/pdf/cv-3.pdf) | [D+](d-plus/en/ch/pdf/cv-4.pdf) |

```sh
bash ./ccvl build-cv en-ch 4 --style cluster --substyle d-plus
```

Pages 2–4 come from `../harvard/src/entries-*.typ` through Harvard's
`render-continuation`, so they change only when Harvard changes. Because those
entries are candidate source files, the cluster style, like Harvard, is not
exported as a reusable bundle. The
[Harvard D+ substyle](../harvard/d-plus/README.md) remains available as the
chronological CV and is the workspace's default CV.

Earlier geometry samples remain available with Lorem Ipsum openings and the
same continuation: `standard` has 2–2–2 stations, `middle-three` has 2–3–2,
and `middle-three-spaced` balances the category gaps. D+ is the cluster
default. Fonts, wording and line counts affect visible gaps: remeasure after
changing them rather than treating these calibrated values as universal
constants.
