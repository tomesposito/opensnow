# Worksheets and navigation UI

Slice 1.8, issue #11. The UI is one static page served by the HTTP
transport from slice 1.9. It implements the design package Tom approved
on 2026-11-09: the design draft and mockups for all ten states, with
rulings Q1 to Q6. The page has no build step and no external assets.
The browser holds the caller's token in sessionStorage only.

## The ten states

1. Sign-in. Tenant, login, and password, or an API token. A failed
   sign-in shows one message: sign-in failed, check your credentials.
   It never says which check failed.
2. Empty catalog. A tenant with no tables sees a true empty state. No
   staged or sample data is ever shown.
3. Populated catalog. A tree of the caller's tables, catalog search,
   and a table detail view with columns from the catalog and sample
   rows (first 5) through the authenticated execute path (Q4).
4. Worksheet, empty. Editor, saved worksheets, Run and Save. No
   results and no cost line until a query runs.
5. Worksheet, running. Elapsed time plus the tenant's running totals
   from the cost summary. No exact dollar figure for the running
   query, because that figure only exists when the query completes
   and its cost record is written (Q5).
6. Worksheet, results. Grid, row count, elapsed, bytes, vCPU-seconds,
   and the exact cost in dollars from the query's own cost record.
   Download CSV is built in the browser from the grid.
7. Worksheet, error. The error in plain text. The UI re-reads the
   cost records after a failure: if a new record appeared, its cost
   line is shown, because failed queries are still metered. If none
   appeared, the panel says the query was not executed and no cost
   was recorded, which is the pre-execution rejection case.
8. Cost cap. A cost cap failure renders as its own amber cap state,
   distinct from the red error panel. The editor stays readable.
9. History. The tenant's query history from the cost records (Q3):
   time, SQL, status, rows, elapsed, and cost per row, with a re-run
   button that loads the SQL into a worksheet. Only the caller's own
   tenant records are ever returned.
10. Suspended or deleted tenant. Any 401 after sign-in clears the
    token and returns to the sign-in screen with the same opaque
    failure message as state 1. No catalog, history, or cost data is
    shown.

## Backend additions in this slice

Saved worksheets live server-side in the control-plane Postgres (Q2).
A new `worksheets` table holds id, tenant_id, user_id, name, sql_text,
and timestamps. Every read and write filters by tenant and user in
the query itself, so a caller can only ever see and change their own
worksheets. Updating or deleting another user's worksheet reads as
not found.

New HTTP routes, all on the existing transport:

- `GET /` and `GET /ui` serve the page as text/html.
- `POST /api/sign-in` with tenant, login, and password. Password auth
  returns a read-scope principal, so sign-in issues that user a
  read-scope token with the default 24 hour expiry and returns it
  once. The browser then holds the same revocable, expiring
  credential as any API caller. Every failure is the same 401 body.
- `GET /api/session` returns the tenant name, namespace, login, and
  scopes for the caller's token.
- `POST /api/describe` with a table name returns its columns from
  the catalog in the caller's namespace.
- `POST /api/search` with a query returns the caller's tables whose
  name or column names contain the text, case-insensitive.
- `GET /api/worksheets`, `POST /api/worksheets` (create, or update
  when an id is given), and `POST /api/worksheets/delete`.

## Limits

- No charts (Q6). The Analyse step of the experience sequence is a
  later slice. The grid plus the cost line is the v1 worksheet.
- Password sign-in is read scope. Worksheets run read queries on a
  password session. Write statements need a write-scope token, which
  the UI accepts on the token sign-in form and the control plane
  enforces on every call either way.
- No query cancellation. The transport is one request per connection
  with no cancellation channel, so the running state has no cancel
  button rather than one that does not stop anything.
- Table detail shows columns and sample rows only. Row counts, byte
  sizes, and freshness are not shown, because the catalog layer does
  not measure them and the UI does not invent numbers.
- Sample row previews run through the execute path, so each preview
  is a metered query and appears in history with its cost. That is
  the Q4 ruling applied literally, and it is visible in the detail
  view's cost line.

## Tests

`crates/opensnow-mcp/tests/ui.rs` covers: the served page and its
states, identical opaque 401 bodies for a wrong password, an unknown
tenant, an unknown login, and a suspended tenant; a working read
token from sign-in and its write denial; describe and search scoped
to the caller's namespace; and worksheet create, list, update, and
delete with a second tenant unable to list, update, or delete the
first tenant's worksheets. The hosted tests skip cleanly when
Postgres on localhost:5433 is unreachable.

The live check for this slice ran the real server against a
throwaway database, provisioned a tenant, loaded a table through the
API, and captured the states in a headless browser: sign-in, empty
catalog, populated catalog with detail and sample rows, worksheet
results with the exact cost line, an error, and history.
