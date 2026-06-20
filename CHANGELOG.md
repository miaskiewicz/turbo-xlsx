# Changelog

All notable changes to **turbo-xlsx** are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-06-20

First release. A native, write-only, country-agnostic XLSX writer (Rust core)
shipped to **npm** (`turbo-xlsx`), **PyPI** (`turbo-xlsx`), and the **browser**
(`turbo-xlsx-wasm`).

### Added

- **Core engine** (`turbo-xlsx-core`): structured workbook model → OOXML
  SpreadsheetML → deterministic OPC zip. `#![forbid(unsafe_code)]`, dependencies
  limited to `serde`/`serde_json`/`thiserror`/`itoa`. **100% line-coverage gate.**
- **Cell types**: `string`, `number`, `currency`, `percent`, `date` (ISO or Excel
  serial), `boolean`, `blank` — all native Excel types (Excel sorts/sums/filters).
- **Accountant formatting**: currency per locale + ISO-4217 code (no hardcoding),
  thousands separators, negative-in-red / parens, locale dates, styled headers,
  bold totals rows (`isTotal`), grouped/outlined columns, merges, freeze panes.
- **Five entry modes**: declarative `write`, `writeFromJson` (string or value,
  validated fail-closed against a shipped JSON Schema), `writeRows` fast-path, the
  row-by-row streaming `WorkbookWriter` (`createWriter`), and the imperative
  `createWorkbook` CRUD builder (JS).
- **Bindings**: N-API (`turbo-xlsx` on npm, prebuilt `.node` for 5 targets +
  musl), PyO3/maturin abi3 (`turbo-xlsx` on PyPI), and `wasm-bindgen`
  (`turbo-xlsx-wasm`).
- **Throughput paths** — the fastest XLSX writer measured in both Node and Python:
  - `WorkbookWriter.writeColumns` (napi): numeric columns as a `Float64Array`
    crossing N-API as one zero-copy buffer; format interned once per column.
  - `WorkbookWriter.write_table` (Python): per-column type spec + bare scalar rows,
    no per-cell dicts and no `json.dumps`.
  - `WorkbookWriter.writeRowsJson` / `write_rows_json` (all bindings): stringify a
    chunk, parse once in Rust.
- **Performance** (release, darwin/arm64; reproduce with the harnesses):
  - Node 1k × 20: **1.6 ms** (~11× faster than SheetJS); 50k × 30: **0.13 s** (~16×).
  - Python 1k × 20: **4.4 ms** (~10× faster than XlsxWriter); 50k × 30: **0.23 s** (~18×).
  - Native 50k full write **~0.13 s** (cut ~17× over the first working version).
  - Implementation: slice-by-8 table CRC-32, per-column number-format cache,
    allocation-free per-cell XML writer, mimalloc in the addon.
- **Parser** (optional `parse` feature, dependency-free — hand-rolled DEFLATE
  inflater + OPC-zip reader + XML tokenizer): read an `.xlsx` (incl. the
  DEFLATE-compressed files Excel/SheetJS/openpyxl produce) → **JSON** (values grid
  or round-trippable typed model), **CSV** (RFC-4180), or **Markdown**. Exposed as
  `parse(...)` in all three bindings. Each binding ships a writer-only base package
  and a `…-parse` variant (the with/without split mirrors `turbo-html2pdf`'s fonts;
  wasm 188 KB → 211 KB gzipped). Verified **cell-for-cell against SheetJS and
  openpyxl** on their own DEFLATEd output.
- **MCP server** (`turbo-xlsx-mcp`): native stdio JSON-RPC 2.0 (no SDK) exposing
  the utilities as agent tools — `write`, `write_rows`, `convert_csv`, `parse`,
  `inspect`, `read_range` — with path-or-base64 binary I/O.
- **Tooling**: cyclomatic-complexity gate (cc ≤ 5), `criterion` + hotspot
  profiler benches, Node + Python competitive harnesses (write **and** parse
  compat/perf), conformance matrix (round-trip through a real reader), CI
  (fmt/clippy/test/coverage + binding conformance), tag-driven release workflows
  for npm + PyPI.

### Known limitations

- Output uses a **STORED** (uncompressed) OPC zip, so files are larger than a
  DEFLATE writer's. DEFLATE is planned.
- The writer is **write-only**; the optional parser is a values/types reader (it
  recovers cell values, types and dates — not fonts, fills or freeze panes). No
  formulas, no cross-sheet references, no charts, no embedded images, no `.xls`.
- `WriteOptions.password` is accepted but a no-op (XLSX encryption is v2).

[0.1.0]: https://github.com/miaskiewicz/turbo-xlsx/releases/tag/v0.1.0
