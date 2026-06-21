# turbo-xlsx

Fast native **structured-workbook-model → formatted XLSX** for Node — a Rust core
exposed via `napi-rs`. Accountant-grade number formats (currency per locale,
thousands separators, negative-in-red/parens), styled headers, bold totals,
grouped/outlined columns, merges, freeze panes, and a streaming writer for huge
sheets. **Country-agnostic** (locale + ISO-4217 code are inputs) and
**deterministic**. The `turbo-xlsx-parse` build adds an `.xlsx` reader.

## Status

`v0.1.2`. Prebuilt `.node` addons ship for linux x64 gnu/musl, linux arm64,
darwin arm64, win32 x64 msvc.

## Install

```sh
npm install turbo-xlsx          # lean writer
npm install turbo-xlsx-parse    # writer + XLSX reader (adds parse())
```

Two variants (like `turbo-html2pdf`'s with/without fonts): the base `turbo-xlsx`
is write-only; `turbo-xlsx-parse` is the same plus an `.xlsx` **reader**. The
`parse()` function exists only in the `-parse` build.

## Quick start

```ts
import { write } from "turbo-xlsx";

const xlsx = write({
  locale: "es-MX",
  sheets: [{
    name: "Resumen",
    freeze: { rows: 1 },
    rows: [
      { cells: [{ type: "string", value: "Depto", style: { font: { bold: true } } }, { type: "string", value: "Bruto" }] },
      { cells: [{ type: "string", value: "Ingeniería" }, { type: "currency", value: 1234567, currency: { code: "MXN", locale: "es-MX", negative: "red-parens" } }] },
      { isTotal: true, cells: [{ type: "string", value: "Total" }, { type: "currency", value: 9876543, currency: { code: "MXN" } }] },
    ],
  }],
}, { meta: { title: "Reporte" } });
// `xlsx` is a Buffer (the .xlsx); currency values are integer minor units (cents).
```

## API

- `write(workbook, opts?) → Buffer` — one-shot, declarative object.
- `writeFromJson(stringOrObject, opts?) → Buffer` — JSON in, validated fail-closed.
- `writeRows({ sheetName?, columns, rows }, opts?) → Buffer` — single-sheet fast-path.
- `createWriter(opts?) → WorkbookWriter` — `startSheet`/`writeRow`/`endSheet`/`finish()` for huge sheets.
- `createWorkbook(opts?) → WorkbookBuilder` — imperative CRUD (`addSheet`, `addRows`, `updateCell`, `merge`, `freeze`, `setTotalsRow`) → `build()`.
- `parse(data, opts?) → string` — **`turbo-xlsx-parse` build only.** Read an
  `.xlsx` (incl. DEFLATE-compressed) into JSON (values grid or `{ typed: true }`
  model), `{ format: "csv" }`, or `{ format: "md" }`. ~1.85× faster than calamine,
  cell-for-cell verified against SheetJS / ExcelJS / openpyxl.

Returned Buffers carry the run's non-fatal `.diagnostics`; the streaming
`finish()` returns `{ xlsx, diagnostics }`. Fatal faults throw a typed
`TurboXlsxError` with a stable `.code`. Full types in `index.d.ts`; the JSON
workbook schema in `schema/turbo-xlsx.workbook.schema.json`.

## License

MIT
