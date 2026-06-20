// PARSE perf: turbo-xlsx vs SheetJS reading the SAME DEFLATEd file into a value
// grid. SheetJS writes the fixture (so the bytes are real, Excel-style deflated
// parts), then both parsers read it N times; we report the median wall-clock and
// the speedup. Run after building the parse-enabled addon (see parse-compat.mjs).

import { createRequire } from "node:module";
import * as XLSX from "xlsx";
import { perfGrid, viaSheetJS } from "./parse-fixtures.mjs";

const require = createRequire(import.meta.url);
const turbo = require("../../../crates/turbo-xlsx-napi/index.js");

if (typeof turbo.parse !== "function") {
  console.error("turbo.parse is unavailable — build with `--features parse` (see parse-compat.mjs).");
  process.exit(2);
}

function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

function time(fn, iters) {
  const ms = [];
  for (let i = 0; i < iters; i++) {
    const t0 = process.hrtime.bigint();
    fn();
    ms.push(Number(process.hrtime.bigint() - t0) / 1e6);
  }
  return median(ms);
}

const turboRead = (buf) => () => JSON.parse(turbo.parse(buf, { format: "json" }));
const sheetjsRead = (buf) => () => {
  const wb = XLSX.read(buf, { type: "buffer" });
  return XLSX.utils.sheet_to_json(wb.Sheets[wb.SheetNames[0]], { header: 1 });
};

for (const rows of [1_000, 50_000]) {
  const buf = viaSheetJS(perfGrid(rows));
  const iters = rows >= 50_000 ? 5 : 30;
  // Warm up both paths (JIT + addon load).
  turboRead(buf)();
  sheetjsRead(buf)();
  const t = time(turboRead(buf), iters);
  const s = time(sheetjsRead(buf), iters);
  const kb = (buf.length / 1024).toFixed(0);
  console.log(
    `${String(rows).padStart(6)} rows (${kb}KB deflated):  ` +
      `turbo ${t.toFixed(2)}ms   SheetJS ${s.toFixed(2)}ms   → ${(s / t).toFixed(1)}× faster`,
  );
}
