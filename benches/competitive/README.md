# Competitive perf + conformance harness

Compares **turbo-xlsx** against [`exceljs`](https://www.npmjs.com/package/exceljs)
and [`xlsx`](https://www.npmjs.com/package/xlsx) (SheetJS Community) on identical
logical workloads. Two questions:

1. **Performance** — wall-clock, output size, peak RSS on a styled 1k-row sheet and
   a 50k-row sheet.
2. **Conformance / compatibility** — generate the same feature-rich workbook with
   each library, then read every file back through one independent reader
   (`exceljs`) and check which formatting capabilities survive the round-trip.

## Setup

Build the turbo-xlsx native addon first (the harness loads it from
`../../crates/turbo-xlsx-napi`), then install the competitors:

```sh
# from the repo root — RELEASE build for honest numbers
cargo build -p turbo-xlsx-napi --release
node crates/turbo-xlsx-napi/scripts/copy-addon.mjs

cd benches/competitive
npm install
```

## Run

```sh
node src/perf.mjs            # → console table + RESULTS.perf.md
node --expose-gc src/perf.mjs  # add --expose-gc for meaningful peak-RSS numbers
node src/conformance.mjs     # → console matrix + RESULTS.conformance.md
npm run all                  # both
```

## Layout

| File | What |
|---|---|
| `src/workloads.mjs` | Neutral logical workloads (tabular + feature-rich), library-agnostic. |
| `src/adapters.mjs` | One adapter per library: neutral workload → that library's model → `.xlsx` Buffer. turbo-xlsx uses the **streaming** writer for the large workload (its scale path) and the batch writer for the feature case. |
| `src/perf.mjs` | Times each adapter on each workload; writes `RESULTS.perf.md`. |
| `src/conformance.mjs` | Round-trips each adapter's feature workbook through `exceljs` and builds the capability matrix; writes `RESULTS.conformance.md`. |
| `src/parse-fixtures.mjs` | Writes a mixed-type grid as DEFLATEd `.xlsx` via SheetJS / ExcelJS (the bytes turbo's STORED writer never produces) + the reference read. |
| `src/parse-compat.mjs` | `npm run parse:compat` — parses each writer's file with turbo **and** SheetJS and diffs **every cell**; exits non-zero on any mismatch. |
| `src/parse-perf.mjs` | `npm run parse:perf` — times turbo vs SheetJS reading the same DEFLATEd file into a value grid. |

The parse scripts need the **parse-enabled** addon:

```sh
cargo build -p turbo-xlsx-napi --release --features parse
node ../../crates/turbo-xlsx-napi/scripts/copy-addon.mjs
npm run parse:compat   # turbo vs SheetJS + ExcelJS, cell-for-cell
npm run parse:perf     # turbo ~3–4× faster than SheetJS reading DEFLATEd files
```

## Notes

- Numbers are **indicative** and machine-specific — reproduce locally; do not treat
  the committed prose in the root README as a guarantee.
- Use the **release** addon. A debug `.node` is ~10× slower and inflates RSS.
- turbo-xlsx currently emits a **STORED** (uncompressed) OPC zip, so its files are
  larger than exceljs's DEFLATE output; DEFLATE is planned future work.
- `RESULTS.*.md` are generated and git-ignored.
