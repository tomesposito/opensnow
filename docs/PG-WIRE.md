# PostgreSQL wire endpoint

Start the hosted endpoint with `opensnow-mcp --pg 127.0.0.1:5434`.
Set `OPENSNOW_CONTROL_DATABASE_URL` to the control-plane PostgreSQL URI.
Set `OPENSNOW_WAREHOUSE` to the warehouse directory. Its default is
`opensnow-warehouse`. Add `--http 127.0.0.1:8080` to serve HTTP and
PostgreSQL wire traffic in the same process with the same control plane.

Use cleartext only on loopback for local testing. Set both
`OPENSNOW_PG_TLS_CERT` and `OPENSNOW_PG_TLS_KEY` to PEM files for a
production listener. The certificate file can include the certificate
chain. Setting one without the other fails startup. A listener with TLS
configured requires clients to negotiate TLS before sending credentials.
A listener without TLS rejects SSL negotiation, so clients requiring
TLS fail to connect.

## Connect

The host and port are the `--pg` listener. Use the tenant name as the
database and the account login as the user. Supply an API token for that
tenant and login, or the user's account password. Password sessions have
read scope. Token sessions use the token's scopes, with write scope
including read. Authentication failures return `28P01` with the message
`authentication failed`.

```text
postgresql://LOGIN:PASSWORD@HOST:PORT/TENANT?sslmode=verify-full
postgresql://LOGIN:PASSWORD@127.0.0.1:5434/TENANT?sslmode=disable
```

Percent-encode credential and tenant components when building a URI.
Configure the client's trust store for your TLS certificate. The server
uses the cleartext-password authentication exchange inside TLS. If the
BI tool asks for a schema, use the namespace returned by
`SELECT current_schema()`.

## Query and browse

The endpoint supports simple queries and text-format prepared queries.
Every warehouse statement runs through the same tenant SQL guard, scope
checks, cost caps, and dollar metering as HTTP execute. The wire adapter
accepts quotes around lowercase warehouse identifiers when removing
them preserves the parsed statement. Other quoted identifiers stay
unsupported. Bind parameters use unspecified type OIDs. Parameter values become SQL
strings with escaped quotes, plain numeric literals, or SQL NULL. Describe a bound SELECT portal to
get its columns. The endpoint executes that SELECT once and reuses the
result for Execute.

Session commands and catalog probes do not consume warehouse usage:

- `SET` is accepted as a no-op. Wire encoding uses UTF-8, UTC, and ISO
  dates. `SHOW` supports `server_version`, `server_encoding`,
  `client_encoding`, `DateStyle`, `TimeZone`, `integer_datetimes`, and
  `standard_conforming_strings`.
- Single-expression SELECT probes support `version()`,
  `current_database()`, `current_user`, `session_user`, `user`,
  `current_schema()`, and `pg_backend_pid()`.
  `pg_catalog.set_config` echoes its Boolean third argument without
  changing settings. `format_type` resolves supported OIDs.
- `information_schema.tables` and `information_schema.columns`
  expose the tenant's own tables and columns, with simple projections,
  equality and LIKE filters, and ordering. Nullability reflects Iceberg's
  required field flag.
- Basic `pg_class` and `pg_namespace` join queries and
  `pg_attribute` queries expose relation and column metadata. Relation
  OIDs are synthetic and derived from the sorted tenant table list.
  The plain psql 16 `\dt` query is supported. Catalog ownership is
  unavailable, so its Owner column is NULL.

Unsupported catalog shapes, isolation-level probes, binary formats,
COPY, and FunctionCall return `0A000`. Cancel requests close without
canceling the query. GSS encryption is unavailable.

`BEGIN` and `START TRANSACTION` enter a protocol transaction block.
`COMMIT` and `ROLLBACK` end it. Each warehouse statement executes
independently, and a block adds no atomicity. ROLLBACK cannot undo a
completed write. After a statement error, end the failed block before
issuing further warehouse queries.

## Result types

The wire endpoint returns the same Arrow display strings as HTTP execute.
A SQL NULL uses the protocol's NULL value.

| Arrow type | PostgreSQL type | OID |
| --- | --- | ---: |
| Boolean | boolean | 16 |
| Int8, Int16 | smallint | 21 |
| Int32, UInt8, UInt16 | integer | 23 |
| Int64, UInt32, UInt64 | bigint | 20 |
| Float32 | real | 700 |
| Float64 | double precision | 701 |
| Decimal128, Decimal256 | numeric | 1700 |
| Utf8, LargeUtf8, Null, other types | text | 25 |
| Date32, Date64 | date | 1082 |
| Timestamp | timestamp without time zone | 1114 |
| Time32, Time64 | time without time zone | 1083 |
| Binary | bytea | 17 |

## Stock driver proof

Use a test tenant. With a write token, create a table and load the fixture:

```sql
CREATE TABLE facts (dept BIGINT NOT NULL, amount BIGINT);
INSERT INTO facts VALUES (1, 10), (1, 30), (2, 50);
```

Set `OPENSNOW_PG_URL` to the tenant connection string. Run the script
from a checkout with Node.js and `pg` installed. Set
`OPENSNOW_PG_MODULE` to the absolute `pg` package directory if Node
cannot resolve `pg` from the script's directory.

```sh
node crates/opensnow-mcp/tests/pgwire-driver-check.mjs
```

The script checks login and UTC setup, catalog discovery, and a
parameterized query using the OpenSight executor's array row mode and
text result parser. The hosted integration test can also provision the
fixture and run the script with `cargo test -p opensnow-mcp --test pgwire
stock_pg_driver --offline --locked -- --ignored --nocapture`.
