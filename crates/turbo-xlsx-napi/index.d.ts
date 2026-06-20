/* turbo-xlsx N-API binding — TypeScript surface.
 *
 * Hand-maintained to mirror the `#[napi]` exports in `src/lib.rs` plus the
 * pure-JS `createWorkbook` builder in `index.js`. The shapes below are the
 * contract callers depend on.
 *
 * Do NOT let `napi build` overwrite this file. The build scripts redirect codegen
 * to `index.generated.d.ts` (git-ignored) via `--dts`. */

/** A fatal write fault. Thrown by every entry point. Non-fatal lints are NOT
 *  thrown — they come back as `WriteResult.diagnostics` / `Buffer.diagnostics`. */
export class TurboXlsxError extends Error {
  name: "TurboXlsxError";
  /** Stable machine code, e.g. `"DuplicateSheetName"`, `"BadCellRef"`,
   *  `"InvalidSheetName"`, `"BadColor"`, `"InvalidJson"`, `"SchemaViolation"`,
   *  `"EmptyWorkbook"`. */
  code: string;
}

/** A non-fatal diagnostic (lint) collected during a write. */
export interface Diagnostic {
  /** Stable lint code, e.g. `"ClampedColumnWidth"`, `"DroppedDuplicateMerge"`. */
  code: string;
  message: string;
}

/** The `.xlsx` bytes plus light diagnostics (the streaming `finish` shape). */
export interface WriteResult {
  /** The OOXML SpreadsheetML (OPC-zipped) document. */
  xlsx: Buffer;
  /** Non-fatal diagnostics — never thrown. */
  diagnostics: Diagnostic[];
}

/** The Buffer the top-level `write*` functions return: the `.xlsx` bytes, with
 *  the run's non-fatal `diagnostics` attached as a property. */
export type XlsxBuffer = Buffer & { diagnostics: Diagnostic[] };

/** Document metadata + global options. */
export interface WriteOptions {
  /** Workbook metadata written to the OPC core/app parts. */
  meta?: { title?: string; author?: string; subject?: string; company?: string };
  /** Default locale for the streaming writer (the batch path reads the
   *  workbook's own `locale`). BCP-47, e.g. `"es-MX"`. */
  locale?: string;
  /** AES-style XLSX password protection — accepted but deferred to v2 (no-op). */
  password?: string;
}

// ---- Workbook model ---------------------------------------------------------

export interface Workbook {
  /** Schema version of a JSON workbook (`"1.0"`). */
  schemaVersion?: string;
  /** Default BCP-47 locale for currency/date formats; per-cell override wins. */
  locale?: string;
  /** ≥1; sheet names must be unique. */
  sheets: Sheet[];
}

export interface Sheet {
  /** ≤31 chars, unique, no `: \ / ? * [ ]`. */
  name: string;
  columns?: Column[];
  rows: Row[];
  /** Merged ranges, e.g. `"A1:C1"`. */
  merges?: string[];
  /** Freeze panes — keep header rows / id columns visible while scrolling. */
  freeze?: { rows?: number; cols?: number };
  /** Default outline state for grouped columns. */
  outline?: { columnsCollapsed?: boolean };
}

export interface Column {
  /** A stable key, used by `updateCell(row, key, cell)` in the builder. */
  key?: string;
  /** Character width (Excel width units). Omit for the default. */
  width?: number;
  /** Outline level (1..7) for grouped columns. */
  outlineLevel?: number;
  hidden?: boolean;
  /** Default style for the whole column. */
  style?: CellStyle;
}

export interface Row {
  cells: Cell[];
  /** Outline level (1..7) for grouped/sub-total rows. */
  outlineLevel?: number;
  height?: number;
  /** Bold + top-border totals/footer row (style without restating per cell). */
  isTotal?: boolean;
}

export type Cell =
  | { type: "string"; value: string; style?: CellStyle }
  | { type: "number"; value: number; format?: NumberFormat; style?: CellStyle }
  | { type: "currency"; value: number; currency: CurrencyFormat; style?: CellStyle }
  | { type: "percent"; value: number; decimals?: number; style?: CellStyle }
  | { type: "date"; value: string | number; format?: DateFormat; style?: CellStyle }
  | { type: "boolean"; value: boolean; style?: CellStyle }
  | { type: "blank"; style?: CellStyle };
// NO formula cell type — totals are pre-computed values, no live Excel formulas.

/** Currency formatting — `code` + `locale` are inputs; no hardcoding. A
 *  `currency` cell's `value` is an integer in minor units (e.g. cents). */
export interface CurrencyFormat {
  /** ISO-4217 code, e.g. `"MXN"`, `"USD"`, `"EUR"`, `"BRL"`. */
  code: string;
  /** BCP-47 locale for symbol placement; falls back to sheet/workbook, then "en-US". */
  locale?: string;
  /** Decimal places. Default 2. */
  decimals?: number;
  /** How negatives render. */
  negative?: "red" | "parens" | "minus" | "red-parens";
  /** Show the currency symbol (true, default) or the ISO code (false). */
  symbol?: boolean;
}

export interface NumberFormat {
  decimals?: number;
  /** Thousands grouping. Default true. */
  grouped?: boolean;
  negative?: "red" | "parens" | "minus" | "red-parens";
  /** Escape hatch: a raw Excel number-format code, used verbatim. */
  raw?: string;
}

export interface DateFormat {
  /** Semantic kind. Default `"date"` (locale short date). */
  kind?: "date" | "datetime" | "month-year";
  /** Escape hatch: a raw Excel date code, e.g. `"dd/mm/yyyy"`. */
  raw?: string;
}

export interface CellStyle {
  font?: { bold?: boolean; italic?: boolean; size?: number; color?: string; name?: string };
  /** Background fill `#rrggbb`. */
  fill?: string;
  align?: {
    horizontal?: "left" | "center" | "right";
    vertical?: "top" | "middle" | "bottom";
    wrap?: boolean;
  };
  border?: { top?: BorderEdge; bottom?: BorderEdge; left?: BorderEdge; right?: BorderEdge };
}

export interface BorderEdge {
  style?: "thin" | "medium" | "thick" | "double";
  color?: string;
}

// ---- Top-level functions ----------------------------------------------------

/** One-shot: a complete workbook object → `.xlsx` bytes (with `.diagnostics`). */
export function write(workbook: Workbook, opts?: WriteOptions): XlsxBuffer;

/** JSON in: a JSON string OR a value matching the workbook schema. Validated
 *  fail-closed (throws `TurboXlsxError` with code `InvalidJson`/`SchemaViolation`). */
export function writeFromJson(input: string | object, opts?: WriteOptions): XlsxBuffer;

/** Convenience fast-path: one sheet from typed columns + rows. NOT a CSV
 *  ingester — `rows` are already-typed cells. */
export function writeRows(
  input: { sheetName?: string; locale?: string; columns?: Column[]; rows: Row[] },
  opts?: WriteOptions,
): XlsxBuffer;

/** Streaming writer for large sheets — rows pushed one at a time. */
export function createWriter(opts?: WriteOptions): WorkbookWriter;

/** One column for the columnar fast path (`WorkbookWriter.writeColumns`). Numeric
 *  columns pass `numbers` (a Float64Array — currency values are integer minor
 *  units); string columns pass `strings`. */
export type ColumnInput =
  | { kind: "string"; strings: string[] }
  | { kind: "currency"; currency: CurrencyFormat; numbers: Float64Array }
  | { kind: "number"; format?: NumberFormat; numbers: Float64Array }
  | { kind: "percent"; decimals?: number; numbers: Float64Array };

export interface WorkbookWriter {
  /** Begin a sheet from its metadata (its `rows` are ignored — stream them). */
  startSheet(sheet: Omit<Sheet, "rows"> & { rows?: Row[] }): void;
  /** Stream one row into the open sheet. */
  writeRow(row: Row): void;
  /** Throughput path: stream a chunk of rows from a `JSON.stringify(Row[])`
   *  string, skipping the per-cell N-API object walk `writeRow` pays. Stringify
   *  a bounded chunk in JS and push it; far faster for tens of thousands of rows. */
  writeRowsJson(rowsJson: string): void;
  /** Fastest path: stream a block of columns. Numeric columns carry their values
   *  as a `Float64Array` (one zero-copy buffer crossing, no per-cell FFI or
   *  deserialize); the number format is interned once per column. Currency values
   *  are integer minor units. */
  writeColumns(columns: ColumnInput[]): void;
  /** Close the open sheet (idempotent). */
  endSheet(): void;
  /** Finish all sheets, ZIP the package, return the bytes + diagnostics. */
  finish(): WriteResult;
}

// ---- Imperative builder (CRUD) ----------------------------------------------

/** Create an imperative workbook builder. Assemble/edit, then `build()`. */
export function createWorkbook(opts?: { locale?: string }): WorkbookBuilder;

export interface WorkbookBuilder {
  /** Hydrate from a JSON workbook (string or object), then CRUD on top. */
  loadJson(input: string | object): WorkbookBuilder;
  addSheet(name: string, opts?: Partial<Omit<Sheet, "name" | "rows">>): SheetBuilder;
  getSheet(name: string): SheetBuilder | undefined;
  removeSheet(name: string): WorkbookBuilder;
  /** Serialize the current state to the JSON workbook schema. */
  toJson(): Workbook;
  /** Validate + emit `.xlsx` bytes (with `.diagnostics`). */
  build(opts?: WriteOptions): XlsxBuffer;
}

export interface SheetBuilder {
  setColumns(cols: Column[]): SheetBuilder;
  addRows(rows: Row[]): SheetBuilder;
  insertRows(at: number, rows: Row[]): SheetBuilder;
  updateRow(at: number, row: Row): SheetBuilder;
  updateCell(rowAt: number, colKeyOrIdx: string | number, cell: Cell): SheetBuilder;
  deleteRows(at: number, count?: number): SheetBuilder;
  merge(range: string): SheetBuilder;
  freeze(opts: { rows?: number; cols?: number }): SheetBuilder;
  /** Bold/top-border footer (pre-computed totals). */
  setTotalsRow(row: Row): SheetBuilder;
}
