// Conformance for embedded-image support across the N-API surface, run against
// the BUILT addon. Images live entirely in the core; this guards that the JS
// type surface + the createWorkbook builder route them through correctly and the
// resulting OPC zip carries the drawing/media parts.

import assert from "node:assert/strict";
import { test } from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const xlsx = require("../index.js");

/** Assert a result Buffer is an OPC zip and contains every expected part name. */
function assertParts(buf, mustContain) {
  assert.ok(Buffer.isBuffer(buf), "result is a Buffer");
  assert.equal(buf.subarray(0, 2).toString(), "PK", "is a zip");
  const text = buf.toString("latin1");
  for (const needle of mustContain) {
    assert.ok(text.includes(needle), `contains ${needle}`);
  }
}

// "hello"/"world" base64 — valid, non-empty payloads.
const PNG = "aGVsbG8=";
const JPEG = "d29ybGQ=";

test("write() embeds a two-cell anchored image", () => {
  const buf = xlsx.write({
    sheets: [
      {
        name: "S1",
        rows: [{ cells: [{ type: "string", value: "hi" }] }],
        images: [
          {
            data: PNG,
            format: "png",
            anchor: { kind: "twoCell", from: { col: 0, row: 0 }, to: { col: 3, row: 6 } },
            alt: "logo",
          },
        ],
      },
    ],
  });
  assertParts(buf, [
    "xl/media/image1.png",
    "xl/drawings/drawing1.xml",
    "xl/worksheets/_rels/sheet1.xml.rels",
    "xdr:twoCellAnchor",
    'descr="logo"',
  ]);
});

test("createWorkbook builder addImage routes through to the package", () => {
  const wb = xlsx.createWorkbook();
  wb.addSheet("Pics")
    .addRows([{ cells: [{ type: "number", value: 1 }] }])
    .addImage({
      data: JPEG,
      format: "jpeg",
      anchor: { kind: "oneCell", at: { col: 1, row: 1 }, width: 120, height: 80 },
    });
  const buf = wb.build();
  assertParts(buf, [
    "xl/media/image1.jpeg",
    "xl/drawings/drawing1.xml",
    "xdr:oneCellAnchor",
    'cx="1143000"', // 120 px * 9525 EMU
  ]);
  // toJson reflects the staged image.
  const model = wb.toJson();
  assert.equal(model.sheets[0].images.length, 1);
  assert.equal(model.sheets[0].images[0].format, "jpeg");
});

test("invalid image anchor surfaces a typed fault", () => {
  assert.throws(
    () =>
      xlsx.write({
        sheets: [
          {
            name: "S1",
            rows: [],
            images: [
              {
                data: PNG,
                format: "png",
                anchor: { kind: "twoCell", from: { col: 3, row: 3 }, to: { col: 1, row: 1 } },
              },
            ],
          },
        ],
      }),
    (err) => err.code === "BadImage",
  );
});

test("image-free workbook has no drawing parts (regression)", () => {
  const buf = xlsx.write({
    sheets: [{ name: "Plain", rows: [{ cells: [{ type: "string", value: "x" }] }] }],
  });
  const text = buf.toString("latin1");
  assert.ok(!text.includes("xl/drawings/"), "no drawings");
  assert.ok(!text.includes("xl/media/"), "no media");
});
