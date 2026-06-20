"""End-to-end tests for the turbo_xlsx PyO3 binding.

Writes real workbooks, asserts the OPC zip magic header (``b"PK"``), and
exercises the rows fast-path, the JSON-string entry, the streaming
``WorkbookWriter`` + ``create_writer``, ``write_full``'s diagnostics tuple, and
the typed ``TurboXlsxError`` (duplicate sheet name + bad fill color).
"""

import json

import pytest

import turbo_xlsx as x

# A workbook with a string cell and a currency cell (integer minor units).
WORKBOOK = {
    "sheets": [
        {
            "name": "Pay",
            "rows": [
                {
                    "cells": [
                        {"type": "string", "value": "Alice"},
                        {
                            "type": "currency",
                            "value": 123456,
                            "currency": {"code": "MXN", "locale": "es-MX"},
                        },
                    ]
                }
            ],
        }
    ]
}


def test_write_returns_xlsx_bytes():
    data = x.write(WORKBOOK)
    assert isinstance(data, bytes)
    assert data.startswith(b"PK")


def test_write_from_json_string():
    data = x.write_from_json(json.dumps(WORKBOOK))
    assert isinstance(data, bytes)
    assert data.startswith(b"PK")


def test_write_from_json_value():
    data = x.write_from_json(WORKBOOK)
    assert data.startswith(b"PK")


def test_write_rows_fast_path():
    data = x.write_rows(
        {
            "sheetName": "Fast",
            "rows": [{"cells": [{"type": "number", "value": 42}]}],
        }
    )
    assert data.startswith(b"PK")


def test_write_full_returns_bytes_and_list():
    data, lints = x.write_full(WORKBOOK)
    assert data.startswith(b"PK")
    assert isinstance(lints, list)


def test_write_with_opts_meta():
    data = x.write(WORKBOOK, {"meta": {"title": "T", "author": "A"}})
    assert data.startswith(b"PK")


def test_streaming_writer():
    w = x.WorkbookWriter()
    w.start_sheet({"name": "Stream"})
    w.write_row({"cells": [{"type": "string", "value": "row1"}]})
    w.write_row({"cells": [{"type": "number", "value": 1.5}]})
    w.end_sheet()
    data, lints = w.finish()
    assert data.startswith(b"PK")
    assert isinstance(lints, list)


def test_create_writer_streaming():
    w = x.create_writer({"locale": "es-MX", "meta": {"title": "T"}})
    w.start_sheet({"name": "S"})
    w.write_row({"cells": [{"type": "boolean", "value": True}]})
    data, _ = w.finish()
    assert data.startswith(b"PK")


def test_write_rows_json_throughput_path():
    w = x.create_writer({"locale": "es-MX"})
    w.start_sheet({"name": "Bulk"})
    chunk = [
        {
            "cells": [
                {"type": "string", "value": f"r{i}"},
                {"type": "currency", "value": i * 100, "currency": {"code": "MXN"}},
            ]
        }
        for i in range(200)
    ]
    w.write_rows_json(json.dumps(chunk))
    w.end_sheet()
    data, _ = w.finish()
    assert data.startswith(b"PK")


def test_write_table_typed_fast_path():
    # Bare scalar rows + a per-column type spec; openpyxl-validate the result so
    # malformed markup or a type mismatch cannot pass silently.
    openpyxl = pytest.importorskip("openpyxl")
    import io

    w = x.create_writer({"locale": "es-MX"})
    w.start_sheet({"name": "T"})
    columns = [
        {"type": "string"},
        {"type": "currency", "currency": {"code": "MXN", "locale": "es-MX"}},
        {"type": "number", "format": {"decimals": 2}},
        {"type": "percent", "decimals": 1},
        {"type": "boolean"},
        {"type": "date"},
    ]
    rows = [
        ["Alice", 123456, 7.5, 0.16, True, "2026-06-20"],
        ["Bob", 654321, 1.0, 0.5, False, None],  # trailing None -> blank cell
    ]
    w.write_table(columns, rows)
    w.end_sheet()
    data, _ = w.finish()
    ws = openpyxl.load_workbook(io.BytesIO(data)).active
    assert ws["A1"].value == "Alice"
    assert abs(ws["B1"].value - 1234.56) < 0.005
    assert "#,##0.00" in ws["B1"].number_format
    assert ws["C1"].value == 7.5
    assert abs(ws["D1"].value - 0.16) < 0.005 and "%" in ws["D1"].number_format
    assert ws["E1"].value is True
    assert ws["F1"].value is not None  # a real date
    assert ws["F2"].value is None  # blank


def test_write_table_bad_column_type_raises():
    w = x.create_writer()
    w.start_sheet({"name": "S"})
    with pytest.raises(x.TurboXlsxError):
        w.write_table([{"type": "bogus"}], [[1]])


def test_openpyxl_roundtrip_conformance():
    # A real Excel reader fully parses the OOXML — this catches malformed XML
    # (e.g. a cell tag missing its closing '>') that a PK-magic check would miss.
    openpyxl = pytest.importorskip("openpyxl")
    import io

    wb = {
        "locale": "es-MX",
        "sheets": [
            {
                "name": "Conf",
                "freeze": {"rows": 1},
                "rows": [
                    {
                        "cells": [
                            {
                                "type": "string",
                                "value": "Dept",
                                "style": {"font": {"bold": True}, "fill": "#dddddd"},
                            },
                            {"type": "string", "value": "Gross"},
                        ]
                    },
                    {
                        "cells": [
                            {"type": "string", "value": "Eng"},
                            {
                                "type": "currency",
                                "value": 1234567,
                                "currency": {"code": "MXN", "locale": "es-MX", "negative": "red-parens"},
                            },
                        ]
                    },
                    {
                        "cells": [
                            {"type": "string", "value": "Eng2"},
                            {
                                "type": "currency",
                                "value": 7654321,
                                "currency": {"code": "MXN", "locale": "es-MX", "negative": "red-parens"},
                            },
                        ]
                    },
                ],
            }
        ],
    }
    sheet = openpyxl.load_workbook(io.BytesIO(x.write(wb))).active
    assert sheet.title == "Conf"
    assert sheet.freeze_panes == "A2"
    assert sheet["A1"].value == "Dept" and sheet["A1"].font.bold is True
    # currency: integer minor units / 100, and the negative-in-red number format.
    assert abs(sheet["B2"].value - 12345.67) < 0.005
    assert abs(sheet["B3"].value - 76543.21) < 0.005
    assert "#,##0.00" in sheet["B2"].number_format
    assert "Red" in sheet["B2"].number_format


def test_finish_twice_raises():
    w = x.create_writer()
    w.start_sheet({"name": "S"})
    w.finish()
    with pytest.raises(x.TurboXlsxError) as exc_info:
        w.finish()
    assert isinstance(exc_info.value.code, str)


def test_duplicate_sheet_name_raises():
    wb = {"sheets": [{"name": "Dup"}, {"name": "Dup"}]}
    with pytest.raises(x.TurboXlsxError) as exc_info:
        x.write(wb)
    assert exc_info.value.code == "DuplicateSheetName"


def test_bad_fill_color_raises():
    wb = {
        "sheets": [
            {
                "name": "S",
                "rows": [
                    {
                        "cells": [
                            {
                                "type": "string",
                                "value": "x",
                                "style": {"fill": "nope"},
                            }
                        ]
                    }
                ],
            }
        ]
    }
    with pytest.raises(x.TurboXlsxError) as exc_info:
        x.write(wb)
    assert exc_info.value.code == "BadColor"
