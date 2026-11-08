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

## Slice 1.4: copy-on-write writer

`StorageCatalog::execute` routes a leading DELETE or UPDATE keyword,
case-insensitively, to the writer. Other statements go through the existing
`sql` method. `sql` itself is unchanged. Mutations return an empty batch list.
The accepted grammar is:

```sql
DELETE FROM ice.<namespace>.<table> [WHERE <predicate>]
UPDATE ice.<namespace>.<table> SET <column> = <expression> [, ...] [WHERE <predicate>]
```

The storage parser validates this grammar before DataFusion evaluates any
expression. Exactly one fully qualified table under `ice` is required.
Unquoted identifiers fold to lowercase. Quoted identifiers retain case.
One optional trailing semicolon and SQL comments are accepted. Aliases,
joins, FROM/USING clauses, subqueries, RETURNING, ORDER BY, LIMIT,
multiple statements, and duplicate assignments are rejected. Column references in mutation expressions are unqualified. Expressions
must be deterministic scalar expressions over the row. Aggregates, windows,
and non-immutable functions are rejected. Unsupported non-mutation statements
retain DataFusion's errors, including MERGE on this provider.

The writer loads one table snapshot, plans its files, and reads each file's
tasks through the Iceberg Arrow reader. In this pinned crate FileScanTask
exposes a path, not the full DataFile. The writer resolves the exact live
DataFile values from that snapshot's manifests. It rejects delete files.
DataFusion MemTables evaluate WHERE and SET over each file's batches.
A NULL predicate is not a match. DELETE keeps non-matching rows, including
NULL predicates. UPDATE assignments all see the original row and affect
matching rows only. Output preserves column order and types, casts assignment
results to the table types, and rejects NULL in required columns.
Invalid columns and expressions are checked even on empty tables.

A file without a matching row keeps its exact path. Each affected file's
replacement rows go through DataFileWriterBuilder, ParquetWriterBuilder,
RollingFileWriterBuilder, and the default location and file name generators.
An empty replacement only removes its old file. There are no position or
equality delete files. A mutation with no matched rows succeeds without
creating a snapshot.

One Transaction::overwrite action contains all added and removed DataFiles.
Transaction::commit uses the normal atomic catalog path and its UUID and
main-snapshot requirements. The vendored action validates additions and
rejects removals that are no longer live or whose metadata changed. It writes
an Overwrite snapshot, then storage refreshes the cached provider. Effective
DELETE and UPDATE statements each create exactly one snapshot. Old data
and manifest files are retained for historical reads. No drop/register or
direct catalog edits occur on this path.

`replace_all(&TableIdent, Vec<RecordBatch>)` uses the same writer and commit
path. It removes all files in the loaded snapshot and writes the supplied
row set in one snapshot. An empty replacement, including on a new empty
table, is valid. Input column names, order, and types must match the table.

Shipped limits: local filesystem warehouses, unpartitioned Iceberg v2,
Parquet data files, no delete files, and no encrypted tables. Acceptance uses
SQLite and BIGINT columns with nullable values. The catalog's existing
Postgres round trip remains covered separately; mutation tests do not prove
Postgres, cloud, partition evolution, or nested-type mutation behavior.
Files are processed in memory one at a time. There is no spill or mutation
memory budget yet. Failure before the catalog commit can leave unreferenced
new files; orphan cleanup is later work. Transactions rebase on current
catalog metadata. Concurrent appends can survive a swap; a competing change
to a removed file fails validation. This is not serializable predicate
locking or a multi-table transaction.

The writer acceptance tests cover selective DELETE and UPDATE, exact file
reuse, no-match no-ops, null predicates, simultaneous assignments, full-table
DELETE, bulk and empty replacement, historical reads, snapshot counts and
summaries, invalid SQL and values, mixed-manifest sequence preservation, and
stale-removal rejection. See `crates/opensnow-storage/tests/writer.rs`.

## Slice 1.5: compaction and maintenance

`StorageCatalog::maintain(&TableIdent, &MaintenancePolicy)` runs one pass per
table for a scheduler. There is no daemon loop in this slice. The policy
has these defaults:

- Target file size: 256 MiB, inside the design's 128 to 512 MB band.
- Minimum file size: 128 MiB. Only files below this threshold and below
  the target are candidates.
- Reserved retention minimum: five snapshots. The optional expiry cutoff
  is Unix milliseconds. Expiry is inactive, including when a cutoff is set.
- Orphan grace period: 24 hours. This is reserved for future safe cleanup
  and does not enable file deletion.

The target, minimum size, and retained snapshot count must be positive.
Candidates use deterministic first-fit decreasing bin packing by physical
file size, with path ordering to break ties. Each bin's input bytes fit the
target. Only bins with at least two files are rewritten. Files at or above
target and files outside the candidate threshold keep their exact paths.
No eligible bin means no compaction commit or new snapshot.

The pass loads the current snapshot's live DataFiles and plans its scan
tasks. It reuses the writer's manifest resolution, per-file Iceberg Arrow
reader, schema normalization, and Parquet writer stack. Each bin is read
in memory and written through DataFileWriterBuilder, ParquetWriterBuilder,
RollingFileWriterBuilder with the policy target, and the default location
and file name generators. The rolling writer checks buffered size between
batches. The target is a sizing goal, not a hard byte cap for arbitrary
rows, compression, batches, or Parquet footer sizes. There is no spill or
memory budget yet.

All bins share exactly one Transaction::overwrite action. It removes the
old DataFiles and adds the merged DataFiles through the atomic catalog
commit, then refreshes the provider. The overwrite patch validates removed
files at commit time. Concurrent appends can survive; competing changes to
removed files fail validation. Old files remain for historical reads and
clones. Failed writes can leave unreferenced output files.

`MaintenanceReport` counts rewritten input and output files, their physical
bytes, expired snapshots, removed orphan files, and whether compaction
committed a snapshot. Compaction counters describe the committed swap.
Errors are returned. Failed commits can leave new unreferenced files.
Expiry and orphan counters are always zero in this slice.

Snapshot expiry does not ship. The pinned ExpireSnapshots action only
removes metadata and leaves shared physical files intact. However, its
per-branch retention settings override the requested retain_last and age
cutoff, and TableMetadata has no public reference iterator or retention
accessor. The service cannot validate those overrides or guarantee the
policy's minimum and cutoff for all accepted tables through that action.
No extra vendored action or dependency was added to bypass this gap.
All snapshots and their files are retained. A test requests an aggressive
cutoff and zero orphan grace, then verifies every source snapshot remains
and both source history and the clone still return their exact rows.
Cross-table reference coordination remains required before any future
expiry path can be paired with physical cleanup.

Physical orphan cleanup does not ship. The catalog can list tables, but
there is no atomic cross-table reference registry or lock coordinating
clone registration, writers, and deletion. Enumerating same-namespace
clones and all their snapshots is not a stable protection set while a
clone or a write can commit. A grace period alone cannot prove that an
older unreferenced file will not be committed by a slow writer. Until that
coordination exists, the service preserves every file and reports zero
orphan removals. A test proves even zero grace leaves an unreferenced file
alone. Historical and clone reads are also checked after maintenance.

The overwrite action writes new manifests for swapped files. The pinned
crate has no standalone manifest-rewrite action. Standalone manifest
compaction does not ship in this slice.

Bergman was evaluated on paper only. It is absent from Cargo.lock and
would add an unpinned external dependency and a second maintenance engine.
This slice keeps the vendored Iceberg stack and adds no dependency.

Inherited limits are local filesystem warehouses, unpartitioned Iceberg
v2, Parquet data files, no delete files, and no encrypted tables. Tests use
SQLite and BIGINT values including NULL. They prove 50 ten-row appends
compact from 50 files to one with a 64 KiB test target, exact 500-row
results and history, one new snapshot, immediate no-op, exact large-file
reuse, disabled expiry, disabled physical cleanup, and three bins plus
one untouched singleton committed in one snapshot. These fixtures do not establish production throughput or a hard file size bound.
