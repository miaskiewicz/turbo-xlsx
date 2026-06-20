// Conformance + e2e for the turbo-xlsx N-API surface, run against the BUILT
// addon. Asserts every entry mode produces a valid OPC zip and that fatal faults
// surface as a typed TurboXlsxError.

import assert from "node:assert/strict";
import { test } from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const xlsx = require("../index.js");

/** A zip starts with "PK"; assert that and that expected part names appear. */
function assertXlsx(buf, mustContain = []) {
  assert.ok(Buffer.isBuffer(buf), "result is a Buffer");
  assert.equal(buf.subarray(0, 2).toString(), "PK", "is a zip");
  const text = buf.toString("latin1");
  for (const needle of mustContain) {
    assert.ok(text.includes(needle), `contains ${needle}`);
  }
}

const sampleWorkbook = {
  locale: "es-MX",
  sheets: [
    {
      name: "Resumen",
      columns: [{ key: "dept", width: 24 }, { key: "gross" }],
      freeze: { rows: 1 },
      rows: [
        {
          cells: [
            {
              type: "string",
              value: "Departamento",
              style: { font: { bold: true }, fill: "#dddddd" },
            },
            { type: "string", value: "Bruto" },
          ],
        },
        {
          cells: [
            { type: "string", value: "Ingeniería" },
            {
              type: "currency",
              value: 1234567,
              currency: { code: "MXN", locale: "es-MX", negative: "red-parens" },
            },
          ],
        },
        {
          isTotal: true,
          cells: [
            { type: "string", value: "Total" },
            { type: "currency", value: 9876543, currency: { code: "MXN" } },
          ],
        },
      ],
    },
  ],
};

test("write(workbook) → styled xlsx Buffer with diagnostics", () => {
  const buf = xlsx.write(sampleWorkbook, { meta: { title: "Reporte", company: "Flux" } });
  assertXlsx(buf, ["xl/worksheets/sheet1.xml", "<dc:title>Reporte</dc:title>", 'name="Resumen"']);
  assert.ok(Array.isArray(buf.diagnostics));
});

test("writeFromJson accepts a JSON string and an object identically", () => {
  const fromObj = xlsx.writeFromJson(sampleWorkbook);
  const fromStr = xlsx.writeFromJson(JSON.stringify(sampleWorkbook));
  assertXlsx(fromObj, ["xl/workbook.xml"]);
  assert.equal(fromObj.length, fromStr.length);
});

test("writeRows fast-path", () => {
  const buf = xlsx.writeRows({
    sheetName: "Data",
    locale: "en-US",
    columns: [{ width: 12 }],
    rows: [{ cells: [{ type: "number", value: 42, format: { decimals: 2 } }] }],
  });
  assertXlsx(buf, ['name="Data"']);
});

test("createWriter streams rows", () => {
  const w = xlsx.createWriter({ locale: "pt-PT", meta: { author: "stream" } });
  w.startSheet({ name: "Big", columns: [{ width: 10 }] });
  for (let i = 0; i < 1000; i++) {
    w.writeRow({
      cells: [
        { type: "number", value: i },
        { type: "currency", value: i * 100, currency: { code: "EUR", locale: "pt-PT" } },
      ],
    });
  }
  w.endSheet();
  const result = w.finish();
  assertXlsx(result.xlsx, ['name="Big"']);
  assert.ok(Array.isArray(result.diagnostics));
});

test("createWriter writeRowsJson throughput path", () => {
  const w = xlsx.createWriter({ locale: "es-MX" });
  w.startSheet({ name: "Bulk" });
  const chunk = [];
  for (let i = 0; i < 500; i++) {
    chunk.push({
      cells: [
        { type: "string", value: `r${i}` },
        { type: "currency", value: i * 100, currency: { code: "MXN" } },
      ],
    });
  }
  w.writeRowsJson(JSON.stringify(chunk));
  w.endSheet();
  const result = w.finish();
  assertXlsx(result.xlsx, ['name="Bulk"', "r499"]);
});

test("createWriter writeColumns columnar fast path", () => {
  const w = xlsx.createWriter({ locale: "es-MX" });
  w.startSheet({ name: "Cols" });
  w.writeColumns([
    { kind: "string", strings: ["A", "B", "C"] },
    {
      kind: "currency",
      currency: { code: "MXN", locale: "es-MX" },
      numbers: new Float64Array([111111, 222222, 333333]),
    },
    { kind: "number", format: { decimals: 2 }, numbers: new Float64Array([1.5, 2.5, 3.5]) },
    { kind: "percent", decimals: 1, numbers: new Float64Array([0.1, 0.2, 0.3]) },
  ]);
  w.endSheet();
  const result = w.finish();
  assertXlsx(result.xlsx, ['name="Cols"', "<v>1111.11</v>", "C"]);
});

test("createWorkbook builder CRUD then build", () => {
  const wb = xlsx.createWorkbook({ locale: "es-MX" });
  const sheet = wb.addSheet("S", { freeze: { rows: 1 } });
  sheet
    .setColumns([{ key: "name", width: 20 }, { key: "amount" }])
    .addRows([
      {
        cells: [
          { type: "string", value: "A" },
          { type: "currency", value: 100, currency: { code: "MXN" } },
        ],
      },
      {
        cells: [
          { type: "string", value: "B" },
          { type: "currency", value: 200, currency: { code: "MXN" } },
        ],
      },
    ])
    .insertRows(1, [{ cells: [{ type: "string", value: "inserted" }, { type: "blank" }] }])
    .updateCell(0, "amount", { type: "currency", value: 150, currency: { code: "MXN" } })
    .deleteRows(2, 1)
    .merge("A1:B1")
    .setTotalsRow({
      cells: [
        { type: "string", value: "Total" },
        { type: "currency", value: 150, currency: { code: "MXN" } },
      ],
    });
  assert.equal(wb.toJson().sheets[0].name, "S");
  assert.ok(wb.getSheet("S"));
  assert.equal(wb.getSheet("missing"), undefined);
  const buf = wb.build({ meta: { title: "built" } });
  assertXlsx(buf, ['name="S"', 'mergeCell ref="A1:B1"']);
});

test("loadJson hydrates the builder", () => {
  const wb = xlsx.createWorkbook().loadJson(JSON.stringify(sampleWorkbook));
  assert.equal(wb.toJson().sheets[0].name, "Resumen");
  wb.removeSheet("Resumen");
  assert.equal(wb.toJson().sheets.length, 0);
});

test("fatal faults throw a typed TurboXlsxError", () => {
  const dup = {
    sheets: [
      { name: "X", rows: [] },
      { name: "x", rows: [] },
    ],
  };
  assert.throws(
    () => xlsx.write(dup),
    (err) => {
      assert.ok(err instanceof xlsx.TurboXlsxError);
      assert.equal(err.code, "DuplicateSheetName");
      return true;
    },
  );
  assert.throws(
    () => xlsx.writeFromJson("{ not json"),
    (err) => err.code === "InvalidJson",
  );
  const badColor = {
    sheets: [
      { name: "S", rows: [{ cells: [{ type: "string", value: "x", style: { fill: "nope" } }] }] },
    ],
  };
  assert.throws(
    () => xlsx.write(badColor),
    (err) => err.code === "BadColor",
  );
});

test("unknown column key in updateCell throws", () => {
  const wb = xlsx.createWorkbook();
  const s = wb.addSheet("S");
  s.setColumns([{ key: "a" }]).addRows([{ cells: [{ type: "blank" }] }]);
  assert.throws(
    () => s.updateCell(0, "missing", { type: "blank" }),
    (err) => err.code === "BadCellRef",
  );
});
