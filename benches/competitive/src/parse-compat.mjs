// PARSE conformance: prove turbo-xlsx reads SheetJS/ExcelJS-DEFLATEd files
// cell-for-cell — i.e. it is not "outputting garbage". For each writer we parse
// the same bytes with turbo and with SheetJS and diff every cell after a shared
// canonicalization (numbers within epsilon, dates to YYYY-MM-DD, bools to
// TRUE/FALSE, blanks to ""). Exit non-zero on any mismatch.

import { createRequire } from "node:module";
import {
  readSheetJS,
  sampleGrid,
  viaExcelJS,
  viaSheetJS,
} from "./parse-fixtures.mjs";

const require = createRequire(import.meta.url);
const turbo = require("../../../crates/turbo-xlsx-napi/index.js");

if (typeof turbo.parse !== "function") {
  console.error("turbo.parse is unavailable — build the addon with `--features parse`:");
  console.error("  cargo build -p turbo-xlsx-napi --release --features parse && \\");
  console.error("  node crates/turbo-xlsx-napi/scripts/copy-addon.mjs");
  process.exit(2);
}

// Canonicalize any cell value (from either reader) to a comparable token.
function canon(v) {
  if (v === null || v === undefined || v === "") return "";
  if (v instanceof Date) return v.toISOString().slice(0, 10);
  if (typeof v === "boolean") return v ? "TRUE" : "FALSE";
  if (typeof v === "number") return Math.abs(v) < 1e-9 ? "0" : String(Math.round(v * 1e6) / 1e6);
  const m = String(v).match(/^(\d{4}-\d{2}-\d{2})/); // turbo emits ISO dates as text
  return m ? m[1] : String(v);
}

// Diff two grids cell-for-cell; return the list of mismatches.
function diff(turboGrid, refGrid) {
  const rows = Math.max(turboGrid.length, refGrid.length);
  const misses = [];
  for (let r = 0; r < rows; r++) {
    const a = turboGrid[r] ?? [];
    const b = refGrid[r] ?? [];
    const cols = Math.max(a.length, b.length);
    for (let c = 0; c < cols; c++) {
      const ta = canon(a[c]);
      const tb = canon(b[c]);
      if (ta !== tb) misses.push({ r, c, turbo: ta, ref: tb });
    }
  }
  return misses;
}

function turboGridOf(buf) {
  return JSON.parse(turbo.parse(buf, { format: "json" })).sheets[0].rows;
}

async function main() {
  const grid = sampleGrid();
  const cases = [
    { writer: "SheetJS", buf: viaSheetJS(grid) },
    { writer: "ExcelJS", buf: await viaExcelJS(grid) },
  ];

  let failed = 0;
  for (const { writer, buf } of cases) {
    const misses = diff(turboGridOf(buf), readSheetJS(buf));
    const cells = grid.length * grid[0].length;
    if (misses.length === 0) {
      console.log(`✓ ${writer.padEnd(8)} ${cells} cells parsed identically (DEFLATE, ${buf.length}B)`);
    } else {
      failed++;
      console.log(`✗ ${writer.padEnd(8)} ${misses.length}/${cells} cell(s) differ:`);
      for (const m of misses.slice(0, 8)) {
        console.log(`    [r${m.r},c${m.c}] turbo=${JSON.stringify(m.turbo)} ref=${JSON.stringify(m.ref)}`);
      }
    }
  }

  // Also verify the CSV + Markdown serializers render without throwing.
  const csv = turbo.parse(viaSheetJS(grid), { format: "csv" });
  const md = turbo.parse(viaSheetJS(grid), { format: "md" });
  console.log(`\nserializers: csv ${csv.length}B, markdown ${md.length}B (both non-empty: ${csv.length > 0 && md.length > 0})`);

  if (failed > 0) {
    console.error(`\n${failed} writer(s) mismatched — parser is wrong somewhere.`);
    process.exit(1);
  }
  console.log("\nAll writers round-trip cell-for-cell. ✓");
}

main();
