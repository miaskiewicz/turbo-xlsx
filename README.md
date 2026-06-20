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

## Parse (XLSX → JSON / CSV / Markdown)

The `…-parse` build adds the inverse direction: read an `.xlsx` — including the
**DEFLATE-compressed** files Excel, SheetJS and openpyxl produce — into JSON, CSV,
or Markdown. It is **dependency-free**: a hand-rolled inflater (RFC-1951 puff),
OPC-zip reader, and XML tokenizer, no new crates.

```ts
import { parse } from "turbo-xlsx-parse"; // the parse-enabled npm build

const bytes = fs.readFileSync("report.xlsx");

parse(bytes);                              // JSON values grid (default)
parse(bytes, { typed: true });             // round-trippable typed JSON model
parse(bytes, { format: "csv", sheet: "Q1" }); // RFC-4180 CSV of one sheet
parse(bytes, { format: "md" });            // GitHub-flavored Markdown table
```

```python
import turbo_xlsx as tx                    # the turbo-xlsx-rs-parse wheel
tx.parse(data, format="csv", sheet="Q1")   # -> str
```

- **JSON** comes in two shapes: a plain **values grid** (`{ sheets: [{ name, rows:
  [[…]] }] }`) for quick consumption, or a **typed** model (`typed: true`,
  `schemaVersion` header) that round-trips back through the writer.
- **CSV / Markdown** render one sheet (the first by default, or `sheet` by name).
- Cells are typed on the way out — numbers stay numbers, booleans booleans, and
  date-formatted serials become ISO-8601 strings.

Correctness is verified **cell-for-cell against SheetJS and openpyxl** on their own
DEFLATEd output (see [Benchmarks](#parse--turbo-xlsx-vs-sheetjs--openpyxl)), on top
of the core's round-trip + edge-case unit tests.

## MCP server

`turbo-xlsx-mcp` is a native **MCP** (Model Context Protocol) server — hand-rolled
JSON-RPC 2.0 over stdio, no SDK — exposing the Excel utilities as tools an agent can
call. Binary I/O is path-or-base64: every reader takes `path` **or** `dataBase64`;
every writer takes an optional `out` path (else it returns base64).

| tool | does |
|---|---|
| `write` | a full workbook object → `.xlsx` |
| `write_rows` | typed columns + rows fast-path → `.xlsx` |
| `convert_csv` | CSV text/file → `.xlsx` (numbers inferred; ZIP codes stay text) |
| `parse` | `.xlsx` → JSON (grid or typed) / CSV / Markdown |
| `inspect` | per-sheet name + row/column dimensions |
| `read_range` | a sheet's values, or just an `A1:C3` window |

```jsonc
// stdin (newline-delimited JSON-RPC)
{"jsonrpc":"2.0","id":1,"method":"initialize"}
{"jsonrpc":"2.0","id":2,"method":"tools/call",
 "params":{"name":"parse","arguments":{"path":"report.xlsx","format":"md"}}}
```

```sh
cargo build -p turbo-xlsx-mcp --release   # binary: target/release/turbo-xlsx-mcp
```

Register it like any stdio MCP server (e.g. `claude mcp add turbo-xlsx -- \
/path/to/turbo-xlsx-mcp`).

## Architecture

```
crates/turbo-xlsx-core   Rust core: workbook model → OOXML SpreadsheetML → OPC zip
                         (+ optional `parse` feature: XLSX → JSON/CSV/Markdown reader)
crates/turbo-xlsx-napi   napi-rs binding → published as `turbo-xlsx` on npm
crates/turbo-xlsx-py     PyO3/maturin binding → `pip install turbo-xlsx-rs` (abi3 wheels; import turbo_xlsx)
crates/turbo-xlsx-wasm   wasm-bindgen browser build (`turbo-xlsx-wasm`)
crates/turbo-xlsx-mcp    MCP server (stdio JSON-RPC 2.0): write / parse / convert / inspect
schema/                  versioned JSON Schema for the workbook model
benches/competitive      perf + conformance harness vs exceljs / SheetJS (write + parse)
benches/competitive-py   perf + conformance harness vs XlsxWriter / openpyxl (write + parse)
tools/cc-check           cyclomatic-complexity gate (cc ≤ 5), sibling of scripts/cc-check.js
```

Each binding ships in **two variants**, exactly like `turbo-html2pdf`'s with/without
fonts: a lean writer-only base package, and a `…-parse` build that adds the XLSX
**reader** (the `parse` Cargo feature). On npm that is `turbo-xlsx` vs
`turbo-xlsx-parse`; on PyPI `turbo-xlsx-rs` vs `turbo-xlsx-rs-parse` (the import
name stays `turbo_xlsx`; PyPI rejects `turbo-xlsx` as too close to the existing
`turboxlsx`); for wasm `turbo-xlsx-wasm` vs `turbo-xlsx-wasm-parse`. The reader is
off by default so the common write-only install carries no extra code (wasm: 188 KB
→ 211 KB gzipped).

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

### Parse — turbo-xlsx vs SheetJS / openpyxl

The parser is held to a hard bar: read **the other libraries' own DEFLATEd files**
back **cell-for-cell**. The harnesses write a mixed-type grid (unicode, embedded
commas/quotes, empty strings, zero, negatives, large/fractional numbers, booleans)
with SheetJS / ExcelJS / openpyxl, parse it with both turbo and the reference
reader, and diff every cell. Both pass with **zero mismatches** — and turbo is
several times faster reading the same compressed bytes:

| read a DEFLATEd file → value grid | 1,000 rows | 50,000 rows |
|---|---|---|
| **turbo-xlsx** vs **SheetJS** (Node) | **3.1×** faster | **4.0×** faster |
| **turbo-xlsx** vs **openpyxl** (Python) | **8.3×** faster | **9.7×** faster |

(Node 24 / Python 3, darwin/arm64, release builds — indicative, reproduce locally.)

```sh
# Node: build the parse-enabled addon, then run compat + perf
cargo build -p turbo-xlsx-napi --release --features parse
node crates/turbo-xlsx-napi/scripts/copy-addon.mjs
cd benches/competitive && npm install
npm run parse:compat   # cell-for-cell vs SheetJS + ExcelJS
npm run parse:perf     # read speed vs SheetJS

# Python: build the parse-enabled wheel, then run
cd benches/competitive-py && python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt maturin
maturin develop --manifest-path ../../crates/turbo-xlsx-py/Cargo.toml --features parse --release
python parse_compat.py   # cell-for-cell + read speed vs openpyxl
```

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cc ≤ 5
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

The **writer** is write-only and styling-rich; the optional **parser** is a
values/types reader (it extracts cell values, types and dates — not fonts, fills or
freeze panes — and writing remains the primary direction). No charts, no HTML/Jinja
templating, no `.xls` (legacy BIFF), no pivot tables / conditional formatting beyond
the static negative-in-red number format, no formulas / cross-sheet references.
Embedded images and password protection are v2 (`WriteOptions.password` is accepted
but a no-op today).

## License

MIT
