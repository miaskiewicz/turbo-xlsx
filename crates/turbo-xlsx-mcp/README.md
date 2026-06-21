# turbo-xlsx-mcp

A native **MCP** (Model Context Protocol) server for Excel utilities — hand-rolled
JSON-RPC 2.0 over stdio, no SDK — exposing the [turbo-xlsx](https://github.com/miaskiewicz/turbo-xlsx)
writer + reader as tools an agent can call.

## Build & run

```sh
cargo build -p turbo-xlsx-mcp --release   # binary: target/release/turbo-xlsx-mcp
```

Register it like any stdio MCP server, e.g. with Claude Code:

```sh
claude mcp add turbo-xlsx -- /path/to/turbo-xlsx-mcp
```

## Tools

| tool | does |
|---|---|
| `write` | a full workbook object → `.xlsx` |
| `write_rows` | typed columns + rows fast-path → `.xlsx` |
| `convert_csv` | CSV text/file → `.xlsx` (numbers inferred; ZIP codes stay text) |
| `parse` | `.xlsx` → JSON (grid or `typed`) / CSV / Markdown |
| `inspect` | per-sheet name + row/column dimensions |
| `read_range` | a sheet's values, or just an `A1:C3` window |

Binary I/O is **path-or-base64**: every reader takes `path` **or** `dataBase64`;
every writer takes an optional `out` path (else it returns `{ base64, bytes }`).

```jsonc
// stdin (newline-delimited JSON-RPC)
{"jsonrpc":"2.0","id":1,"method":"initialize"}
{"jsonrpc":"2.0","id":2,"method":"tools/call",
 "params":{"name":"parse","arguments":{"path":"report.xlsx","format":"md"}}}
```

The server is a thin layer over `turbo-xlsx-core` (with the `parse` feature); all
the real work — and the 100% coverage gate — lives in the core. See the
[repo](https://github.com/miaskiewicz/turbo-xlsx) for the full workbook schema.

## License

MIT
