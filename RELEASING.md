# Releasing turbo-xlsx

Publishing mirrors the sibling **turbo-html2pdf** setup: **tag-driven**, with two
independent tag prefixes. Pushing a tag is the only action that publishes —
nothing is released on a normal `main` push.

| Tag         | Workflow                                | Publishes |
| ----------- | --------------------------------------- | --------- |
| `vX.Y.Z`    | `.github/workflows/release.yml`         | **npm**: `turbo-xlsx` + `turbo-xlsx-parse` (5-platform napi + musl) and `turbo-xlsx-wasm` + `turbo-xlsx-wasm-parse` (browser) |
| `vX.Y.Z`    | `.github/workflows/release-crates.yml`  | **crates.io**: `turbo-xlsx-core` (same tag; gated on `CARGO_REGISTRY_TOKEN`) |
| `pyvX.Y.Z`  | `.github/workflows/release-py.yml`      | **PyPI**: `turbo-xlsx-rs` + `turbo-xlsx-rs-parse` (abi3 wheels + sdist; import `turbo_xlsx`) |

## Required repository secrets

Add these in **GitHub → Settings → Secrets and variables → Actions** before the
first tag:

- **`NPM_TOKEN`** — an npm **Automation** token allowed to publish public,
  unscoped names (`turbo-xlsx`, `turbo-xlsx-parse`, `turbo-xlsx-wasm`,
  `turbo-xlsx-wasm-parse`).
- **`PYPI_TOKEN`** — a PyPI API token. Until it is set, the PyPI `publish` job
  **self-skips** (it still builds the wheels), so the file is safe to merge.
- **`CARGO_REGISTRY_TOKEN`** — a crates.io API token (publishes `turbo-xlsx-core`
  on the `v*` tag). The publish step self-skips when it is unset.

> The `turbo-xlsx-core` crate is standalone (no `path` deps of its own), so it
> publishes to crates.io cleanly; only the core is published — the bindings are
> cdylibs shipped to npm/PyPI and the MCP server is a binary. Consumers opt into
> the reader with `features = ["parse"]`.

## 1. Bump the version — EVERY place (NOT auto-synced)

Set the same `X.Y.Z` in all of these before tagging:

- `Cargo.toml` → `[workspace.package] version` (all four crates inherit it)
- `crates/turbo-xlsx-napi/package.json` → `version`
- `crates/turbo-xlsx-py/pyproject.toml` → `version`

The wasm package version is **auto-stamped** from the git tag in `release.yml` —
do not bump it by hand. `Cargo.lock` regenerates on the next build; commit the churn.

Cosmetic strings to refresh: the `## Status` line in `crates/turbo-xlsx-napi/README.md`
(it ships in the npm tarball) and the new `CHANGELOG.md` section.

Sweep for stragglers before tagging:

```sh
grep -rn "OLD.VERSION" --include="*.json" --include="*.toml" --include="*.md" . \
  | grep -vE "node_modules|/target/|Cargo.lock|pnpm-lock"
```

## 2. Pre-tag gate (all must pass locally; CI re-runs them)

```sh
RUSTFLAGS="-D warnings" cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cc < 6
cargo tarpaulin                                                          # 100% gate
node --test crates/turbo-xlsx-napi/__test__/*.test.mjs                   # napi e2e
```

## 3. Tag + push

```sh
git push origin main                  # push commits first (CI runs on main)
git tag -a vX.Y.Z   -m "turbo-xlsx vX.Y.Z"   # npm  → release.yml
git tag -a pyvX.Y.Z -m "turbo-xlsx vX.Y.Z"   # PyPI → release-py.yml (needs PYPI_TOKEN)
git push origin vX.Y.Z pyvX.Y.Z
```

Watch: `gh run list`. The npm `build-napi` matrix (4 platforms) + the musl
container build take ~10–15 min. Verify after:

```sh
npm view turbo-xlsx@X.Y.Z version
npm view turbo-xlsx-wasm@X.Y.Z version
pip index versions turbo-xlsx          # or check https://pypi.org/project/turbo-xlsx/
```

## Local build (no publish)

```sh
# napi addon (host platform) — drops the .node next to index.js
cargo build -p turbo-xlsx-napi --release && node crates/turbo-xlsx-napi/scripts/copy-addon.mjs

# python wheel (needs maturin in an active venv)
maturin build --release --manifest-path crates/turbo-xlsx-py/Cargo.toml

# browser package (needs wasm-pack + the wasm32 target)
wasm-pack build crates/turbo-xlsx-wasm --target web --out-dir pkg --release
```
