/* turbo-xlsx N-API binding — JS entry point.
 *
 * Loads the platform-native addon and wraps the native `write` / `writeFromJson`
 * / `writeRows` / `createWriter` surface so that fatal faults surface as a typed
 * `TurboXlsxError` (carrying `.code`) instead of a bare Error. Non-fatal lints
 * are returned by the native layer and attached to the returned Buffer as
 * `.diagnostics` (and returned in full by the streaming writer's `finish`).
 *
 * Also implements the imperative `createWorkbook()` CRUD builder purely in JS:
 * a spreadsheet is data, so the builder assembles/edits a plain workbook object
 * and hands it to the native `write` at `build()` time — no native state.
 *
 * Do NOT let `napi build` overwrite this file — its plain loader has neither the
 * `TurboXlsxError` wrapper nor the builder. The build scripts redirect codegen
 * to `index.generated.js` (git-ignored) via `--js`.
 */

"use strict";

const { existsSync } = require("node:fs");
const { join } = require("node:path");

// --- locate the native addon -------------------------------------------------
function isMusl() {
  if (!process.report || typeof process.report.getReport !== "function") {
    try {
      const ldd = require("node:child_process").execSync("which ldd").toString().trim();
      return require("node:fs").readFileSync(ldd, "utf8").includes("musl");
    } catch {
      return true;
    }
  }
  const { glibcVersionRuntime } = process.report.getReport().header;
  return !glibcVersionRuntime;
}

// The platform suffix @napi-rs/cli uses for the bundled `.node` filenames.
function napiPlatform() {
  const { platform, arch } = process;
  if (platform === "darwin") return `darwin-${arch}`;
  if (platform === "win32") return `win32-${arch}-msvc`;
  if (platform === "linux") return `linux-${arch}-${isMusl() ? "musl" : "gnu"}`;
  return `${platform}-${arch}`;
}

function addonName() {
  if (process.platform === "darwin") return "libturbo_xlsx_napi.dylib";
  if (process.platform === "win32") return "turbo_xlsx_napi.dll";
  return "libturbo_xlsx_napi.so";
}

function loadNative() {
  const candidates = [
    join(__dirname, `turbo-xlsx-napi.${napiPlatform()}.node`),
    join(__dirname, "turbo-xlsx-napi.node"),
    join(__dirname, "..", "..", "target", "release", addonName()),
    join(__dirname, "..", "..", "target", "debug", addonName()),
  ];
  for (const p of candidates) {
    if (existsSync(p)) return require(p);
  }
  throw new Error(
    "turbo-xlsx-napi: native addon not found. Run `napi build --release` (or " +
      "`cargo build -p turbo-xlsx-napi --release`) first. Looked in:\n  " +
      candidates.join("\n  "),
  );
}

const native = loadNative();

// --- typed error -------------------------------------------------------------
const SENTINEL = "TURBO_XLSX_ERR:";

/** A fatal write fault, carrying a stable machine-readable `.code`
 *  (e.g. `"DuplicateSheetName"`, `"BadCellRef"`). Thrown by every entry point. */
class TurboXlsxError extends Error {
  constructor(payload) {
    super(payload.message);
    this.name = "TurboXlsxError";
    this.code = payload.code;
  }
}

/** If `err` is a sentinel-encoded native fault, rethrow it typed; else as-is. */
function rethrow(err) {
  const msg = err && typeof err.message === "string" ? err.message : "";
  const at = msg.indexOf(SENTINEL);
  if (at === -1) throw err;
  try {
    throw new TurboXlsxError(JSON.parse(msg.slice(at + SENTINEL.length)));
  } catch (parsed) {
    if (parsed instanceof TurboXlsxError) throw parsed;
    throw err;
  }
}

function guard(fn) {
  return (...args) => {
    try {
      return fn(...args);
    } catch (err) {
      rethrow(err);
    }
  };
}

// A native result is `{ xlsx: Buffer, diagnostics: [...] }`. Top-level write*
// functions return the Buffer with `.diagnostics` attached for the rare caller
// that wants the lints; the streaming `finish` returns the full result object.
function toBuffer(result) {
  const buf = result.xlsx;
  buf.diagnostics = result.diagnostics;
  return buf;
}

const write = guard((workbook, opts) => toBuffer(native.write(workbook, opts)));
const writeFromJson = guard((input, opts) => toBuffer(native.writeFromJson(input, opts)));
const writeRows = guard((input, opts) => toBuffer(native.writeRows(input, opts)));

// Parse (xlsx -> JSON/CSV/Markdown) ships only in the `turbo-xlsx-parse` build;
// in the lean `turbo-xlsx` package `native.parse` is absent and `parse` throws.
const parse =
  typeof native.parse === "function"
    ? guard((data, opts) => native.parse(data, opts))
    : () => {
        throw new Error(
          "parse is not available in this build — install the `turbo-xlsx-parse` package",
        );
      };

// --- streaming writer --------------------------------------------------------
/** Wrap a native streaming writer so each method rethrows a typed error. */
function wrapWriter(w) {
  return {
    startSheet: guard((sheet) => w.startSheet(sheet)),
    writeRow: guard((row) => w.writeRow(row)),
    writeRowsJson: guard((rowsJson) => w.writeRowsJson(rowsJson)),
    writeColumns: guard((columns) => w.writeColumns(columns)),
    endSheet: guard(() => w.endSheet()),
    finish: guard(() => w.finish()),
  };
}

const createWriter = guard((opts) => wrapWriter(native.createWriter(opts)));

// --- imperative CRUD builder (pure JS) ---------------------------------------
/** A chainable builder over one sheet object. */
function sheetBuilder(sheet) {
  const api = {
    setColumns(cols) {
      sheet.columns = cols;
      return api;
    },
    addRows(rows) {
      sheet.rows.push(...rows);
      return api;
    },
    insertRows(at, rows) {
      sheet.rows.splice(at, 0, ...rows);
      return api;
    },
    updateRow(at, row) {
      sheet.rows[at] = row;
      return api;
    },
    updateCell(rowAt, colKeyOrIdx, cell) {
      const idx = typeof colKeyOrIdx === "number" ? colKeyOrIdx : columnIndex(sheet, colKeyOrIdx);
      sheet.rows[rowAt].cells[idx] = cell;
      return api;
    },
    deleteRows(at, count) {
      sheet.rows.splice(at, count ?? 1);
      return api;
    },
    merge(range) {
      (sheet.merges ??= []).push(range);
      return api;
    },
    freeze(opts) {
      sheet.freeze = opts;
      return api;
    },
    setTotalsRow(row) {
      sheet.rows.push({ ...row, isTotal: true });
      return api;
    },
    addImage(image) {
      (sheet.images ??= []).push(image);
      return api;
    },
  };
  return api;
}

/** Resolve a column key to its index via the sheet's column `key`s. */
function columnIndex(sheet, key) {
  const idx = (sheet.columns ?? []).findIndex((c) => c.key === key);
  if (idx === -1)
    throw new TurboXlsxError({ code: "BadCellRef", message: `unknown column key ${key}` });
  return idx;
}

/** Create an imperative workbook builder. Assemble/edit, then `build()`. */
function createWorkbook(opts) {
  const wb = { schemaVersion: "1.0", sheets: [] };
  if (opts && opts.locale) wb.locale = opts.locale;
  const api = {
    loadJson(input) {
      const parsed = typeof input === "string" ? JSON.parse(input) : input;
      Object.assign(wb, parsed);
      wb.sheets ??= [];
      return api;
    },
    addSheet(name, sheetOpts) {
      const sheet = { name, columns: [], rows: [], ...sheetOpts };
      wb.sheets.push(sheet);
      return sheetBuilder(sheet);
    },
    getSheet(name) {
      const sheet = wb.sheets.find((s) => s.name === name);
      return sheet ? sheetBuilder(sheet) : undefined;
    },
    removeSheet(name) {
      wb.sheets = wb.sheets.filter((s) => s.name !== name);
      return api;
    },
    toJson() {
      return wb;
    },
    build(writeOpts) {
      return write(wb, writeOpts);
    },
  };
  return api;
}

module.exports = {
  write,
  writeFromJson,
  writeRows,
  createWriter,
  createWorkbook,
  parse,
  TurboXlsxError,
};
module.exports.default = module.exports;
