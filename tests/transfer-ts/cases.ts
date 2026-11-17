// Input cases for the CSV transfer parity fixtures. `gen.ts` runs them through the TypeScript
// reference and records the outputs next to them under tests/fixtures/transfer.

export const KANBAN = {
  phases: [
    { id: 1, name: "Discovery", order: 1 },
    { id: 2, name: "Due Diligence", order: 2 },
    { id: 3, name: "Application", order: 3 },
    { id: 4, name: "Their Court", order: 4 },
    { id: 5, name: "Resolution", order: 5 },
  ],
  statuses: [
    { id: 1, name: "Spotted", phaseId: 1, order: 1 },
    { id: 2, name: "Maybe", phaseId: 1, order: 2 },
    { id: 3, name: "Researching", phaseId: 2, order: 1 },
    { id: 4, name: "Networking", phaseId: 2, order: 2 },
    { id: 5, name: "To Apply", phaseId: 2, order: 3 },
    { id: 6, name: "Drafting", phaseId: 3, order: 1 },
    { id: 7, name: "Submitted", phaseId: 3, order: 2 },
    { id: 8, name: "Waiting", phaseId: 4, order: 1 },
    { id: 9, name: "Following Up", phaseId: 4, order: 2 },
    { id: 10, name: "Interview", phaseId: 5, order: 1 },
    { id: 11, name: "Offer", phaseId: 5, order: 2 },
    { id: 12, name: "Closed", phaseId: 5, order: 3 },
  ],
};

export const SETTINGS = {
  customColumns: [
    { id: "culture_fit", label: "Culture Fit" },
    { id: "ai_resilience", label: "AI Resilience" },
    { id: "growth_potential", label: "Growth" },
  ],
  kanban: KANBAN,
};

export const DEFAULT_ORDER = [
  "_category", "_drag", "source", "title", "organization", "industry", "location", "workload", "salary",
  "commute", "score", "status", "applied_at", "follow_up_at", "cv", "cl", "misc_files", "delete",
];

const job = (n: number, extra: Record<string, unknown> = {}) => ({
  id: `job_${n}`,
  title: `Role ${n}`,
  organization: `Org ${n}`,
  location: "Zurich",
  salary: "120",
  commute: "45",
  workload: "100",
  source: "LinkedIn",
  status_id: 1,
  applied_at: "2026-01-05",
  follow_up_at: "2026-02-01",
  snooze_count: 0,
  follow_up_manual: false,
  files: [],
  evaluations: {},
  ...extra,
});

export interface ExportCase {
  name: string;
  jobs: any[];
  orderedColumnIds: string[];
  settings: any;
  layout?: any;
}

export const exportCases: ExportCase[] = [
  { name: "basic", jobs: [job(1), job(2, { status_id: 7, score: 7.5 }), job(3, { status_id: 99 })], orderedColumnIds: DEFAULT_ORDER, settings: SETTINGS },
  {
    name: "category-and-order",
    jobs: [job(1, { categoryName: "Hot" }), job(2), job(3, { categoryName: "Cold, but \"fine\"" })],
    orderedColumnIds: ["status", "title", "organization", "score"],
    settings: SETTINGS,
  },
  {
    name: "layout-visibility-and-label",
    jobs: [job(1), job(2)],
    orderedColumnIds: DEFAULT_ORDER,
    settings: SETTINGS,
    layout: {
      columnVisibility: { source: false, commute: false, eval_culture_fit: false },
      columnLayout: { title: { label: "Position", superHeader: "Opportunity", subHeader: "Pos" } },
    },
  },
  {
    name: "custom-evaluation-columns",
    jobs: [
      job(1, { evaluations: { culture_fit: true, ai_resilience: 7, growth_potential: 3.5, orphan: false } }),
      job(2, { evaluations: { culture_fit: false, ai_resilience: "n/a" } }),
      job(3, {}),
      job(4, { evaluations: null }),
    ],
    orderedColumnIds: ["title", "eval_culture_fit", "eval_ai_resilience", "eval_growth_potential", "eval_orphan", "eval_unknown"],
    settings: SETTINGS,
  },
  {
    name: "visibility-of-custom-uses-raw-id",
    jobs: [job(1, { evaluations: { culture_fit: true, growth_potential: 2 } })],
    orderedColumnIds: ["title", "eval_culture_fit", "eval_growth_potential"],
    settings: SETTINGS,
    layout: { columnVisibility: { culture_fit: false, eval_growth_potential: false } },
  },
  {
    name: "quoting",
    jobs: [
      job(1, { title: "Comma, inside", organization: 'Quote " inside', location: "Line\nbreak" }),
      job(2, { title: "CR only\rhere", organization: " padded ", location: "=1+1" }),
      job(3, { title: "Zürich Ünïcode ✓ 日本", organization: "semi;colon", location: "tab\there" }),
      job(4, { title: "", organization: "", location: "" }),
    ],
    orderedColumnIds: ["title", "organization", "location"],
    settings: SETTINGS,
  },
  {
    name: "numbers",
    jobs: [
      job(1, { score: 7.333333333333333, salary: 120000, workload: 80, commute: 0.1 + 0.2 }),
      job(2, { score: 1e21, salary: 1.5e-7, workload: -0, commute: 123456789012345 }),
      job(3, { score: 2147483648, salary: -2147483649, workload: 12345678901.5, commute: 0.000123456789012 }),
      job(4, { score: 1234567890123, salary: 0.5, workload: 100, commute: 1e300 }),
      job(5, { score: 99999.99999999, salary: 0.0001, workload: 3, commute: -7.25 }),
      job(6, { score: true, salary: false, workload: null, commute: undefined }),
    ],
    orderedColumnIds: ["score", "salary", "workload", "commute"],
    settings: SETTINGS,
  },
  { name: "no-jobs", jobs: [], orderedColumnIds: DEFAULT_ORDER, settings: SETTINGS },
  { name: "no-columns", jobs: [job(1), job(2)], orderedColumnIds: ["_drag", "cv", "delete"], settings: SETTINGS },
  {
    name: "duplicate-labels",
    jobs: [job(1, { evaluations: { culture_fit: 5, growth_potential: 9 } })],
    orderedColumnIds: ["title", "organization", "eval_culture_fit", "eval_growth_potential"],
    settings: { ...SETTINGS, customColumns: [{ id: "culture_fit", label: "Job" }, { id: "growth_potential", label: "Job" }] },
  },
  {
    name: "numeric-like-labels",
    jobs: [job(1, { evaluations: { culture_fit: 5, growth_potential: 9 } })],
    orderedColumnIds: ["title", "eval_culture_fit", "organization", "eval_growth_potential"],
    settings: { ...SETTINGS, customColumns: [{ id: "culture_fit", label: "2024" }, { id: "growth_potential", label: "7" }] },
  },
  {
    name: "id-label-first",
    jobs: [job(1), job(2)],
    orderedColumnIds: ["title", "organization"],
    settings: SETTINGS,
    layout: { columnLayout: { title: { label: "ID", superHeader: "Opportunity", subHeader: "ID" } } },
  },
  {
    name: "non-scalar-and-unknown-columns",
    jobs: [job(1, { files: ["a", "b"], extra: { x: 1 }, note: "n1" }), job(2, { files: [], note: null })],
    orderedColumnIds: ["title", "files", "extra", "note", "categoryName"],
    settings: SETTINGS,
  },
  {
    name: "status-id-types",
    jobs: [job(1, { status_id: "1" }), job(2, { status_id: 2 }), job(3, { status_id: 2.0 }), job(4, {})],
    orderedColumnIds: ["title", "status"],
    settings: SETTINGS,
  },
];

export interface ImportCase {
  name: string;
  /** Raw file bytes as a latin1 string (every char is one byte). */
  bytes: string;
  settings: any;
  /** Overrides applied to the guessed mapping, by file header. */
  mapping?: Record<string, { targetColumnId: string; isCustom?: boolean }>;
  workspaceId?: string;
}

const utf8 = (s: string) => Buffer.from(s, "utf8").toString("latin1");
const bom = "\xEF\xBB\xBF";

export const importCases: ImportCase[] = [
  {
    name: "english-bom",
    bytes: bom + utf8(
      "Job,Organization,Location,Salary,Workload,Source,Status,Last Action,Next Action,Culture Fit\n" +
      "Engineer,Acme,Zurich,120,80,LinkedIn,Interview,sent,soon,Yes\n" +
      "Designer,Globex,Bern,95,100,Referral,Spotted,,later,No\n",
    ),
    settings: SETTINGS,
  },
  {
    name: "german-aliases",
    bytes: bom + utf8(
      "Firma,Bezeichnung,Ort,Gehalt,Quelle,Pensum,Phase,Bemerkung,Frist\n" +
      "Acme AG,Entwickler,Zürich,\"120'000\",Jobs.ch,80,Warten,Rückruf offen,bald\n" +
      "Beta GmbH,Designerin,Bern,95000,Empfehlung,100,Angebot,,\n",
    ),
    settings: SETTINGS,
  },
  {
    name: "custom-columns",
    bytes: bom + utf8(
      "Title,Culture Fit,AI Resilience,Growth,culture,Other Score\n" +
      "A,yes,7,3.5,ja,\"7,5\"\n" +
      "B,Nein,n/a,abc,false,4x\n" +
      "C,TRUE,12.5kg,1e2,N,-3\n",
    ),
    settings: SETTINGS,
  },
  {
    name: "status-linking",
    bytes: utf8(
      "Title,Status\n" +
      "a,Interview\n" + "b,INTERVIEW scheduled\n" + "c,waiting\n" + "d,Following up soon\n" + "e,Nonsense\n" + "f,\n" + "g,closed offer\n",
    ),
    settings: SETTINGS,
  },
  {
    name: "semicolon-and-crlf",
    bytes: bom + utf8("Job;Organization;Location\r\nA;Acme;Zurich\r\nB;\"Be; ta\";Bern\r\n"),
    settings: SETTINGS,
  },
  {
    name: "tab-separated",
    bytes: utf8("Job\tOrganization\tLocation\nA\tAcme\tZurich\nB\tBeta\tBern\n"),
    settings: SETTINGS,
  },
  {
    name: "quoted-fields",
    bytes: bom + utf8(
      "Job,Organization,Notes\n" +
      "\"Comma, role\",\"Quote \"\"Q\"\" Org\",\"line1\nline2\"\n" +
      "B,Beta,\"trailing \"\" quote\"\n" +
      "C,\"\",plain\n",
    ),
    settings: SETTINGS,
  },
  {
    name: "ragged-and-blank",
    bytes: utf8("Job,Organization,Location\nA,,Zurich\n\nB\n,,\n  ,x,\nC,Gamma\n"),
    settings: SETTINGS,
  },
  {
    name: "first-row-has-gaps",
    bytes: utf8("Job,Organization,Location\n,Acme,\nB,Beta,Bern\n"),
    settings: SETTINGS,
  },
  {
    name: "duplicate-and-empty-headers",
    bytes: utf8("Job,Job,,,Location,Location\nA,B,c,d,e,f\n"),
    settings: SETTINGS,
  },
  {
    name: "numeric-headers",
    bytes: utf8("Job,2,1,Location\nA,x,y,Zurich\n"),
    settings: SETTINGS,
  },
  {
    name: "latin1-no-bom",
    bytes: "Job,Organization,Location\nIngenieur,M\xFCller AG,Z\xFCrich\n",
    settings: SETTINGS,
  },
  {
    name: "utf8-no-bom-is-read-as-latin1",
    bytes: utf8("Job,Organization,Location\nIngenieur,Müller AG,Zürich\n"),
    settings: SETTINGS,
  },
  {
    name: "number-typing",
    bytes: utf8(
      "Job,Salary,Workload,Commute,Notes\n" +
      "a,120000,80%,007,1e3\n" +
      "b,\"95'000\",100,\"1,234\",0x1F\n" +
      "c,CHF 120k,\"(5)\",1.50,$1\n" +
      "d,120.00,.5,5.5,+7\n" +
      "e,TRUE,FALSE,true,=SUM(A1)\n" +
      "f,#N/A,#NULL!,#REF!,=\"quoted\"\n" +
      "g,  42  ,   ,\"1,2,3,4\",-0\n",
    ),
    settings: SETTINGS,
  },
  {
    name: "url-and-local-urls",
    bytes: utf8(
      "Job,URL,Organization\n" +
      "a,  https://example.com/jobs/1  ,Acme\n" +
      "b,,Beta\n" +
      "c,\"   \",Gamma\n" +
      "d,https://example.com/jobs/4,Delta\n",
    ),
    settings: SETTINGS,
    workspaceId: "ws-test-1",
  },
  {
    name: "manual-mapping-edits",
    bytes: utf8("Col A,Col B,Col C\nAcme,Engineer,Bern\nBeta,Designer,Zurich\n"),
    settings: SETTINGS,
    mapping: {
      "Col A": { targetColumnId: "organization" },
      "Col B": { targetColumnId: "title" },
      "Col C": { targetColumnId: "location" },
    },
  },
  {
    name: "mapping-to-other-fields",
    bytes: utf8("Job,Notes,Description,Language,Industry,Company,Score\nA,n,d,de,Tech,Acme,8.5\n"),
    settings: SETTINGS,
  },
  {
    name: "header-text-edge-cases",
    bytes: utf8("job,ORGANISATION,Stadt,Link,Lohn,Datum,Percent,state\nA,B,C,D,E,F,G,H\n"),
    settings: SETTINGS,
  },
  {
    name: "no-statuses",
    bytes: utf8("Job,Status\nA,Interview\n"),
    settings: { customColumns: [], kanban: { phases: [], statuses: [] } },
  },
  {
    name: "status-order-by-phase",
    bytes: utf8("Job,Status\nA,x\nB,second\nC,third\n"),
    settings: {
      customColumns: [],
      kanban: {
        phases: [{ id: 10, order: 2 }, { id: 20, order: 1 }],
        statuses: [
          { id: 100, name: "second", phaseId: 10, order: 1 },
          { id: 200, name: "third", phaseId: 10, order: 2 },
          { id: 300, name: "first", phaseId: 20, order: 5 },
          { id: 400, name: "", phaseId: 99, order: 0 },
        ],
      },
    },
  },
  { name: "header-only", bytes: utf8("Job,Organization\n"), settings: SETTINGS },
  { name: "empty-file", bytes: "", settings: SETTINGS },
  { name: "single-column-quoted-header", bytes: utf8("\"Job\"\n\"A\"\n"), settings: SETTINGS },
  { name: "sep-line", bytes: utf8("sep=;\nJob;Organization\nA;Acme\n"), settings: SETTINGS },
  { name: "sep-line-crlf", bytes: utf8("sep=|\r\nJob|Organization\r\nA|Acme\r\n"), settings: SETTINGS },
  { name: "pipe-separated", bytes: utf8("Job|Organization|Location\nA|Acme|Zurich\n"), settings: SETTINGS },
  {
    name: "many-rows-preview-limit",
    bytes: utf8("Job,Organization\n" + Array.from({ length: 9 }, (_, i) => `r${i},o${i}`).join("\n") + "\n"),
    settings: SETTINGS,
  },
  {
    name: "chunking-60-rows",
    bytes: utf8("Job,Organization\n" + Array.from({ length: 60 }, (_, i) => `r${i},o${i}`).join("\n") + "\n"),
    settings: SETTINGS,
  },
];

/** Strings whose CSV cell typing is compared with SheetJS (date-like text is deliberately absent). */
export const cellStrings: string[] = [
  "0", "1", "-1", "+1", "007", "7.50", "7.", ".5", "-.5", "+.5", "1e3", "1E3", "1e+3", "1e-3", "1e", "e3", ".", "-", "+", "1.2.3",
  "0x10", "0X1f", "0o17", "0b101", "0x", "0xg", "-0x10", "1_000", "1 000", " 42 ", "\t42", "42 ", "Infinity", "-Infinity", "+Infinity",
  "infinity", "NaN", "nan", "1e999", "-1e999", "12,345", "1,234,567", "1,2,3", "12,34", "1,,2", ",5", "5,", "$5", "$1,000", "5%", "50%", "5 %",
  "1.5%", "%5", "(5)", "(5.5)", "(1,000)", "-(5)", "((5))", "(5", "5)", "$(5)", "(5%)", "$", "%", "()", "(a)", "abc", "12abc", "abc12", "1a",
  "TRUE", "FALSE", "true", "false", "True", "yes", "=1+1", "=SUM(A1:A2)", "=\"x\"", "=\"x\"\"y\"", "= 5", "==", "#N/A", "#NULL!", "#DIV/0!",
  "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#GETTING_DATA", "#nope", "#", "  ", " ", "-0", "0.0", "00", "0e0", "1e308", "1e-324", "9007199254740993",
  "123456789012345678901234567890", "0.1", "0.30000000000000004", "1.7976931348623157e308", "5e-324", "٣", "１２", "1 ", " 1", "- 1", "1-", "--1", "+-1",
  "CHF 120k", "120k", "120'000", "€5", "5€",
];
