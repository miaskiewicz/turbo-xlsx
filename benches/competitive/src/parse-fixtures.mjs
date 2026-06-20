// Fixtures for the PARSE side: we write `.xlsx` files with *other* libraries
// (SheetJS and ExcelJS), which DEFLATE-compress their parts the way Excel does,
// then read them back with turbo-xlsx's hand-rolled parser. This is the real
// test — turbo's writer emits STORED (uncompressed) parts, so round-tripping our
// own output never exercises the inflater. Parsing a SheetJS/ExcelJS file does.

import ExcelJS from "exceljs";
import * as XLSX from "xlsx";

// A neutral grid with the cases most likely to break a parser: unicode, embedded
// commas/quotes, empty strings, zero, negatives, large/fractional numbers, and
// booleans. Shared-string reuse is forced by repeating "repeat". Dates are
// deliberately excluded: a Date here serializes through each writer's *local*
// timezone, so the reference value is tz-dependent noise, not a parser signal —
// serial→ISO date conversion is proven exactly in the core unit tests instead.
export function sampleGrid() {
  return [
    ["id", "name", "amount", "ratio", "active", "score", "note"],
    [1, "Alice", 1234.56, 0.125, true, 88, "repeat"],
    [2, 'O’Brien, "Bob"', -42, 0, false, -1.5, 'commas, "quotes" ☃'],
    [3, "", 0.1, 1, true, 0, "repeat"],
    [4, "Zoë", 9999999.99, 0.9999, false, 100000, "tail"],
  ];
}

// SheetJS, DEFLATE forced on (`compression: true`) — the common real-world shape.
export function viaSheetJS(grid) {
  const ws = XLSX.utils.aoa_to_sheet(grid, { cellDates: true });
  const wb = XLSX.utils.book_new();
  XLSX.utils.book_append_sheet(wb, ws, "Data");
  return XLSX.write(wb, { type: "buffer", bookType: "xlsx", compression: true });
}

// ExcelJS (always DEFLATEs) — a second, independently-quirky writer.
export async function viaExcelJS(grid) {
  const wb = new ExcelJS.Workbook();
  const ws = wb.addWorksheet("Data");
  for (const row of grid) ws.addRow(row);
  return Buffer.from(await wb.xlsx.writeBuffer());
}

// SheetJS's own read, normalized to a 2D grid — the reference values turbo must
// match. `cellDates` returns real Dates; `defval: null` keeps blank cells aligned.
export function readSheetJS(buf) {
  const wb = XLSX.read(buf, { type: "buffer", cellDates: true });
  const ws = wb.Sheets[wb.SheetNames[0]];
  return XLSX.utils.sheet_to_json(ws, { header: 1, raw: true, defval: null });
}

// A wide numeric+string grid for the perf comparison.
export function perfGrid(rows) {
  const out = [["id", "label", "a", "b", "c", "d", "e", "flag"]];
  for (let i = 0; i < rows; i++) {
    out.push([i, `row-${i}`, i * 1.5, i - 7, i % 100, i / 3, -i, i % 2 === 0]);
  }
  return out;
}
