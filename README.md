# turbo-xlsx

Fast native **structured-workbook-model → formatted XLSX** for Node — no headless
office, no paid SheetJS Pro licence. A Rust core (`turbo-xlsx-core`) exposed to
Node via `napi-rs` (`turbo-xlsx-napi`, published as **`turbo-xlsx`**), the XLSX
peer of [`turbo-html2pdf`](https://github.com/miaskiewicz/turbo-html2pdf): same
Rust-core + N-API blueprint, same `turbo-*` packaging.

It owns **workbook-model → XLSX**. The report engine produces structured result
sets; mapping a result set → workbook model is a plain data transform, and this
library turns that model into accountant-grade `.xlsx`.

## Why

- **Full styling control** over the OOXML we emit — currency number formats per
  locale, thousands separators, negative-in-red/parens, styled headers, bold
  totals rows, grouped/outlined columns, merges, freeze panes — with no paid
  licence.
- **Rust performance + a streaming writer** for tens of thousands of payroll rows
  without holding the workbook in memory.
- **Country-agnostic.** Locale and ISO-4217 currency code are **inputs**, never
  hardcoded — a MX report passes `{ code: "MXN", locale: "es-MX" }`, a PT report
  `{ code: "EUR", locale: "pt-PT" }`, with no branching anywhere.
- **Write-only, deterministic.** No formulas, no cross-sheet references. Totals
  are pre-computed values written as plain `number`/`currency` cells. The same
  input always produces byte-identical output.

## Install

```sh
npm install turbo-xlsx
```

Prebuilt platform `.node` addons ship in the tarball for the five `turbo-*`
targets (linux x64 gnu/musl, linux arm64, darwin arm64, win32 x64 msvc).

## Usage

```ts
import { write, writeRows, createWriter, createWorkbook } from "turbo-xlsx";

// One-shot: a complete workbook object → .xlsx bytes (a Buffer).
const buf = write({
  locale: "es-MX",
  sheets: [{
    name: "Resumen",
    columns: [{ key: "dept", width: 24 }, { key: "gross" }],
    freeze: { rows: 1 },
    rows: [
      { cells: [
        { type: "string", value: "Departamento", style: { font: { bold: true }, fill: "#dddddd" } },
        { type: "string", value: "Bruto" },
      ] },
      { cells: [
        { type: "string", value: "Ingeniería" },
        { type: "currency", value: 1234567, currency: { code: "MXN", locale: "es-MX", negative: "red-parens" } },
      ] },
      { isTotal: true, cells: [
        { type: "string", value: "Total" },
        { type: "currency", value: 9876543, currency: { code: "MXN" } },
      ] },
    ],
  }],
}, { meta: { title: "Reporte", company: "Flux" } });
// `currency` values are integer minor units (cents): 1234567 → 12,345.67.
```

### Five ways in

| Entry mode | Call |
|---|---|
| Declarative object | `write(workbook, opts?)` |
| JSON (string or value) | `writeFromJson(input, opts?)` — validated fail-closed against the schema |
| Imperative CRUD builder | `createWorkbook(opts?)` → `addSheet`/`addRows`/`updateCell`/`merge`/`freeze`/`setTotalsRow` → `build()` |
| Rows fast-path | `writeRows({ sheetName?, columns, rows }, opts?)` |
| Streaming (huge sheets) | `createWriter(opts?)` → `startSheet`/`writeRow`/`endSheet`/`finish()` |

CSV-in and HTML/template-in are **not** supported (CSV is an export *output*; XLSX
is data-driven, not a rendered page).

```ts
// Streaming: push rows as you page query results — O(1) per-row retention.
const w = createWriter({ locale: "es-MX" });
w.startSheet({ name: "Detalle", columns: [{ width: 20 }] });
for (const r of pageRows()) w.writeRow(r);
w.endSheet();
const { xlsx, diagnostics } = w.finish();
```

Fatal faults throw a typed `TurboXlsxError` with a stable `.code`
(`DuplicateSheetName`, `BadCellRef`, `InvalidSheetName`, `BadColor`,
`InvalidJson`, `SchemaViolation`, `EmptyWorkbook`). Non-fatal issues (a clamped
width, a dropped duplicate merge) come back as `diagnostics`, never thrown — the
same fatal-vs-lint split `turbo-html2pdf` uses.

## JSON workbook schema

The JSON input is the workbook model verbatim with a `schemaVersion` header. A
versioned JSON Schema ships with the package at
[`schema/turbo-xlsx.workbook.schema.json`](schema/turbo-xlsx.workbook.schema.json);
`writeFromJson` / `loadJson` validate against it and reject unknown/invalid shapes.

## Architecture

```
crates/turbo-xlsx-core   Rust core: workbook model → OOXML SpreadsheetML → OPC zip
crates/turbo-xlsx-napi   napi-rs binding → published as `turbo-xlsx` on npm
crates/turbo-xlsx-py     PyO3/maturin binding → `pip install turbo-xlsx` (abi3 wheels)
crates/turbo-xlsx-wasm   wasm-bindgen browser build (`turbo-xlsx-wasm`)
schema/                  versioned JSON Schema for the workbook model
benches/competitive      perf + conformance harness vs exceljs / SheetJS
tools/cc-check           cyclomatic-complexity gate (cc < 6), sibling of scripts/cc-check.js
```

All three bindings expose the same surface over the one core — Node (`turbo-xlsx`),
Python (`import turbo_xlsx`), browser (wasm). The Python `WorkbookWriter` mirrors
the Node streaming writer; the wasm build returns `{ xlsx: Uint8Array, diagnostics }`.

The core is `#![forbid(unsafe_code)]`, depends only on `serde`/`serde_json`, emits
the OOXML directly (strings inline, so per-row work is O(1) for streaming), and
packages a deterministic **STORED** OPC zip — no DEFLATE dependency. Output opens
cleanly in Excel / LibreOffice / openpyxl.

## Benchmarks

Three harnesses live under [`benches/`](benches):

- **Native perf** — a `criterion` bench (`crates/turbo-xlsx-core/benches/write.rs`,
  run `cargo bench`) measuring the writer in isolation against the spec's targets,
  plus a phase profiler (`--features bench-internals --bench hotspot`).
- **Node competitive + conformance** — [`benches/competitive`](benches/competitive)
  comparing turbo-xlsx vs `exceljs` vs `xlsx` (SheetJS), and reading every generated
  file back through one independent reader to see which formatting survives.
- **Python competitive** — [`benches/competitive-py`](benches/competitive-py)
  comparing turbo-xlsx vs `XlsxWriter` vs `openpyxl`.

**turbo-xlsx is the fastest writer in both Node and Python**, on both a small
styled sheet and a 50k-row payroll-scale sheet.

### Native (criterion, release)

The writer in isolation (model → bytes), `cargo bench`:

| Workload | Target | Measured |
|---|---|---|
| 1,000 rows × 20 cols, styled, `write` | < 50 ms | **~4 ms** ✓ |
| 50,000 rows × 30 cols (full write) | < 1.5 s | **~0.13 s** ✓ |

### Node — turbo-xlsx vs `xlsx` (SheetJS) vs `exceljs`

(Node 24, darwin/arm64, release addon — indicative, reproduce locally.)

| 1k × 20 styled | time | 50k × 30 | time |
|---|---|---|---|
| **turbo-xlsx** | **1.6 ms** | **turbo-xlsx** | **0.13 s** |
| xlsx (SheetJS) | 17.4 ms | xlsx (SheetJS) | 2.06 s |
| exceljs | 65.2 ms | exceljs | 4.98 s |

turbo-xlsx is **~11× faster than SheetJS** on 1k and **~16× faster** on 50k. The
path is `WorkbookWriter.writeColumns`: numeric columns are passed as a
`Float64Array` that crosses the N-API boundary as one buffer copy (zero per-cell
FFI, no JSON, no per-cell deserialize), and each column's number format is interned
once. A micro-profiler (`benches/competitive/src/profile.mjs`) showed the per-row
JSON path was bottlenecked on serde's internally-tagged `Cell` deserialize; the
columnar path skips it. The addon also uses mimalloc for the small-alloc churn.
(`writeRowsJson` — V8 stringify a chunk, parse once in Rust — remains available for
callers who already have row objects.)

### Python — turbo-xlsx vs `XlsxWriter` vs `openpyxl`

(Python 3, release wheel — `benches/competitive-py`.)

| 1k × 20 styled | time | 50k × 30 | time |
|---|---|---|---|
| **turbo-xlsx** | **4.4 ms** | **turbo-xlsx** | **0.23 s** |
| XlsxWriter | 46 ms | XlsxWriter | 4.12 s |
| openpyxl | 128 ms | openpyxl | 10.3 s |

On the 50k sheet turbo-xlsx is **~18× faster than XlsxWriter** and **~45× faster
than openpyxl**. CPython's `json.dumps` is too slow for the Node trick, so the
Python path uses `WorkbookWriter.write_table` — a per-column type spec plus rows of
*bare scalar values*, with no per-cell dicts and no JSON.

One honest caveat across all of these: turbo-xlsx output is larger because v1
emits a **STORED** (uncompressed) OPC zip — DEFLATE is planned and would close the
size gap (at some speed cost).

The wins came from a profile-guided pass (the `cargo bench --features
bench-internals --bench hotspot` phase profiler): a slice-by-8 table CRC-32
(checksum 725 ms → 26 ms on a 50k sheet), a per-column number-format cache that
collapses millions of per-cell `xf` interns + format-code rebuilds, and an
allocation-free per-cell XML writer — together cutting the native 50k write **~17×**
(2.2 s → ~0.13 s).

### Conformance / compatibility matrix

Same feature-rich workbook generated by each library, read back through `exceljs`:

| feature | turbo-xlsx | exceljs | xlsx (SheetJS) |
|---|---|---|---|
| string / currency / percent / date values | ✓ | ✓ | ✓ |
| currency + percent number formats | ✓ | ✓ | ✓ |
| negative-in-red | ✓ | ✓ | ✓ |
| bold header / header fill / bold total | ✓ | ✓ | **✗** |
| frozen header pane | ✓ | ✓ | **✗** |

SheetJS Community preserves values and number formats but **drops fonts, fills and
freeze panes** — the exact gap that motivated this library. Reproduce:

```sh
cd benches/competitive && npm install
node src/conformance.mjs   # writes RESULTS.conformance.md
node --expose-gc src/perf.mjs   # writes RESULTS.perf.md
```

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cc < 6
cargo tarpaulin                                                          # 100% gate
```

The core crate carries a **100% line-coverage gate** (`tarpaulin.toml`,
`fail-under = 100`); the napi cdylib shim is excluded (it cannot be
line-instrumented) and kept minimal and mechanical, with every branch pushed into
the covered core.

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — build, the full gate, and the rules of the road.
- [`RELEASING.md`](RELEASING.md) — tag-driven npm + PyPI publish + local builds.
- [`CHANGELOG.md`](CHANGELOG.md) — release notes.

## Non-goals (v1)

No XLSX parsing/import (write-only), no charts, no HTML/Jinja templating, no
`.xls` (legacy BIFF), no pivot tables / conditional formatting beyond the static
negative-in-red number format, no formulas / cross-sheet references. Embedded
images and password protection are v2 (`WriteOptions.password` is accepted but a
no-op today).

## License

MIT
