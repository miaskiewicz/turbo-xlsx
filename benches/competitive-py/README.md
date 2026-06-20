# Python competitive harness

Compares **turbo-xlsx** (the PyO3 binding) against
[`openpyxl`](https://pypi.org/project/openpyxl/) (write-only mode) and
[`XlsxWriter`](https://pypi.org/project/XlsxWriter/) on the same styled 1k-row and
50k-row workloads as the Node harness.

turbo-xlsx uses its **typed-table fast path** (`WorkbookWriter.write_table`): a
per-column type spec plus rows of *bare scalar values* — no per-cell dicts and no
`json.dumps`, so it sidesteps CPython's slow object building (which is why the
JSON-string path that wins in V8 loses in CPython).

## Setup

```sh
cd benches/competitive-py
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt maturin
# build turbo-xlsx in RELEASE for honest numbers
maturin develop --release --manifest-path ../../crates/turbo-xlsx-py/Cargo.toml
python perf.py        # prints a table, writes RESULTS.py.md, exits non-zero if not fastest
```

## Indicative result (Python 3.14, darwin/arm64, release)

| 50k × 30 | time | output |
|---|---|---|
| **turbo-xlsx** | **0.23 s** | 58 MB |
| XlsxWriter | 4.12 s | 9.5 MB |
| openpyxl | 10.3 s | 9.5 MB |

turbo-xlsx is **~18× faster than XlsxWriter** and **~45× faster than openpyxl** on
the 50k sheet, and fastest on 1k too. Use the **release** build — a debug
extension is ~25× slower. Output is larger because v1 emits a STORED
(uncompressed) OPC zip (DEFLATE is future work).

`RESULTS.py.md` is generated and git-ignored.
