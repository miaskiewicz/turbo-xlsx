// Conformance / compatibility matrix: generate the same feature-rich workbook
// with each library, then read every file back through a single independent
// reader (exceljs) and check which formatting capabilities survived the
// round-trip. This is the concrete evidence behind the spec's claim that the
// community `xlsx` write path has no styling, while turbo-xlsx and exceljs do.
// Run: `node src/conformance.mjs`.

import { writeFileSync } from "node:fs";
import ExcelJS from "exceljs";
import { adapters } from "./adapters.mjs";
import { featureRows } from "./workloads.mjs";

/** Each probe reads the round-tripped sheet and returns whether it held up. */
const PROBES = {
  "string value": (ws) => ws.getCell("A2").value === "Ingeniería",
  "currency value": (ws) => approx(numberOf(ws.getCell("B2").value), 12345.67),
  "currency format": (ws) => /#,##0\.00/.test(ws.getCell("B2").numFmt ?? ""),
  "negative-in-red": (ws) => /red/i.test(ws.getCell("B2").numFmt ?? ""),
  "percent value": (ws) => approx(numberOf(ws.getCell("C2").value), 0.16),
  "percent format": (ws) => /%/.test(ws.getCell("C2").numFmt ?? ""),
  "real date": (ws) => isDateLike(ws.getCell("D2").value),
  "bold header": (ws) => ws.getCell("A1").font?.bold === true,
  "header fill": (ws) => hasFill(ws.getCell("A1")),
  "bold total": (ws) => ws.getCell("A3").font?.bold === true,
  "frozen header": (ws) => frozen(ws),
};

function numberOf(v) {
  if (typeof v === "number") return v;
  if (v && typeof v.result === "number") return v.result; // exceljs formula cell
  return Number(v);
}

function approx(a, b) {
  return Number.isFinite(a) && Math.abs(a - b) < 0.005;
}

function isDateLike(v) {
  return v instanceof Date && !Number.isNaN(v.getTime());
}

function hasFill(cell) {
  const argb = cell.fill?.fgColor?.argb;
  return typeof argb === "string" && argb.length > 0;
}

function frozen(ws) {
  const view = ws.views?.[0];
  return view?.state === "frozen" && (view.ySplit ?? 0) >= 1;
}

async function readBack(buffer) {
  const wb = new ExcelJS.Workbook();
  await wb.xlsx.load(buffer);
  return wb.worksheets[0];
}

async function run() {
  const rows = featureRows();
  const features = Object.keys(PROBES);
  const results = {};
  for (const a of adapters) {
    try {
      const ws = await readBack(await a.feature(rows));
      results[a.name] = features.map((f) => safeProbe(PROBES[f], ws));
    } catch (err) {
      results[a.name] = features.map(() => `err: ${err.message}`);
    }
  }
  report(features, results);
}

function safeProbe(probe, ws) {
  try {
    return probe(ws) === true;
  } catch {
    return false;
  }
}

function mark(v) {
  if (v === true) return "  ✓";
  if (v === false) return "  ✗";
  return " ?";
}

function report(features, results) {
  const names = Object.keys(results);
  const pad = Math.max(...features.map((f) => f.length));
  const head = "feature".padEnd(pad) + names.map((n) => n.padStart(16)).join("");
  console.log(`\n${head}`);
  const lines = [
    "# Conformance / compatibility matrix\n",
    `| feature | ${names.join(" | ")} |`,
    `|---|${names.map(() => "---").join("|")}|`,
  ];
  features.forEach((f, i) => {
    const cells = names.map((n) => results[n][i]);
    console.log(f.padEnd(pad) + cells.map((c) => mark(c).padStart(16)).join(""));
    lines.push(
      `| ${f} | ${cells.map((c) => (c === true ? "✓" : c === false ? "✗" : c)).join(" | ")} |`,
    );
  });
  writeFileSync(new URL("../RESULTS.conformance.md", import.meta.url), `${lines.join("\n")}\n`);
  console.log("\nwrote RESULTS.conformance.md");
}

run();
