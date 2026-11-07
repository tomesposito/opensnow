# Engine service

Slice 1.2, issue #5. `opensnow_engine::EngineService` is the in-process,
single-node SQL endpoint. `register_tenant` creates a separate SessionContext,
RuntimeEnv, GreedyMemoryPool, admission state, and cumulative cost counter for
each tenant. The net_price UDF is registered on every context. Duplicate tenant
names and invalid resource groups are rejected without replacing existing state.
`context` provides trusted setup access for registering tables or views. Calling
SQL directly through that context bypasses the service's admission and accounting.

## Resource groups

| Field | Enforcement |
| --- | --- |
| memory_bytes | Hard size of the tenant's DataFusion memory pool, shared by its concurrent queries. |
| target_partitions | DataFusion execution parallelism setting, not an OS thread cap. |
| max_concurrent | Maximum queries executing for this tenant. |
| queue_capacity | Additional waiting requests. Running plus queued requests cannot exceed max_concurrent plus queue_capacity. |
| queue_timeout | Maximum wait for a running slot. An immediately available slot works even with zero timeout. |
| cost_cap_units | Optional cumulative tenant cap, retained for this service lifetime. |

DataFusion supplies per-context configuration, memory reservations, and operator
spill. Our layer supplies bounded admission, concurrency, timeout, cumulative
accounting, and cost rejection. Each tenant has its own semaphores. Filling one
tenant's queue does not consume another tenant's slots. Dropping an admission
guard or cancelling a waiting future releases its permits. A queued request
checks the cap again after obtaining a running slot.

`execute(tenant, sql)` returns batches and a QueryOutcome containing rows returned,
cost units charged, and elapsed time including admission and planning. Errors
distinguish UnknownTenant, AdmissionDenied, CostCapExceeded, and Execution.
DataFusion errors, including Resources exhausted, remain wrapped intact.

## Cost enforcement

One cost unit is one row in a returned stream batch. These deterministic units
are an enforcement testing placeholder, never dollars. Actual query cost and
metering belong to slice 1.7, issue #10. An aggregate scanning millions of rows
but returning one row therefore costs one placeholder unit.

The service rejects a tenant already at or above its cap before taking any
admission permit or planning SQL. It executes using a stream and atomically
charges each batch as it arrives. This is incremental cumulative accounting:
concurrent queries cannot each spend the same unused balance. A successful
query's outcome reports the sum of its batch charges, with no second charge at
completion. If a batch pushes the cumulative charge above the cap, the stream
is dropped and CostCapExceeded returns no partial results. The entire aborting
batch is charged, along with earlier batches, so the cumulative counter may
overshoot by a batch. Previously processed batches also stay charged if a later
execution error or caller cancellation occurs. No further batches are consumed
after an abort. There is no cost refund or counter reset API in this slice.

## Correctness and isolation checks

`standing_yardstick_diff` runs the six existing B1 query shape row-count checks
and compares Q1 count and revenue, every Q2 region/month/count/revenue, every Q3
category/count/revenue, and the net_price aggregate against independent plain
Rust calculations on 24 generated sales rows and two products. Floating numeric
values use absolute tolerance 1e-9. Keys, dates, counts, and result cardinalities
are checked too. `opensnow_engine::check()` runs the same diff, so stack-check
prints `PASS yardstick value diffs: Q1, Q2, Q3, net_price`.

This is the standing form of the DuckDB yardstick. B1 verified the same arithmetic
against DuckDB on the spike data, with net revenue 2,429,806,225.786 in both
engines. DuckDB is reference only: never imported, invoked, or added as a
dependency. All test fixtures are generated in memory without reading spikes/.
CI's existing `cargo test --workspace --locked` runs the harness and the tenant
isolation tests. No workflow changes are required.

The memory isolation test executes the same non-equi nested-loop join for two
tenants using 4,000 generated integer keys. The high-memory tenant completes;
the 1 KiB tenant must return a DataFusion Resources exhausted error. This tests
an actual build-side reservation failure, rather than inspecting configuration.
Sort is not suitable for that proof because it can spill and succeed. Concurrency
tests hold admission guards and poll queued futures to prove bounded admission,
cancellation cleanup, and cross-tenant progress without sleeps. A paused Tokio
clock tests queue timeout. Cost tests cover exact-cap admission rejection,
stream abort, charging of the aborting batch, and another tenant's success.

## Limits and dependencies

Memory pools cover reservations made by DataFusion operators, not every byte in
the process. Registered in-memory input tables, output batches retained by the
caller/service, planner allocations, and untracked operator allocations are not
a process RSS limit. There is no global host memory arbiter, disk spill quota,
or fairness scheduler across groups yet. Spill uses each context's disk manager.
The service buffers successful results in memory. Caps are checked at batch
boundaries, so they do not interrupt a long-running operator before its next
output. Dropping the stream requests cancellation through DataFusion's stream
lifecycle; it is not a preemptive OS kill or a rollback of SQL side effects.

There is no HTTP transport, listening socket, authentication, authorization,
persistent accounting, or multi-node execution. Transport and auth belong to
later slices #9 and #12. A tenant context is a resource boundary, not a security
sandbox. The API uses DataFusion types for engine integration at this boundary;
no DataFusion internal components or fork are needed.

Only existing workspace pins are added as engine dependencies: futures 0.3.34
(MIT OR Apache-2.0) and tokio 1.54.1 (MIT), with sync, time, and test-util support.
Their licenses are recorded in PINNED-STACK.md and confirmed in cached manifests.
No new package or version is introduced into Cargo.lock.

Reproduce with the toolchain environment in PINNED-STACK.md:

```sh
cargo test --workspace --offline
cargo run -p stack-check --offline
cargo tree --offline -i datafusion
cargo tree --offline -i arrow
```

Local verification passed: `cargo test --workspace --offline` ran eight engine
tests and one combined_stack test, with nine passed and zero failed. Both
library doc-test targets ran zero tests and passed. `cargo run -p stack-check
--offline` exited zero and printed the yardstick diff result along with all six
B1 shapes and the B2 storage checks. Both inverse dependency-tree commands
confirmed the single pinned DataFusion and Arrow versions. Cargo.lock still
contains the same 477 packages, with no git sources or DuckDB.
