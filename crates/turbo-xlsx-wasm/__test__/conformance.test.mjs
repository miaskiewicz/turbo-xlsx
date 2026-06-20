// turbo-xlsx WASM conformance test (Node target).
//
// Mirrors the N-API surface against the WASM build: it proves the browser
// binding produces a real OPC-zipped `.xlsx` (PK magic) and rejects a fatal
// fault (duplicate sheet name) with a structured `{ code }` error.
//
// HOW TO BUILD THE TESTED ARTIFACT (from crates/turbo-xlsx-wasm):
//   wasm-pack build --target nodejs --out-dir pkg-node
//
// Then run:  node --test __test__/conformance.test.mjs
// The suite is SKIPPED (not failed) when ./pkg-node is not built.

import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const PKG = join(root, "pkg-node", "turbo_xlsx_wasm.js");

const wasm = existsSync(PKG) ? require(PKG) : null;

const WORKBOOK = {
  sheets: [
    {
      name: "Sheet1",
      rows: [
        {
          cells: [
            { type: "string", value: "hello" },
            { type: "number", value: 42 },
          ],
        },
      ],
    },
  ],
};

test("write returns a Uint8Array xlsx starting with PK magic", { skip: !wasm }, () => {
  const result = wasm.write(WORKBOOK, undefined);
  assert.ok(result.xlsx instanceof Uint8Array, "xlsx is a Uint8Array");
  assert.equal(result.xlsx[0], 0x50, "first byte is 'P' (0x50)");
  assert.equal(result.xlsx[1], 0x4b, "second byte is 'K' (0x4B)");
  assert.ok(Array.isArray(result.diagnostics), "diagnostics is an array");
});

test("createWriter + writeRowsJson throughput path", { skip: !wasm }, () => {
  const w = wasm.createWriter(undefined);
  w.startSheet({ name: "Bulk" });
  const chunk = [];
  for (let i = 0; i < 200; i++) {
    chunk.push({
      cells: [
        { type: "string", value: `r${i}` },
        { type: "number", value: i },
      ],
    });
  }
  w.writeRowsJson(JSON.stringify(chunk));
  w.endSheet();
  const result = w.finish();
  assert.ok(result.xlsx instanceof Uint8Array && result.xlsx[0] === 0x50);
});

test("duplicate sheet name rejects with DuplicateSheetName", { skip: !wasm }, () => {
  const dup = {
    sheets: [
      { name: "Dupe", rows: [] },
      { name: "Dupe", rows: [] },
    ],
  };
  assert.throws(
    () => wasm.write(dup, undefined),
    (err) => err && err.code === "DuplicateSheetName",
  );
});
