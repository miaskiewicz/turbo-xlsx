"""turbo-xlsx — native workbook-model-to-XLSX writer (PyO3 binding).

Turn a structured workbook dict into a formatted `.xlsx` (OOXML SpreadsheetML,
OPC-zipped). Every entry mode converges on the same typed model and emitter:

    import turbo_xlsx as x

    wb = {
        "sheets": [
            {"name": "S", "rows": [{"cells": [{"type": "string", "value": "hi"}]}]}
        ]
    }
    data = x.write(wb)              # -> bytes, starts with b"PK" (xlsx is a zip)

JSON in (string or value), the rows fast-path, and row-by-row streaming:

    data = x.write_from_json('{"sheets": [...]}')
    data = x.write_rows({"rows": [{"cells": [...]}]})

    w = x.create_writer()
    w.start_sheet({"name": "S"})
    w.write_row({"cells": [{"type": "number", "value": 1}]})
    w.end_sheet()
    data, lints = w.finish()

Fatal validate/write faults raise :class:`TurboXlsxError` (with ``.code`` and
``.message``). Non-fatal lints are *returned* by :func:`write_full` /
:meth:`WorkbookWriter.finish`, never raised.
"""

from ._turbo_xlsx import (  # noqa: F401
    TurboXlsxError,
    WorkbookWriter,
    create_writer,
    write,
    write_from_json,
    write_full,
    write_rows,
)

__all__ = [
    "TurboXlsxError",
    "WorkbookWriter",
    "create_writer",
    "write",
    "write_from_json",
    "write_full",
    "write_rows",
]
