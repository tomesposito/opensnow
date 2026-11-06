# B2 Iceberg spike: findings

Question: can our own engine stack, DataFusion plus the Rust Iceberg crates, own the storage layer for opensnow?

Method: two hands-on runs on local disk. One Rust program using DataFusion SQL over the iceberg-datafusion provider. One Python script using pyiceberg on the same kind of table. Both used a SQL catalog on SQLite and a warehouse directory on local disk.

## Stack and versions

- iceberg 0.11.0 (Apache iceberg-rust). Current release line: 0.8.0, 0.9.0, 0.10.0, 0.11.0 across 2026. Active and shipping.
- iceberg-datafusion 0.11.0. It pins DataFusion 54.1.0 and Arrow 58.
- iceberg-catalog-sql 0.11.0 and iceberg-catalog-rest 0.11.0.
- DataFusion latest on crates.io is 55.2.0. The Iceberg integration lags one major version behind.
- pyiceberg 0.12.0 with pyarrow 26.0.1.
- Toolchain: rustc 1.99.1, installed into this spike directory.

Governance moved in September 2026. Apache Iceberg and Apache DataFusion voted to move the DataFusion integration out of iceberg-rust into a new repo, apache/datafusion-iceberg, run by the DataFusion project. The new crate is named datafusion-iceberg and targets DataFusion 55. The 0.11.0 crate we tested is the last line under the old home. Plan for a crate migration when we move to DataFusion 55.

## What worked end to end (Rust, through DataFusion SQL)

Run log is in rs_results.txt. Program is rs-spike/src/main.rs.

- SQL catalog on SQLite loaded. Namespace `db` created.
- DataFusion catalog provider built and registered as catalog `ice`.
- CREATE TABLE ice.db.events through DataFusion SQL: worked. Format version 2.
- INSERT of 100,000 rows through DataFusion SQL: worked. Count read back through DataFusion: 100,000.
- Second INSERT of 50,000 rows: worked. Count: 150,000.
- The two inserts produced exactly two Parquet data files in the warehouse, one per commit.
- Time travel: the iceberg API scan pinned to the first snapshot returned 100,000 rows. Snapshot id 5627625100965907760. The current table held 150,000 rows at the same moment. Snapshot isolation works.
- REST catalog client: the client built and issued GET /v1/config with the warehouse parameter. It failed only because no server runs on this machine. The client exists and speaks the REST catalog protocol shape.

## What worked (pyiceberg)

Full log in py_results.txt.

- Created a format version 2 table partitioned by city, on a SQL catalog over SQLite.
- Wrote 100,000 rows, appended 50,000 rows. Counts verified after each step.
- Time travel: scan at the first snapshot returned the original 100,000 rows, all NYC.
- Row delete of one id worked. Count went to 149,999 and the row was gone.
- Upsert of one id worked. The row read back with the new city and amount.
- The delete and the upsert both landed as overwrite snapshots. The scan plan held zero delete files. pyiceberg 0.12.0 does row changes as copy-on-write rewrites of the affected data files. It writes no position deletes and no equality deletes here.

## What failed, and where

- DELETE through DataFusion SQL failed. Exact error: "This feature is not implemented: DELETE not supported for Base table".
- UPDATE through DataFusion SQL failed. Exact error: "This feature is not implemented: UPDATE not supported for Base table".
- The provider source confirms the limit. iceberg-datafusion 0.11.0 accepts only InsertOp::Append. InsertOp::Overwrite and InsertOp::Replace are rejected with "IcebergTableProvider supports only append inserts". The table provider has no delete path at all.
- Version skew dead end: a first build used DataFusion 55 and Arrow 59. Cargo then held two DataFusion versions in one tree, 54.1.0 for the Iceberg provider and 55.2.0 for our code, and the provider cannot register inside a DataFusion 55 session. The fix was to pin DataFusion 54.1 and Arrow 58, the versions the integration was built against.
- SqlCatalog needs a storage factory wired by hand. Without it, load fails with "StorageFactory must be provided for SqlCatalog. Use `with_storage_factory` to configure it." The fix is one line: LocalFsStorageFactory for local disk.
- sqlx runtime panic: iceberg-catalog-sql depends on sqlx 0.8 with only the "any" feature and default features off. No runtime feature is enabled. The catalog then panics at connect time: "either the `runtime-async-std` or `runtime-tokio` feature must be enabled". The consumer crate must add sqlx with runtime-tokio itself. We did. This is a packaging wart in the catalog crate.
- SqlCatalog does not create the SQLite database file. The connection fails with sqlx error code 14, unable to open database file, until the caller creates the file first. The crate's own tests call create_database before loading for the same reason.
- REST catalog live test is still open. This machine has no Java and no Docker, so no local Polaris server could run. A third-party test of the 0.10.1 REST client found 13 of 25 operations working. Missing pieces included all view operations, scan planning, metrics reporting, the credentials endpoint, and commitTransaction. Multi-table commits through REST are therefore out today. SigV4 signing for Glue and S3 Tables catalogs through the REST client is also missing (iceberg-rust issue 1236, open since April 2025). Those two AWS catalogs have their own dedicated Rust crates.

## Small files and compaction

- pyiceberg test: 50 appends of 10 rows each produced 50 data files and 50 snapshots for 500 rows. Every commit adds files. Nothing merges them.
- Rust test: one file per insert commit at this scale. The same shape appears as soon as writes get small and frequent.
- pyiceberg has no compaction call on the Table object. We checked for compact and rewrite entry points. None exist.
- iceberg 0.11.0 transaction actions are FastAppend, ExpireSnapshots, ReplaceSortOrder, UpdateLocation, UpdateProperties, UpdateSchema, UpdateStatistics, and UpgradeFormatVersion. There is no rewrite data files action. Compaction is not in the core Rust crate.
- Snapshot expiration does exist in the crate, so retention cleanup has a base to build on.
- One external option exists: Bergman, a Rust maintenance engine built on iceberg-rust and DataFusion. It does compaction, snapshot expiration, orphan file cleanup, manifest rewriting, and dangling delete removal, driven by TOML policies. We read its description and did not run it.
- Row-level write support upstream is thin in general. Per SQE's docs, upstream iceberg-rust lacks the position-delete and deletion-vector writers, and they vendored a fork to get them. The copy-on-write path is the one that works today.

## Catalog answer

- The SQL catalog works locally in both stacks. Rust used iceberg-catalog-sql on SQLite. Python used pyiceberg's SQL catalog on SQLite. Same warehouse layout on disk.
- This maps to the design in two steps. For the control plane we build first, a SQL catalog on Postgres is the same code path with a different connection string, and it works today.
- The Polaris plan still holds at the protocol level. Polaris serves the Iceberg REST catalog spec, and the Rust REST client targets that spec. Single-table reads and writes through REST look reachable. Two gaps matter: commitTransaction is missing, so multi-table atomic commits through REST are out, and the live handshake against a real Polaris server is untested because this machine cannot run one.

## Recommendation

Yes on the core call. Iceberg can be the storage format, and our DataFusion stack can own it today for the work that matters most: create tables, append data, read with DataFusion SQL, and time travel by snapshot. All four ran end to end in this spike.

Phase 1 needs one bridge and one companion service.

Bridge for row changes: DELETE, UPDATE, and MERGE cannot go through DataFusion SQL today. Give row changes their own writer path in our code, on top of the iceberg crate. The proven pattern is copy-on-write: read the affected files, write replacement files without the changed rows, and commit the swap as an overwrite transaction. pyiceberg already performs row changes this way against the same table format, so a Python writer service is a working fallback while the Rust writer matures. Append-only tables need no bridge at all.

Companion for maintenance: small files pile up from day one, and neither client ships compaction. Budget a maintenance service as part of Phase 1, not as later polish. Evaluate Bergman first. Fall back to our own jobs on the actions the crate does have, plus snapshot expiration, if Bergman does not fit.

Two standing rules come out of this spike:

- Pin the pair together. Today that pair is iceberg-datafusion 0.11.0 with DataFusion 54.1. Watch the new datafusion-iceberg crate in the DataFusion repo. It already targets DataFusion 55, and it is where the integration lives from here.
- Start the catalog on SQL (Postgres). Move table traffic to a REST catalog such as Polaris once the client gains commitTransaction, or accept single-table commits only.

## Files in this spike

- rs-spike/: the Rust program (src/main.rs), its Cargo.toml, and build logs.
- rs_results.txt: the Rust run log quoted above.
- py_spike.py and py_results.txt: the Python run and its log.
- warehouse/ and warehouse-rs/: the two test warehouses on local disk.
- catalog.db and rs-catalog.db: the two SQLite catalogs.
