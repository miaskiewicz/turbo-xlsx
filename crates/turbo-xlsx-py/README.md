# turbo-xlsx (Python)

Native workbook-model-to-XLSX writer — a [PyO3](https://pyo3.rs) binding over the
`turbo-xlsx` Rust core. Turn a structured workbook into a formatted `.xlsx`
(OOXML SpreadsheetML, OPC-zipped). Write-only and country-agnostic: locale and
ISO-4217 currency code are inputs, never hardcoded.

```python
import turbo_xlsx as x

wb = {
    "sheets": [
        {
            "name": "Pay",
            "rows": [
                {"cells": [
                    {"type": "string", "value": "Alice"},
                    {"type": "currency", "value": 123456,
                     "currency": {"code": "MXN", "locale": "es-MX"}},
                ]}
            ],
        }
    ]
}

data = x.write(wb)            # -> bytes, starts with b"PK" (xlsx is a zip)
assert data.startswith(b"PK")
```

## API

- `write(workbook, opts=None) -> bytes` — one-shot from a workbook dict.
- `write_full(workbook, opts=None) -> (bytes, list)` — also returns lint diagnostics.
- `write_from_json(input, opts=None) -> bytes` — `input` is a JSON string or value.
- `write_rows(input, opts=None) -> bytes` — fast-path: one sheet from typed
  columns + rows (`{sheetName?, locale?, columns?, rows}`).
- `create_writer(opts=None) -> WorkbookWriter` / `WorkbookWriter(locale=None, opts=None)` —
  row-by-row streaming: `start_sheet`, `write_row`, `end_sheet`, `finish() -> (bytes, list)`.

`opts` is an optional dict `{meta: {title, author, subject, company}, locale?}`.

Fatal validate/write faults raise `TurboXlsxError` (with `.code` and `.message`).
Non-fatal lints are *returned*, never raised.
