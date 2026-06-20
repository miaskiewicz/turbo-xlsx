# Releasing turbo-xlsx

Publishing mirrors the sibling **turbo-html2pdf** setup: **tag-driven**, with two
independent tag prefixes. Pushing a tag is the only action that publishes —
nothing is released on a normal `main` push.

| Tag         | Workflow                                | Publishes |
| ----------- | --------------------------------------- | --------- |
| `vX.Y.Z`    | `.github/workflows/release.yml`         | **npm**: `turbo-xlsx` (5-platform napi + musl) and `turbo-xlsx-wasm` (browser build) |
| `pyvX.Y.Z`  | `.github/workflows/release-py.yml`      | **PyPI**: `turbo-xlsx` (maturin abi3 wheels + sdist) |

## Required repository secrets

Add these in **GitHub → Settings → Secrets and variables → Actions** before the
first tag:

- **`NPM_TOKEN`** — an npm **Automation** token allowed to publish public,
  unscoped names (`turbo-xlsx`, `turbo-xlsx-wasm`).
- **`PYPI_TOKEN`** — a PyPI API token. Until it is set, the PyPI `publish` job
  **self-skips** (it still builds the wheels), so the file is safe to merge.

> crates.io is **not** wired up (the core uses a `path` dependency, so it is not
> crates.io-publishable as-is). If you want to publish the Rust crate, switch
> `turbo-xlsx-napi`/`-py`/`-wasm` to a versioned `turbo-xlsx-core` dependency and
> add a `cargo publish` job. Out of scope for v0.1.0.

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
