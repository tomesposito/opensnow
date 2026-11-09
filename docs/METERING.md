# Metering and per-query cost

Slice 1.7, issue #10. Every executed query leaves a cost record, and a
tenant can read its records back in dollars. The metering lives inside
execution in the opensnow-engine crate. The opensnow-control crate
persists the records to Postgres and serves the authenticated read
back. There is no HTTP transport and no UI in this slice.

## What a record holds

One record per executed query, per tenant, with a per-tenant sequence
number starting at 1:

- tenant, sequence number, and the SQL text as submitted. A tenant
  reads back its own query history.
- kind: query, or background for non-query work (below).
- status: completed, failed (a planning or execution error), or
  cost_cap_exceeded (the existing rows cap aborted the stream).
- start time as Unix milliseconds, and elapsed wall time.
- rows returned: the rows in the batches actually pulled from the
  result stream. This matches the existing accounting in the engine: a
  batch that trips the cost cap is pulled and charged, so it counts. A
  batch that is never pulled does not.
- bytes returned: the in-memory size of those batches, from
  RecordBatch::get_array_memory_size. This is bytes produced. It is
  not bytes scanned from storage. The result stream does not hand us a
  truthful scanned figure, so we do not claim one.
- allocated vCPU-seconds and cost in dollars, below.

## The dollar math

Dollars are cost attribution, not a price, not a bill, and not a
charge to the customer. The rate is our internal cost basis from
docs/COST-MODEL.md Section 1: the EC2 m7i family prices linearly at
$0.0504 per vCPU-hour, us-east-1, on-demand, list.

- allocated vCPU-seconds = elapsed seconds x the tenant's
  target_partitions. Partitions are the parallel execution lanes the
  resource group allocates to the tenant, so this is a stated model of
  allocated compute. It is not a CPU-time measurement.
- cost_dollars = vCPU-seconds / 3600 x the rate.

The rate sits in a public MeteringRates struct in the engine, with the
$0.0504 default as a named constant. The Gate 1 run (issue #13) can
price records at a different rate without a code change, and the
records carry everything it needs: elapsed, rows, bytes, and
vCPU-seconds per query, per tenant. That is the feed the Gate 1 cost
model comparison in docs/COST-MODEL.md asked for.

## The recording rule

Every EngineService::execute call for a registered tenant that passes
admission and reaches planning leaves exactly one record. That
includes queries that fail planning, queries that fail during
execution, and queries the cost cap aborts mid-stream.

Calls rejected before that point leave no record, because no query
executed: an unknown tenant, admission denied by a full queue, or the
cost cap already exhausted at the pre-execution check.

A successful query's response carries its record's sequence number,
bytes returned, vCPU-seconds, and dollars, so the cost of a query is
visible in the response itself, as Sections 6 and 7 of
SOLUTION_DESIGN.md ask.

The engine's existing rows-based accounting and cost_cap_units
enforcement do not change. The cap still counts rows. The meter is a
record of what happened, not a second cap.

## Read back

In the engine:

- EngineService::cost_records(tenant) returns the tenant's records in
  sequence order.
- EngineService::cost_summary(tenant) returns the count and the totals
  for dollars, vCPU-seconds, elapsed, rows, and bytes. The totals equal
  the sum over that tenant's records.
- Records are per tenant. No tenant can see another tenant's records.

Through the control plane, authenticated by token like list_tables:

- ControlPlane::execute_authenticated syncs the tenant's new records
  into the Postgres table query_cost_records after every call, on the
  success path and on the engine error path alike. Sync is idempotent:
  the table has a unique constraint on (tenant_id, seq) and already
  stored sequences are skipped.
- ControlPlane::cost_records(token) syncs, then reads the caller's own
  tenant records back from Postgres. Because they are read from
  Postgres, they survive an engine restart.
- ControlPlane::cost_summary(token) returns the caller's totals from
  the stored records.
- Any valid token scope may read the tenant's own costs. A bad,
  expired, or revoked token fails with the same opaque AuthFailed as
  every other authentication failure in the control plane.

## Background work

Section 7 of SOLUTION_DESIGN.md wants background services as their own
line items. The engine has EngineService::record_background(tenant,
service, elapsed, rows, bytes), which writes a background record with
the same dollar math, and such records appear in read back and in
summaries like any other.

Nothing calls it yet. Compaction and maintenance live in the
opensnow-storage crate, which does not depend on the engine, and wiring
that dependency in is not part of this slice. Automatic background
metering is an absence, stated here rather than faked.

## Absences, stated plainly

- No bytes-scanned figure. Bytes returned is bytes produced, labeled
  as such.
- No storage or S3 request charges inside a per-query record. Those
  are shared lines in the Gate 1 model (docs/COST-MODEL.md Sections 2
  and 3), not per-query costs.
- No dollar-denominated budgets or caps, and no alerts. The enforced
  cap is still cost_cap_units, counted in rows. Dollar budgets are
  later work.
- No customer pricing. The dollars here are internal cost at AWS list
  rates. What a customer pays is a pricing decision, not a metering
  output.
