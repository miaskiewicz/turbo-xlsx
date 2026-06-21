# turbo-xlsx — contributor & release notes

Native structured-workbook-model → formatted XLSX (Rust core) shipped to npm via
an N-API binding. The XLSX peer of `turbo-html2pdf`; same Rust-core + napi-rs
blueprint, same `turbo-*` packaging.

## Layout

| Path | What |
| --- | --- |
| `crates/turbo-xlsx-core` | Rust core: model → OOXML SpreadsheetML → OPC zip. **100% coverage gate.** |
| `crates/turbo-xlsx-napi` | napi-rs binding, published as **`turbo-xlsx`** on npm. Excluded from coverage. |
| `crates/turbo-xlsx-py` | PyO3/maturin binding, published as **`turbo-xlsx-rs`** on PyPI (abi3 wheels; import name `turbo_xlsx`). Excluded from coverage. |
| `crates/turbo-xlsx-wasm` | wasm-bindgen browser build (`turbo-xlsx-wasm`). Excluded from coverage. |
| `schema/` | versioned JSON Schema for the workbook model (also shipped in the npm tarball). |
| `tools/cc-check` | cyclomatic-complexity gate (cc < 6), own workspace, excluded from coverage. |

All three bindings (napi/py/wasm) are thin marshaling shims over the same core and
deliberately minimal/mechanical; every branch lives in the covered core. Each is a
cdylib tarpaulin cannot line-instrument, so all are excluded from the coverage
gate and listed in `tarpaulin.toml`. Their hot path is the JSON-string streaming
entry (`WorkbookWriter.writeRowsJson` / equivalent) that skips per-cell FFI.

Core internals: `model` (typed workbook + serde), `numfmt` (locale/currency/date
→ OOXML format codes + ISO→serial), `style` (cell→row→column resolution +
`styles.xml` interning), `worksheet` (sheetN.xml; **inline strings** so per-row
work is O(1)), `zip` (deterministic STORED OPC zip, no DEFLATE dep), `package`
(OPC parts), `writer` (streaming), `validate` (semantic + JSON fail-closed).

## Pre-commit / pre-tag gate (all must pass; CI re-runs them)

```
RUSTFLAGS="-D warnings" cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cc < 6
cargo tarpaulin                                                          # 100% gate
node --test crates/turbo-xlsx-napi/__test__/*.test.mjs                  # napi conformance
```

The coverage gate is **100%** (`tarpaulin.toml` `fail-under = 100`, `Llvm` engine
pinned so host and Linux agree). New core code needs a test that exercises every
branch — prefer inline `#[cfg(test)]` modules for private helpers, integration
tests in `crates/turbo-xlsx-core/tests/` for the public surface. The napi crate is
excluded from the gate: keep it a thin, mechanical marshaling layer and push every
branch down into the covered core.

The `js` CI job runs `pnpm lint` (oxlint) + `pnpm format:check` (biome). Keep
`index.js` / `index.d.ts` in biome style (2-space, double quotes, semicolons,
trailing commas, 100 width).

## Bindings: hand-maintained surface

`index.js` and `index.d.ts` are **hand-maintained** to mirror the `#[napi]`
exports (richer than napi's auto-codegen). Do NOT let `napi build` overwrite them
— the build scripts redirect codegen to `index.generated.{js,d.ts}` (git-ignored)
via `--js`/`--dts`. The imperative `createWorkbook` CRUD builder lives entirely in
`index.js` (a spreadsheet is data — the builder assembles/edits a plain workbook
object and hands it to native `write` at `build()`), so there is no native builder
state to keep in sync.

## Release runbook (tag-driven)

Two independent tag prefixes (mirrors `turbo-html2pdf`):

| Tag | Publishes |
| --- | --- |
| `vX.Y.Z` | **npm**: `turbo-xlsx` + `turbo-xlsx-parse` (5-platform napi matrix) + `turbo-xlsx-wasm` + `turbo-xlsx-wasm-parse` (browser). Secret **`NPM_TOKEN`**. Also **crates.io**: `turbo-xlsx-core`, gated on **`CARGO_REGISTRY_TOKEN`** (`release-crates.yml`). |
| `pyvX.Y.Z` | **PyPI**: `turbo-xlsx-rs` + `turbo-xlsx-rs-parse` (maturin abi3 wheels + sdist; import name stays `turbo_xlsx` — PyPI rejects `turbo-xlsx` as too close to `turboxlsx`). Secret **`PYPI_TOKEN`** (publish self-skips if unset). |

> All publish workflows are wired: `release.yml` (npm napi + wasm, both
> base/parse variants), `release-py.yml` (PyPI, both variants), and
> `release-crates.yml` (crates.io core). The musl napi build drops mimalloc (a
> static mimalloc segfaults under musl Node) — see the `cfg(not(target_env =
> "musl"))` gate in the napi crate.

Bump the same `X.Y.Z` in **all** of these before tagging (not auto-synced):

- `Cargo.toml` → `[workspace.package] version` (all crates inherit it)
- `crates/turbo-xlsx-napi/package.json` → `version`
- `crates/turbo-xlsx-py/pyproject.toml` → `version`

Cosmetic strings to refresh if present: the `## Status` line in the napi
`README.md` (ships in the tarball). `Cargo.lock` regenerates on the next build —
commit the churn. Sweep for stragglers:
`grep -rn "OLD.VERSION" --include="*.json" --include="*.toml" --include="*.md" . | grep -vE "node_modules|/target/|Cargo.lock"`

Then:

```
git push origin main
git tag -a vX.Y.Z -m "..."
git push origin vX.Y.Z
```

Verify after: `npm view turbo-xlsx@X.Y.Z version` and `… dist.tarball`.

## Design decisions (v1)

- **STORED zip, not DEFLATE.** Excel reads stored OPC zips; storing keeps the
  writer dependency-free and byte-deterministic. DEFLATE is a future size win.
- **Inline strings, no shared-string table.** Keeps per-row work O(1) for the
  streaming writer and the package one part smaller.
- **Currency values are integer minor units.** `123456` + `decimals: 2` →
  `1,234.56`. Matches the ledger money contract; off floating point until the
  final divide at emit.
- **No formulas / cross-sheet refs.** Pure rows→spreadsheet writer; totals are
  pre-computed values. Multiple independent sheets, never referencing each other.
- **Country-agnostic.** `numfmt` maps `locale`+`code` to OOXML format codes; no
  currency or locale is hardcoded, unknown pairings fall back (symbol = ISO code,
  prefix placement).

**Password protection** ships via `WriteOptions.password` — ECMA-376 Agile
Encryption (AES-256-CBC + SHA-512 KDF + HMAC, CFB/OLE2 container) behind the core
`encrypt` feature, using the RustCrypto stack. The feature is **always on** for the
napi/py/wasm/MCP bindings (orthogonal to the `parse` variant axis) and excluded
from the coverage gate like `parse`; it is verified by a `msoffcrypto-tool`
round-trip. Encrypting is non-deterministic (random salts/keys).

Deferred to v2: embedded images/logos, DEFLATE compression.
