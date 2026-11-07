# Storage catalog

`opensnow_storage::CatalogConfig` carries a catalog name, URI, and local
warehouse directory. `sqlite` creates the database file if absent without
truncating it. `postgres` accepts a full Postgres URI. `connect` builds a
`StorageCatalog` exposing an `Arc<dyn Catalog>` and a DataFusion
`SessionContext` with the Iceberg provider registered as `ice`. Both backends
use the same SqlCatalog code path, LocalFsStorageFactory, and catalog schema
V1, which supports table registration.

SQLite is for local tests. Postgres is the v1 control plane basis described
in SOLUTION_DESIGN.md Section 3.2 and D-03/D-05. REST and Polaris come later.
The URI schemes postgres and postgresql select DollarNumeric binds because
SQLx Any needs the catalog to rewrite placeholders to $1, $2, and so on.
Other schemes use QMark, the SQLite convention. SQLx enables both drivers
and the Tokio runtime explicitly.

Call `create_namespace`, then use `sql` for CREATE TABLE, INSERT, and SELECT
against `ice.<namespace>.<table>`. SQL execution collects Arrow batches.
The SQL strings and provider wiring stay in storage; no SQL is sent into
the engine crate. Namespace creation and clone registration refresh the
cached provider. Call `refresh` after direct changes through the exposed
catalog handle. Use a Tokio multi-thread runtime with at least two workers:
the upstream provider bridges synchronous table registration to async SQL
catalog calls, so a single-thread runtime can stall on Postgres I/O.

`current_snapshot_id` errors on a table without a current snapshot.
`snapshots` returns ids and millisecond timestamps sorted by timestamp then
id. `scan_snapshot` collects Arrow batches through the Iceberg scan API
pinned to the given snapshot. `resolve_snapshot_as_of` chooses the latest
snapshot at or before the timestamp and errors when none qualifies,
including before the first snapshot. Equal timestamps use the larger
snapshot id as a deterministic tie breaker. `scan_as_of` resolves and scans.

`clone_table` copies source metadata, assigns a fresh UUID, and sets its own
warehouse/namespace/table location. It writes a new metadata JSON under the
clone's metadata directory and registers that file as an independent table.
Snapshots and refs are retained, including the current snapshot. Manifest
lists, manifests, and data files remain shared with the source. The clone
has its own UUID and metadata location; later source appends do not change
its rows. No data files are copied.

Cross-table file reference counting and garbage collection are later work
from Section 3.2's gap list. Expiring source snapshots is not made clone-safe
by this slice. Retention and orphan cleanup must account for shared files
before being used with clones.

The always-running SQLite integration test covers 100 then 50 rows, exact
ids, two snapshots, explicit and timestamp reads, the before-history error,
and a clone with equal nonempty planned file sets. A further source append
proves independence. All test data uses fresh temporary directories.

The Postgres integration test connects to localhost:5433 as postgres using
the postgres maintenance database. It drops and creates the dedicated
opensnow_issue6_test database, runs the same acceptance helper with 3 then
2 rows, and prints `PASS Postgres catalog round trip`. An unreachable server
prints `SKIP Postgres catalog round trip: server unreachable:` plus its error.
Permission, authentication, schema, and query errors fail the test. SqlCatalog has no
public pool-close API. Cleanup drops the scratch database WITH (FORCE) after
the storage handle is dropped, closing remaining pool sessions. Setup also
drops any database left by an interrupted run. Run only one instance of
this test against that server at a time.
