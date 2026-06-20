// One adapter per library. Each turns the neutral workloads into that library's
// model and returns a `.xlsx` Buffer, so perf + conformance compare identical
// logical content. The currency number-format we target is the accountant
// red-in-parens convention; the conformance harness checks which libraries keep
// it (and the fonts/fills/freeze) through a round-trip.

import { createRequire } from "node:module";
import ExcelJS from "exceljs";
import * as XLSX from "xlsx";
import { toMajor } from "./workloads.mjs";

const require = createRequire(import.meta.url);
const turbo = require("../../../crates/turbo-xlsx-napi/index.js");

const HEADER_FILL = "#dddddd";
const CURRENCY_NUMFMT = '"$"#,##0.00;[Red]("$"#,##0.00)';
const PERCENT_NUMFMT = "0.00%";
const DATE_NUMFMT = "dd/mm/yyyy";

// ---- turbo-xlsx -------------------------------------------------------------
function turboCell(cell) {
  if (cell.kind === "string") {
    const style = cell.header ? { font: { bold: true }, fill: HEADER_FILL } : undefined;
    return { type: "string", value: cell.value, style };
  }
  if (cell.kind === "currency") {
    return {
      type: "currency",
      value: cell.value,
      currency: { code: "MXN", locale: "es-MX", negative: cell.negative ?? "minus" },
    };
  }
  if (cell.kind === "percent") return { type: "percent", value: cell.value, decimals: 2 };
  if (cell.kind === "date") return { type: "date", value: cell.value, format: { kind: "date" } };
  return { type: "blank" };
}

function turboRow(row) {
  return { cells: row.map(turboCell), isTotal: row.some((c) => c.total) || undefined };
}

// Batch path — the whole workbook crosses the boundary at once. Used for the
// small/feature cases where holding the model is fine.
function turboWrite(rows, { freeze } = {}) {
  const sheet = { name: "Bench", rows: rows.map(turboRow) };
  if (freeze) sheet.freeze = { rows: 1 };
  return turbo.write({ locale: "es-MX", sheets: [sheet] });
}

// Columnar fast path — the header row goes through writeRow; the uniform data
// columns are transposed into Float64Arrays (numeric, zero-copy across N-API) +
// a string[] label column and pushed once via writeColumns. No per-cell JS
// objects, no JSON, no per-cell deserialize.
function turboColumns(rows, { freeze } = {}) {
  const w = turbo.createWriter({ locale: "es-MX" });
  const meta = { name: "Bench" };
  if (freeze) meta.freeze = { rows: 1 };
  w.startSheet(meta);
  const header = rows[0];
  w.writeRow({
    cells: header.map((c) => ({
      type: "string",
      value: c.value,
      style: { font: { bold: true }, fill: "#dddddd" },
    })),
  });
  const data = rows.slice(1);
  const nrows = data.length;
  const ncols = header.length;
  const labels = Array.from({ length: nrows });
  for (let r = 0; r < nrows; r++) labels[r] = data[r][0].value;
  const columns = [{ kind: "string", strings: labels }];
  for (let c = 1; c < ncols; c++) {
    const arr = new Float64Array(nrows);
    for (let r = 0; r < nrows; r++) arr[r] = data[r][c].value;
    columns.push({ kind: "currency", currency: { code: "MXN", locale: "es-MX" }, numbers: arr });
  }
  w.writeColumns(columns);
  w.endSheet();
  return w.finish().xlsx;
}

// ---- exceljs ----------------------------------------------------------------
function excelValue(cell) {
  if (cell.kind === "currency") return toMajor(cell.value);
  if (cell.kind === "percent") return cell.value;
  if (cell.kind === "date") return new Date(cell.value);
  if (cell.kind === "blank") return null;
  return cell.value;
}

function styleExcelCell(target, cell) {
  if (cell.kind === "currency") target.numFmt = CURRENCY_NUMFMT;
  if (cell.kind === "percent") target.numFmt = PERCENT_NUMFMT;
  if (cell.kind === "date") target.numFmt = DATE_NUMFMT;
  if (cell.header || cell.total) target.font = { bold: true };
  if (cell.header) {
    target.fill = { type: "pattern", pattern: "solid", fgColor: { argb: "FFDDDDDD" } };
  }
}

async function excelWrite(rows, { freeze } = {}) {
  const wb = new ExcelJS.Workbook();
  const ws = wb.addWorksheet("Bench");
  rows.forEach((row, r) => {
    const added = ws.addRow(row.map(excelValue));
    row.forEach((cell, c) => styleExcelCell(added.getCell(c + 1), cell));
    void r;
  });
  if (freeze) ws.views = [{ state: "frozen", ySplit: 1 }];
  return Buffer.from(await wb.xlsx.writeBuffer());
}

// ---- SheetJS (xlsx) community ----------------------------------------------
function sheetjsCell(cell) {
  if (cell.kind === "currency") return { v: toMajor(cell.value), t: "n", z: CURRENCY_NUMFMT };
  if (cell.kind === "percent") return { v: cell.value, t: "n", z: PERCENT_NUMFMT };
  if (cell.kind === "date") return { v: new Date(cell.value), t: "d", z: DATE_NUMFMT };
  if (cell.kind === "blank") return undefined;
  return { v: cell.value, t: "s" };
}

function sheetjsWrite(rows, { freeze } = {}) {
  const ws = {};
  rows.forEach((row, r) => {
    row.forEach((cell, c) => {
      const out = sheetjsCell(cell);
      if (out) ws[XLSX.utils.encode_cell({ r, c })] = out;
    });
  });
  const lastRow = rows.length - 1;
  const lastCol = Math.max(...rows.map((row) => row.length)) - 1;
  ws["!ref"] = XLSX.utils.encode_range({ s: { r: 0, c: 0 }, e: { r: lastRow, c: lastCol } });
  if (freeze)
    ws["!freeze"] = { xSplit: "0", ySplit: "1", topLeftCell: "A2", activePane: "bottomLeft" };
  const wb = XLSX.utils.book_new();
  XLSX.utils.book_append_sheet(wb, ws, "Bench");
  return XLSX.write(wb, { type: "buffer", bookType: "xlsx", cellDates: true });
}

export const adapters = [
  {
    name: "turbo-xlsx",
    perf: (rows) => turboColumns(rows),
    feature: (rows) => turboWrite(rows, { freeze: true }),
  },
  {
    name: "exceljs",
    perf: (rows) => excelWrite(rows),
    feature: (rows) => excelWrite(rows, { freeze: true }),
  },
  {
    name: "xlsx (SheetJS)",
    perf: (rows) => sheetjsWrite(rows),
    feature: (rows) => sheetjsWrite(rows, { freeze: true }),
  },
];
