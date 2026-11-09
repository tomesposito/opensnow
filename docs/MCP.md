# Local mode, MCP server, and HTTP transport

Slice 1.9, issue #12. The `opensnow-mcp` crate holds three things: a
local mode that embeds the same engine with no account, an MCP server
over the engine and catalog, and the HTTP transport for the hosted
control plane. The worksheets UI in slice 1.8, issue #11, builds on
this transport.

## Local mode

`LocalDb::open(dir, cost_cap_units)` opens a database under one
directory: a SQLite catalog file and a local warehouse. There is no
account, no credential, and no Postgres. Reopening the directory
reopens the database with its tables intact.

Local mode is not a second engine. It registers the same Iceberg
catalog on an `EngineService` tenant, and every statement runs through
the control plane's SQL guard, the same function the hosted path uses:
table names are qualified to the namespace, statement types are
limited to the same set, and anything the guard cannot qualify is
rejected. The namespace is the fixed name `local`. The test suite runs
the same fixture and the same compatibility queries through local mode
and through the hosted authenticated path and compares the rows.

## MCP server

The server speaks JSON-RPC 2.0. It handles `initialize`, `ping`,
`tools/list`, and `tools/call`. Two transports share one handler:

- Stdio, over a local database. This is the default for the
  `opensnow-mcp` binary: one JSON-RPC message per line.
- Streamable HTTP style: `POST /mcp` on the HTTP transport, over the
  hosted control plane, with a Bearer token.

Tools:

| Tool | What it does |
| --- | --- |
| provision_database | Hosted: creates a tenant, its first admin, and a first token in one call, and returns the token once. Local: returns the open local database. |
| list_tables | Tables in the caller's namespace. |
| describe_table | Column names and types for one table. |
| search_schema | Tables and columns whose names contain the query text, case-insensitive. |
| sample_rows | Up to `limit` rows from one table. Default 10, max 100. |
| query | One read-only statement. Write statements are refused. |
| execute_write | One write statement: create, insert, update, delete, drop. Read statements are refused. |
| cost_records | The caller's query log from slice 1.7, with the cost of each query in dollars. |
| cost_summary | The caller's cost totals. |
| docs_search | Lines from the shipped docs that match the query text. |

Query results carry the metered outcome from slice 1.7: rows returned,
vCPU-seconds, elapsed time, the cost record sequence, and the cost in
dollars at the internal rate in docs/COST-MODEL.md. That rate is our
internal cost, not a customer price.

Safety defaults:

- Read and write are separate tools. An agent holding only the `query`
  tool cannot change data through this server.
- The hosted path runs every call through the control plane. The
  token's scope decides: the default token scope is read, and a read
  token gets AccessDenied from `execute_write` at the control plane,
  not just at the tool split.
- Cost caps are per tenant and per agent database. Provisioning takes
  `cost_cap_units`, local mode takes the cap at open, and the engine
  enforces it. A query that hits the cap aborts and leaves a cost
  record with status cost_cap_exceeded.
- Provisioning is one call and returns a working token. The acceptance
  test measures it against the 5 second bar from the design; the
  control plane measured 0.20 seconds in slice 1.6.

## HTTP transport

Minimal HTTP/1.1, one request per connection, JSON in and out. The
binary runs it with `--http <addr>` plus the environment variables
`OPENSNOW_CONTROL_DATABASE_URL` (a Postgres URI) and
`OPENSNOW_WAREHOUSE` (a local warehouse path).

Routes:

- `GET /health` returns `{"status": "ok"}`.
- `POST /api/provision` with name, admin_login, admin_password, and an
  optional cost_cap_units. Returns the tenant id, namespace, and the
  one-time token.
- `POST /api/execute` with a Bearer token and `{"sql": ...}`. Returns
  result rows in the same JSON shape the MCP tools return, plus the
  metered outcome.
- `GET /api/tables`, `GET /api/cost-records`, and
  `GET /api/cost-summary`, all with a Bearer token.
- `POST /mcp` with the JSON-RPC body. A Bearer token is required for
  every tool except provision_database.
- The worksheets UI routes from slice 1.8, issue #11: `GET /` serves
  the UI page, `POST /api/sign-in`, `GET /api/session`,
  `POST /api/describe`, `POST /api/search`, and the worksheet routes
  `GET /api/worksheets`, `POST /api/worksheets`, and
  `POST /api/worksheets/delete`. See docs/WORKSHEETS-UI.md.

Error mapping is plain: 401 for authentication failure (the control
plane's opaque error, unchanged), 403 for access denied and cost cap,
409 for a duplicate tenant or user, 400 for bad input, 404 for an
unknown route, 500 otherwise.

Result rows are encoded as column names and types plus rows of display
strings, with null kept as null. Local and hosted use the same
encoding, so a result reads the same on either path.

## What is not here

- No OAuth. Section 6.3 names OAuth for hosted MCP at launch. This
  slice uses the scoped, revocable Bearer tokens from slice 1.6, which
  the control plane already checks on every call. OAuth is launch work
  on top of that token model, not a replacement for it.
- No MCP Registry or directory listing. Registration in the official
  MCP Registry and the Claude, Cursor, and VS Code directories is a
  launch-day step that needs Tom's accounts. Nothing in this slice
  submits anything anywhere.
- No confirmation prompts inside the server. The guard here is the
  read and write tool split, token scope, namespace isolation, and the
  cost cap. An agent host can still prompt before calling
  execute_write; that is the host's setting, not this server's claim.
- No TLS in this listener. A hosted deploy terminates TLS in front of
  it. This slice is local builds only and deploys nothing.
- docs_search covers only the docs shipped in this repo. It is a
  grounding aid, not a web search.
- Describe reads the table schema from the catalog in the caller's
  own namespace. Sample rows go through the execute path, the same
  choice issue #11 makes.

## Tests

`crates/opensnow-mcp/tests/mcp.rs` covers: the local MCP path end to
end (initialize, tool list, create, insert, query, sample, describe,
schema search, cost records and summary, docs search, provisioning
under 5 seconds), the read and write tool split, a cost cap abort
through the query tool, local against hosted on the same fixture and
compatibility queries with identical rows, and the HTTP routes
including a 401 for a missing token. The hosted tests follow the
control plane pattern and skip cleanly when Postgres on localhost:5433
is unreachable.
