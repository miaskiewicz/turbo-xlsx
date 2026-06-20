# Contributing to turbo-xlsx

A Rust core (`turbo-xlsx-core`) with three thin binding crates (N-API, PyO3,
wasm). All writing logic lives in the core, which carries a **100% line-coverage
gate**; the bindings are deliberately minimal marshaling shims.

## Prerequisites

- **Rust** stable (the toolchain is pinned in `rust-toolchain.toml`, incl. the
  `wasm32-unknown-unknown` target).
- **Node** ≥ 18 and **pnpm** 9 (`corepack enable` or `npm i -g pnpm@9`).
- For the bindings/benches: **maturin** + **pytest** (Python), **wasm-pack**
  (browser), and `cargo install cargo-tarpaulin` for coverage.

## Build

```sh
cargo build --workspace                                   # all crates (host)
cargo build -p turbo-xlsx-napi --release \
  && node crates/turbo-xlsx-napi/scripts/copy-addon.mjs   # the Node addon
```

## The gate — run before every PR

```sh
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cyclomatic complexity < 6
cargo tarpaulin                                                          # 100% core coverage
pnpm lint && pnpm format:check                                          # oxlint + biome on JS/TS
node --test crates/turbo-xlsx-napi/__test__/*.test.mjs                  # napi conformance + e2e
```

Optional binding suites (need their toolchains):

```sh
wasm-pack build crates/turbo-xlsx-wasm --target nodejs --out-dir pkg-node \
  && node --test crates/turbo-xlsx-wasm/__test__/conformance.test.mjs
maturin develop --release --manifest-path crates/turbo-xlsx-py/Cargo.toml \
  && pytest crates/turbo-xlsx-py/tests -q
```

## Rules of the road

- **All new core code needs tests** that exercise every branch (the gate is
  100%). Prefer inline `#[cfg(test)]` modules for private helpers; integration
  tests in `crates/turbo-xlsx-core/tests/` for the public surface.
- **Test for well-formedness, not just substrings.** A `contains()` check missed
  a malformed-XML bug once; worksheet tests assert exact cell bytes + an
  `assert_well_formed` walk, and the Python suite round-trips output through
  `openpyxl` (a real reader).
- **Keep every function under cyclomatic complexity 6** (`tools/cc-check`).
- **Output must stay byte-deterministic.** Same input → identical bytes (STORED
  zip, fixed timestamp, inline strings).
- **`index.js` / `index.d.ts` are hand-maintained** to mirror the `#[napi]`
  exports — do not let `napi build` overwrite them (codegen is redirected to the
  git-ignored `index.generated.*`).
- **Bindings stay thin.** Push every branch into the covered core; the binding
  crates are excluded from the coverage gate precisely because they do nothing but
  marshal.

## Benchmarks

- `cargo bench` — native criterion (write path).
- `cargo bench --features bench-internals --bench hotspot` — phase profiler.
- `benches/competitive` (Node) and `benches/competitive-py` (Python) — vs the
  competition, plus a conformance matrix. See each directory's README.

## Releasing

See [`RELEASING.md`](RELEASING.md) for the tag-driven npm + PyPI publish.
