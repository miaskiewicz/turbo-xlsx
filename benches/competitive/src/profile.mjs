// Micro-profiler for the turbo-xlsx Node write path. Splits the end-to-end time
// of the JSON-streaming path into its phases so optimization is profile-guided:
//   build    — JS building the per-row cell objects
//   stringify— JSON.stringify of the chunks
//   native   — writeRowsJson (N-API crossing + Rust from_str + core emit)
//   finish   — w.finish() (zip packaging)
//
// Run: node src/profile.mjs

import { createRequire } from "node:module";
import { tabularRows } from "./workloads.mjs";

const require = createRequire(import.meta.url);
const turbo = require("../../../crates/turbo-xlsx-napi/index.js");

function turboRow(row) {
  return {
    cells: row.map((c) => {
      if (c.kind === "currency") {
        return { type: "currency", value: c.value, currency: { code: "MXN", locale: "es-MX" } };
      }
      const style = c.header ? { font: { bold: true }, fill: "#dddddd" } : undefined;
      return style ? { type: "string", value: c.value, style } : { type: "string", value: c.value };
    }),
  };
}

function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

function timeit(fn, reps) {
  const ts = [];
  let sink;
  for (let i = 0; i < reps; i++) {
    const t0 = performance.now();
    sink = fn();
    ts.push(performance.now() - t0);
  }
  return { ms: median(ts), sink };
}

function profile(name, rows, cols, reps) {
  const grid = tabularRows(rows, cols);
  const build = timeit(() => grid.map(turboRow), reps);
  const built = build.sink;
  const stringify = timeit(() => JSON.stringify(built), reps);
  const json = stringify.sink;

  // Rust side: N-API crossing + serde_json::from_str (internally-tagged Cell
  // enum) + core emit + zip, from a precomputed JSON string (no JS-build/stringify).
  const rust = timeit(() => {
    const w = turbo.createWriter({ locale: "es-MX" });
    w.startSheet({ name: "P" });
    w.writeRowsJson(json);
    return w.finish().xlsx.length;
  }, reps);

  const total = build.ms + stringify.ms + rust.ms;
  console.log(`\n## ${name}  (median of ${reps})`);
  console.log(`  build (JS objects)  : ${build.ms.toFixed(2)} ms`);
  console.log(`  JSON.stringify      : ${stringify.ms.toFixed(2)} ms`);
  console.log(`  rust (from_str+emit): ${rust.ms.toFixed(2)} ms  <- dominant`);
  console.log(`  -- total            : ${total.toFixed(2)} ms`);
}

profile("1k x 20 styled", 1_000, 20, 50);
profile("50k x 30", 50_000, 30, 5);
