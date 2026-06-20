# Native parse benchmark — turbo-xlsx-core vs calamine

Head-to-head **Rust-vs-Rust** read benchmark: turbo-xlsx-core's dependency-free
parser against [`calamine`](https://crates.io/crates/calamine), the de-facto fast
Rust spreadsheet reader. The Node/Python harnesses compare against SheetJS /
openpyxl (other-language readers); this one is the real "fastest parser" yardstick.

It is its **own workspace** (empty `[workspace]` table, like `tools/cc-check`) so
`calamine` / `zip` never enter the published turbo-xlsx workspace lock or its
coverage/cc gates.

## What it does

1. Builds a mixed-type grid (int / string / float / bool) with turbo's **writer**
   → STORED `.xlsx` bytes.
2. Re-zips it with **DEFLATE** (turbo's writer has no compressor) so both readers
   run their real inflate + zip-walk + XML-parse path on Excel-style bytes.
3. Asserts turbo and calamine see the **same cell count** (a correctness cross-check).
4. Times both reading the same bytes into a fully-materialized grid, N times,
   reports the median + ratio.

## Run

```sh
cargo run --release --manifest-path benches/parse-native/Cargo.toml
```

## Indicative result (darwin/arm64, release)

```
  1000 rows (24KB deflated, 4004 cells):   turbo   2.8ms   calamine   2.7ms   -> 0.95x
 50000 rows (1133KB deflated, 200004 cells): turbo 142ms   calamine 134ms   -> 0.93x
```

turbo-xlsx is at **~0.93–0.98× calamine** — i.e. **parity** (calamine a few percent
faster), which is a strong result for a hand-rolled, zero-dependency inflater + zip
reader + XML tokenizer going up against a mature, widely-optimized crate. Numbers
are indicative and machine-specific — reproduce locally.

## Hotspot profiler (`parse-hotspot`)

Splits turbo's parse into its two phases and times each, so optimization effort
goes where the time actually is:

```sh
cargo run --release --bin parse-hotspot --manifest-path benches/parse-native/Cargo.toml
```

- **Phase A — unzip + inflate** (`parse::read_zip`, exposed under the core's
  `bench-internals` feature): DEFLATE-decompress every part + walk the zip.
- **Phase B — XML + value-build** (full `parse` minus phase A): tokenize the
  sheet/shared-strings XML and materialize typed cell values.

Finding (darwin/arm64): **phase B is ~83%** of parse time, phase A only ~17%. So
the path to beating calamine is the **XML tokenizer + number/value parsing**, not
the inflater — that is where the remaining few-percent gap lives.

```
50000 rows (1133KB deflated):
  A unzip+inflate :   27ms  (17% of turbo)
  B xml+value     :  133ms  (83% of turbo)
  ─ turbo total   :  159ms
    calamine total:  156ms   (turbo is 0.98x calamine)
```
