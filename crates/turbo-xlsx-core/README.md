# turbo-xlsx-core

The Rust core of [turbo-xlsx](https://github.com/miaskiewicz/turbo-xlsx): turn a
structured **workbook model** into a formatted `.xlsx` (OOXML SpreadsheetML,
OPC-zipped), and — behind the optional `parse` feature — read one back.

`#![forbid(unsafe_code)]`, dependencies limited to `serde`/`serde_json`. This is
the engine the npm (`turbo-xlsx`), PyPI (`turbo-xlsx-rs`) and wasm bindings wrap;
use it directly from Rust.

```toml
[dependencies]
turbo-xlsx-core = "0.1"                                  # writer only
turbo-xlsx-core = { version = "0.1", features = ["parse"] }  # + XLSX reader
```

## Write

```rust
use turbo_xlsx_core::{write_from_json_str, WriteOptions};

let xlsx = write_from_json_str(
    r#"{"sheets":[{"name":"S","rows":[
        {"cells":[{"type":"string","value":"Alice"},
                  {"type":"currency","value":123456,"currency":{"code":"MXN","locale":"es-MX"}}]}
    ]}]}"#,
    &WriteOptions::default(),
)?.xlsx; // -> Vec<u8>, starts with b"PK"
```

Also `write(&Workbook, ..)`, `write_from_json_value(Value, ..)`, the
`write_rows(..)` fast-path, and the streaming `WorkbookWriter`. Cell types:
`string` / `number` / `currency` (integer minor units) / `percent` / `date`
(ISO or Excel serial) / `boolean` / `blank`. Country-agnostic — locale + ISO-4217
code are inputs. Output is a deterministic **STORED** OPC zip (no DEFLATE dep).

## Parse (`parse` feature)

A dependency-free reader (hand-rolled inflate + zip + XML tokenizer) for `.xlsx`
files — including the DEFLATE-compressed output of Excel / SheetJS / openpyxl.
**~1.85× faster than [calamine](https://crates.io/crates/calamine)**, verified
cell-for-cell against SheetJS / ExcelJS / openpyxl.

```rust
use turbo_xlsx_core::parse::{parse, serialize};

let wb = parse(&bytes)?;                  // ParsedWorkbook (typed cell values)
let json = serialize::to_json_grid(&wb);  // also to_json_typed / to_csv / to_markdown
```

## Design

No formulas, no cross-sheet references — pre-computed typed values in, a
spreadsheet out. The crate carries a **100% line-coverage gate**; the bindings push
every branch down here. See the [repo](https://github.com/miaskiewicz/turbo-xlsx)
for the full surface, benchmarks, and the MCP server.

## License

MIT
