// Performance comparison: wall-clock + output size for turbo-xlsx vs exceljs vs
// SheetJS on two shared workloads (a styled 1k-row sheet and a 50k-row sheet).
// Run: `node src/perf.mjs`  (optionally `node --expose-gc src/perf.mjs` for RSS).

import { writeFileSync } from "node:fs";
import { adapters } from "./adapters.mjs";
import { tabularRows } from "./workloads.mjs";

const WORKLOADS = [
  { name: "1k x 20 styled", rows: 1_000, cols: 20, reps: 5 },
  { name: "50k x 30", rows: 50_000, cols: 30, reps: 3 },
];

/** Median of an array of numbers. */
function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

/** Time `fn` (sync or async) `reps` times; return median ms, bytes, RSS delta. */
async function measure(fn, rows, reps) {
  const times = [];
  let bytes = 0;
  let rss = 0;
  for (let i = 0; i < reps; i++) {
    if (global.gc) global.gc();
    const before = process.memoryUsage().rss;
    const t0 = performance.now();
    const buf = await fn(rows);
    times.push(performance.now() - t0);
    bytes = buf.length;
    rss = Math.max(rss, process.memoryUsage().rss - before);
  }
  return { ms: median(times), bytes, rss };
}

function fmtMs(ms) {
  return ms >= 1000 ? `${(ms / 1000).toFixed(2)} s` : `${ms.toFixed(1)} ms`;
}

function fmtBytes(b) {
  return b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : `${(b / 1e3).toFixed(0)} KB`;
}

async function run() {
  const lines = [
    "# Competitive performance\n",
    `_Node ${process.version}, ${process.platform}/${process.arch}._\n`,
  ];
  for (const wl of WORKLOADS) {
    const rows = tabularRows(wl.rows, wl.cols);
    console.log(`\n## ${wl.name}  (median of ${wl.reps})`);
    lines.push(`\n## ${wl.name}  (median of ${wl.reps})\n`);
    lines.push("| library | time | output | peak RSS |", "|---|---|---|---|");
    const header =
      "library".padEnd(16) + "time".padStart(12) + "output".padStart(12) + "RSS".padStart(12);
    console.log(header);
    for (const a of adapters) {
      const { ms, bytes, rss } = await measure(a.perf, rows, wl.reps);
      console.log(
        a.name.padEnd(16) +
          fmtMs(ms).padStart(12) +
          fmtBytes(bytes).padStart(12) +
          fmtBytes(rss).padStart(12),
      );
      lines.push(`| ${a.name} | ${fmtMs(ms)} | ${fmtBytes(bytes)} | ${fmtBytes(rss)} |`);
    }
  }
  writeFileSync(new URL("../RESULTS.perf.md", import.meta.url), `${lines.join("\n")}\n`);
  console.log("\nwrote RESULTS.perf.md");
}

run();
