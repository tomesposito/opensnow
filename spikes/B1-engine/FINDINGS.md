# B1 Spike: Our Own Engine Core on Apache DataFusion

Date: Nov 5-6, 2026. Question: is DataFusion a credible base for an engine we build and own (Archetype B), good enough for Gate 1 (single-node analytical queries at about a tenth of Snowflake cost)?

DuckDB appears in this spike only as a timing yardstick, clearly labeled reference. It is not a dependency, and nothing in the code imports or shells out to it from the Rust program.

## Setup

- VM: 4 cores, 7 GB RAM, shared with two sibling spike builds for much of the run. Every number below carries that contention; read timings as ballpark, not gospel.
- No Rust toolchain was on the VM. Installed rustup into this spike dir (CARGO_HOME and RUSTUP_HOME inside it): rustc 1.99.1, cargo 1.99.1.
- DataFusion 55.2.0 (latest stable on crates.io), default features, tokio runtime.
- Program: `src/main.rs`, three modes. `gen` reads CSV and writes Parquet through DataFusion COPY. `bench` runs the six queries in `queries.sql`, three runs each, median reported. `probe` tests UDF, EXPLAIN, a second tenant context, and memory limits.
- Data: `gen_data.py` (seed 42) wrote sales.csv, 1,000,000 rows (order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount; 48 MB), and products.csv, 500 rows (product_id, category, product_name, brand). DataFusion converted them to Parquet: sales.parquet 9.3 MB, one file, products.parquet 3.8 KB.
- Reference yardstick: DuckDB CLI v2.0.0, run against the same Parquet files, per-statement timer, three runs each.

## Build cost

This is the first real finding. A clean release build of DataFusion with default features is expensive:

- The dependency build ran about 45 minutes, then a VM reboot killed the session. The restarted build ran about 68 more minutes to the point where only our own crate failed (two API errors, fixed in minutes). Our crate alone then rebuilt in 2 minutes 25 seconds.
- Total wall time across the reboot and with two sibling Rust builds sharing 4 cores: roughly 1 hour 50 minutes for dependencies. A solo build on a bigger machine will be much faster, but the shape is real: DataFusion is a large dependency tree (about 580 crates in our target dir, 745 MB before the final build finished).
- The release binary is 175 MB with default features. Production work trims features and strips symbols; the spike did not.
- Practical fallout: CI needs a warm cargo cache or every build pays this. Budget for it.

## Timings

Median of three runs, milliseconds, same Parquet file, same machine. DataFusion numbers are in-process (no process start). Reference numbers are the DuckDB CLI per-statement timer for the query statement only.

| Query | DataFusion 55.2.0 | DuckDB v2.0.0 (reference only) |
|---|---|---|
| Q1 scan, filter, aggregate | 50 | 80 |
| Q2 group by region and month | 94 | 157 |
| Q3 join, aggregate by category | 75 | 32 |
| Q4 sort, top 10 | 70 | 84 |
| Q5 window rank, top 3 per customer | 863 | 391 |
| Q6 CTE, subquery, months above average | 141 | 96 |

Correctness check: both engines returned the same full-table net revenue, 2,429,806,225.786, and the custom UDF below returned the same number inside DataFusion.

Reading: same ballpark on five of six. DataFusion won the plain scan and the group by, lost the join aggregate and the CTE. The clear gap is Q5, the window function at 863 ms against 391 ms reference. Window queries over 50,000 customers are a normal warehouse shape, so that gap is ours to close or route around. At 1M rows and 9.3 MB of Parquet, none of this predicts behavior at 100 GB; it says the base engine is in the right performance class for single-node work, not that it is tuned.

## Capability probes

- **Custom UDF: works, small code.** A `net_price(quantity, unit_price, discount)` scalar UDF registered with `create_udf` in about 40 lines and ran a full-table aggregate in 40 ms. This is the hook the D-02 compatibility layer needs: Snowflake functions can be implemented as our UDFs on this path.
- **EXPLAIN: good.** Both logical and physical plans print. The physical plan shows the things a warehouse operator wants to see: column pruning in the table scan, the Parquet file split into four row-group ranges for parallel scan, a hash join with the small table collected, and two-phase (partial, then final) partitioned aggregation. Plan inspection will not be a gap.
- **Per-tenant shape: contexts are cheap and separate.** A second SessionContext with its own config (2 target partitions, batch size 4096) registered the same tables and ran Q2 in 101 ms, in the same process, without touching the first context. The isolation shape for one process per host with many tenant contexts, or one context per tenant process, is natural: each context carries its own SessionConfig (partitions, batch size, sort and join limits) and its own RuntimeEnv (memory pool, disk manager, object stores). Nothing is shared unless we share it.
- **Memory limits and spill: sort spills, for real.** With a GreedyMemoryPool capped at 64 MB, a full sort of all 1M rows completed in 591 ms; the external sorter spilled to disk instead of failing. A 50,000-group aggregate also completed under the same pool (89 ms; its state is small, so this does not prove large-aggregate spill). Memory pools are per RuntimeEnv, so a per-tenant cap is a config line, not a project.
- **What we did not probe:** aggregate spill at state sizes far past the pool, join spill, and query cancellation mid-flight. All three matter for a hostile multi-tenant host and belong on the Phase 1 list.

## What DataFusion does not give us

A warehouse needs these, and none of them ship in the box:

1. **A persistent catalog.** SessionContext holds an in-memory catalog. Tables registered in a process die with the process. The design already answers this with Postgres as control plane plus an Iceberg catalog (spike B2), and DataFusion has catalog and schema provider traits to plug one in. The traits are the easy part; the catalog service is ours to build and run.
2. **Auth and tenancy enforcement.** No users, no roles, no row or column security, no per-tenant accounting. Contexts give isolation of config and memory, not a security boundary. Our service layer must authenticate, authorize per tenant before a context is built, and meter per query.
3. **Distributed execution.** DataFusion is one node. Ballista is a separate project that adds a scheduler and shuffle on top; this spike did not build on it, per brief. Single-node is the Gate 1 bet (D-01 revised). Anything past one node is later work, on Ballista or our own, and the research prices a credible multi-node warehouse at 2 to 3 years.
4. **The rest of the warehouse surface.** No cost-based join ordering as deep as the mature engines, no result cache, no warehouse-style suspend and resume, no time travel (that comes from the table format, spike B2), no workload management or query queue. Each is a build item, sized in the design doc, not a surprise.

## Verdict

Yes, DataFusion is a credible base for our own engine toward Gate 1. It ran all six analytical query shapes correctly at reference-class speed on the first attempt, it takes our own functions and our own catalog through public traits, per-tenant contexts and memory caps are built in, and its license is Apache-2.0, which fits D-04. The two honest warnings are window-function speed (about 2.2x behind the reference on Q5) and the build weight (a very large dependency tree that makes cold builds and CI a real cost).

Top three gaps we must build, in order:

1. The catalog and control plane: Postgres-backed catalog provider, table registration from Iceberg, tenant and user model on top.
2. The tenancy and metering layer: auth, per-tenant limits enforced before and during execution, per-query cost accounting (Gate 1's cost case depends on this number being real).
3. Window and join performance work: profile Q5-class queries, tune partitioning and sort settings, and add our own operators only where the profile proves the need.

## Files

- `src/main.rs`: the engine program (gen, bench, probe modes)
- `queries.sql`: the six shared queries
- `gen_data.py`: dataset generator (seed 42)
- `bench_datafusion.txt`, `bench_duckdb_ref.txt`: raw timing output (reference file is DuckDB, yardstick only)
- `probe_output.txt`: raw probe output (UDF, EXPLAIN, tenant context, spill)
- `run_duckdb_ref.sh`: the reference timing script
- `data/`: CSV and Parquet datasets
- `.cargo/`, `.rustup/`, `target/`: toolchain and build output, local to this spike
