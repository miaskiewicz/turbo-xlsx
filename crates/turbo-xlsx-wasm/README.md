# turbo-xlsx-wasm

The in-browser build of [turbo-xlsx](https://github.com/miaskiewicz/turbo-xlsx):
a structured **workbook model → formatted `.xlsx`** writer compiled to WebAssembly
(`wasm-bindgen`). Same Rust core as the Node / Python bindings — accountant-grade
number formats, styled headers, freeze panes, a streaming writer — running
client-side with no server round-trip.

## Install

```sh
npm install turbo-xlsx-wasm          # lean writer (~188 KB gzipped)
npm install turbo-xlsx-wasm-parse    # writer + XLSX reader (~211 KB gzipped, adds parse())
```

Two variants, like the napi packages: the base is write-only; `-parse` adds an
`.xlsx` reader. `parse()` exists only in the `-parse` build.

## Use

```js
import init, { write, parse } from "turbo-xlsx-wasm";

await init();

const { xlsx } = write({
  locale: "es-MX",
  sheets: [{ name: "S", rows: [
    { cells: [{ type: "string", value: "Alice" }, { type: "number", value: 42 }] },
  ] }],
});
// xlsx is a Uint8Array (the .xlsx, starts with the PK zip magic).

// parse() — turbo-xlsx-wasm-parse only:
const grid = JSON.parse(parse(xlsx));                 // { sheets: [{ name, rows }] }
const csv = parse(xlsx, { format: "csv" });
```

`write` / `writeFromJson` / `writeRows` / `createWriter` mirror the Node API and
return `{ xlsx: Uint8Array, diagnostics }`. Fatal faults throw a structured
`{ code }` error. See the [repo](https://github.com/miaskiewicz/turbo-xlsx) for
the full workbook schema and benchmarks.

## License

MIT
