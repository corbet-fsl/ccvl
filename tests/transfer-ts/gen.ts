// Runs the TypeScript reference (CareerVector `lib/domain/src/{export,import}.ts`, SheetJS) over the
// parity cases and writes the fixtures the Rust tests compare against.
//
//   CV_ROOT=<careervector checkout> bun --no-install gen.ts [--check]
//
// `--check` regenerates in memory and fails when a committed fixture differs (drift guard).

import * as fs from "node:fs";
import * as path from "node:path";
import * as XLSX from "xlsx";
import { exportCases, importCases, cellStrings, SETTINGS } from "./cases.ts";

XLSX.set_fs(fs);

const CV_ROOT = process.env.CV_ROOT ?? path.resolve(import.meta.dir, "../../../cv");
const here = import.meta.dir;
const outDir = path.resolve(here, "../fixtures/transfer");
const check = process.argv.includes("--check");
const scratch = path.resolve(here, "out");
fs.mkdirSync(scratch, { recursive: true });

const domain = path.join(CV_ROOT, "lib/domain/src");
const { exportToExcel } = await import(path.join(domain, "export.ts"));
const { parseImportFile, processImportData } = await import(path.join(domain, "import.ts"));
const { DEFAULT_COLUMNS } = await import(path.join(domain, "defaults/index.ts"));

const written = new Map<string, Buffer | string>();
const put = (rel: string, data: Buffer | string) => written.set(rel, data);
const json = (v: unknown) => JSON.stringify(v, null, 2) + "\n";

// ---------------------------------------------------------------- default columns
put(
  "default-columns.json",
  json(Object.entries(DEFAULT_COLUMNS).map(([id, c]: [string, any]) => [id, c.label, c.subHeader, c.superHeader])),
);

// ---------------------------------------------------------------- numbers
const numbers: number[] = [
  0, -0, 1, -1, 0.5, 0.1, 0.2, 0.1 + 0.2, 1 / 3, -1 / 3, 2 / 3, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 0.01, 0.0001234, 0.00001234, 123456.789,
  2147483647, 2147483648, -2147483648, -2147483649, 4294967296, 1e9, 1e10, 1e11, 1e12, 1e15, 1e20, 1e21, 1e22, 1e100, 1e300, 1.5e21, 1234567890123,
  12345678901.5, 123456789012345, 0.000123456789012, 99999.99999999, 7.333333333333333, 9.999999999999999, 100.00000000000001, 5e-324,
  1.7976931348623157e308, 123456789.123456789, 12345.6789012345, -0.000001, -1234567.891, 8.5, 3.5, 4.35, 1.005, 2.675, 0.7, 0.07, 0.007,
  1234.5678, 0.30000000000000004, 1e-10, 1.2e-9, 123456789012, 1.23456789012e-5, 4.9e-324, 0.1 + 0.7, 33.333333333333336, 66.66666666666667,
];
let seed = 12345;
const rnd = () => ((seed = (seed * 1664525 + 1013904223) % 4294967296) / 4294967296);
for (let i = 0; i < 400; i++) {
  const e = Math.floor(rnd() * 40) - 20;
  numbers.push((rnd() - 0.5) * 2 * Math.pow(10, e));
}
for (let i = 0; i < 100; i++) numbers.push(Math.round(rnd() * 1e6) / 1000);
put(
  "numbers.json",
  json(numbers.map((v) => ({ bits: Object.is(v, -0) ? "-0" : String(v), general: XLSX.SSF.format("General", v), string: String(v) }))),
);

// ---------------------------------------------------------------- cell typing
const typed: any[] = [];
const dateLike: string[] = [];
for (const s of cellStrings) {
  const csv = `h\n${s.includes(",") || s.includes('"') || s.includes("\n") ? `"${s.replaceAll('"', '""')}"` : s}\n`;
  const asDate = XLSX.read(csv, { type: "string", cellDates: true });
  const cell = asDate.Sheets[asDate.SheetNames[0]]["A2"];
  if (cell && cell.t === "d") {
    dateLike.push(s);
    continue;
  }
  const wb = XLSX.read(csv, { type: "string" });
  const rows = XLSX.utils.sheet_to_json(wb.Sheets[wb.SheetNames[0]]) as any[];
  typed.push({ csv, rows: rows.map((r) => JSON.parse(JSON.stringify(r, (_k, v) => (typeof v === "number" && !Number.isFinite(v) ? { nonFinite: String(v) } : v)))) });
}
put("cells.json", json({ typed, dateLike }));

// ---------------------------------------------------------------- export
const today = new Date().toISOString().split("T")[0];
for (const c of exportCases) {
  const file = path.join(scratch, `export-${c.name}`);
  fs.rmSync(`${file}.csv`, { force: true });
  await exportToExcel(c.jobs, c.orderedColumnIds, c.settings, "csv", file, c.layout ?? {});
  const bytes = fs.existsSync(`${file}.csv`) ? fs.readFileSync(`${file}.csv`) : Buffer.alloc(0);
  const { prepareExportData } = await import(path.join(domain, "export.ts"));
  const { columns } = prepareExportData(c.jobs, c.orderedColumnIds, c.settings, c.layout ?? {});
  put(`export/${c.name}.json`, json({ jobs: c.jobs, orderedColumnIds: c.orderedColumnIds, settings: c.settings, layout: c.layout ?? {}, expectedColumns: columns, written: bytes.length > 0 }));
  put(`export/${c.name}.csv`, bytes);
}

// ---------------------------------------------------------------- import
// Verbatim sources of the dashboard's job-creation helpers, so the oracle cannot drift from them.
const svelte = fs.readFileSync(path.join(CV_ROOT, "ui/src/lib/components/dashboard/Dashboard.svelte"), "utf8");
function extractFunction(source: string, name: string): string {
  const start = source.indexOf(`function ${name}(`);
  if (start < 0) throw new Error(`function ${name} not found`);
  let i = source.indexOf("(", start);
  for (let depth = 0; i < source.length; i++) {
    if (source[i] === "(") depth++;
    else if (source[i] === ")" && --depth === 0) break;
  }
  i = source.indexOf("{", source.indexOf(")", i) + (source[i] === ")" ? 0 : 1));
  // The body starts at the first "{" after the parameter list (a return type here has no braces).
  let depth = 0;
  for (let j = i; j < source.length; j++) {
    if (source[j] === "{") depth++;
    else if (source[j] === "}" && --depth === 0) return source.slice(start, j + 1);
  }
  throw new Error(`unbalanced ${name}`);
}
const transpiler = new Bun.Transpiler({ loader: "ts" });
const llm = fs.readFileSync(path.join(domain, "llm.ts"), "utf8");
const helperSource = [extractFunction(llm, "normalizeSalary"), ...["normalizedSalaryNumber", "numberOrNull", "jobCreateAttributes"].map((n) => extractFunction(svelte, n))].join("\n");
const helpers = new Function(`${transpiler.transformSync(helperSource)}\nreturn { jobCreateAttributes };`)();

const failures: string[] = [];
for (const c of importCases) {
  const bytes = Buffer.from(c.bytes, "latin1");
  put(`import/${c.name}.csv`, bytes);
  let preview: any;
  try {
    preview = await parseImportFile(new File([bytes], `${c.name}.csv`), c.settings);
  } catch (e: any) {
    put(`import/${c.name}.json`, json({ settings: c.settings, error: e.message }));
    continue;
  }
  // The oracle compares text only: refuse cases where SheetJS read a date.
  const asDate = XLSX.read(bytes, { type: "buffer", cellDates: true });
  for (const [addr, cell] of Object.entries(asDate.Sheets[asDate.SheetNames[0]]) as [string, any][]) {
    if (!addr.startsWith("!") && cell.t === "d") failures.push(`${c.name}: ${addr} is read as a date`);
  }
  const mappings = preview.columns.map((m: any) => {
    const o = c.mapping?.[m.fileHeader];
    return o ? { ...m, targetColumnId: o.targetColumnId, isCustom: o.isCustom ?? m.isCustom } : m;
  });
  const jobs = processImportData(preview.rawData, mappings, c.settings).map((j: any, i: number) => ({
    ...j,
    id: `imported-${i}`,
    follow_up_at: j.follow_up_at === today ? "<today>" : j.follow_up_at,
  }));
  const ws = c.workspaceId ?? "ws-test";
  const now = "2030-01-01T10:00:00.000Z";
  let local = 0;
  const creates = jobs.map((imported: any) => {
    const withToday = { ...imported, follow_up_at: imported.follow_up_at === "<today>" ? today : imported.follow_up_at };
    const url = typeof withToday.url === "string" && withToday.url.trim() ? withToday.url.trim() : `careervector-local://${ws}/L${local++}`;
    return { op: { kind: "job.create", url, attributes: helpers.jobCreateAttributes({ ...withToday, initialFollowUp: "2026-11-24", today, now }) } };
  });
  const batches: any[] = [];
  for (let o = 0; o < creates.length; o += 50) batches.push({ kind: "batch", ops: creates.slice(o, o + 50) });
  const text = (x: unknown) => JSON.stringify(x, (_k, v) => (typeof v === "number" && !Number.isFinite(v) ? { nonFinite: String(v) } : v), 2)
    .replaceAll(today, "<today>");
  put(
    `import/${c.name}.json`,
    text({ settings: c.settings, mapping: c.mapping ?? {}, workspaceId: ws, expected: { preview, mappings, jobs, batches } }) + "\n",
  );
}
if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

// ---------------------------------------------------------------- date divergence
{
  const csv = "Job,Last Action,Next Action,Notes\nA,2026-01-05,01/02/2026,Jan 5 2026\nB,2026-01-05T10:00:00Z,5.1.2026,x\n";
  const wb = XLSX.read(csv, { type: "string" });
  const rows = XLSX.utils.sheet_to_json(wb.Sheets[wb.SheetNames[0]]);
  put("dates.json", json({ csv, sheetjsRows: rows }));
}

// ---------------------------------------------------------------- write or check
let drift = 0;
for (const [rel, data] of written) {
  const target = path.join(outDir, rel);
  if (check) {
    const cur = fs.existsSync(target) ? fs.readFileSync(target) : null;
    if (!cur || !cur.equals(Buffer.isBuffer(data) ? data : Buffer.from(data))) {
      console.error(`drift: ${rel}`);
      drift++;
    }
  } else {
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, data);
  }
}
if (check && drift) process.exit(1);
console.log(`${check ? "checked" : "wrote"} ${written.size} files; ${typed.length} typed cells, ${dateLike.length} date-like skipped`);
