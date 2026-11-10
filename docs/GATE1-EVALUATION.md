# Gate 1 Evaluation

Slice 1.10, issue #13. Run on 2026-11-09 by the build sweep, locally,
with the harness in crates/opensnow-gate1. Local only: no AWS account
was touched and no money was spent. "Real AWS costs" means AWS list
prices applied to measured local runtime, as docs/COST-MODEL.md
defines them.

## Verdict

**Gate 1 passes.** The measured total is **$298.65 per month** against
the Gate 1 line of $767 per month, 39 percent of the line. The
Snowflake total for the same workload set is $7,670 per month, so the
prototype serves the set at about **25.7x under** Snowflake cost,
single node, with the conservative choices in this report (W2 and W3
costed at the runtime parity bound, the W5 refresh carried at the
model's hours, EC2's 60 second minimum charged on every start).

Two qualifications, both weighed below. Four statements cannot run at
all: the three MERGEs and the UPDATE with a FROM clause. The write
side of the ELT and feature workloads is therefore incomplete as a
product, even though its read side is timed and its landing path is
proven. And the run measures at 1:1000 scale, not at 4 TB. The scaling
evidence and the storage sensitivity both say the verdict survives
those limits, but they are limits.

## How the run works

- The harness (crates/opensnow-gate1) generates the shared dataset
  from docs/workloads/README.md at a scale divisor. Main point:
  divisor 1000, so sales_fact is 2,000,000 rows, clickstream_events
  6,000,000, orders_raw 350,000, dim_customer 2,000, dim_product 500.
  Generation is deterministic (a splitmix hash of the row index), and
  plain Rust accumulators over the same rows are the yardstick.
- Data loads into Iceberg tables through opensnow-storage
  (replace_all, one overwrite snapshot per table) and is queried
  through EngineService (opensnow-engine) with a resource group of 2
  partitions and a 2 GiB memory pool. Queries run against Parquet
  files on disk, not memory tables.
- Dates are anchored to 2026-11-09, the run date: CURRENT_DATE() in
  the workload files becomes that date, CURRENT_TIMESTAMP() noon that
  day.
- Every statement runs once warm and three times timed; the median is
  the measurement. The binary is a release build. Timings ran on this
  VM (shared vCPU, not an m7i); the conversion below states how local
  seconds become instance hours.
- A second point at divisor 10,000 checks scaling: at 10x the rows,
  the scan and aggregate shapes grew 5.9x to 7.2x in runtime, the
  window and CTE shapes 9.3x to 9.8x. Nothing grew faster than the
  rows. Linear extrapolation to full scale is therefore the
  conservative choice, and it is what the conversion uses.
- Prices were re-checked on 2026-11-09 before the scorecard was
  filled. No line in docs/COST-MODEL.md Section 1 changed: m7i.large
  at $0.1008 per hour and m7i.xlarge at $0.2016 per hour in us-east-1
  confirmed against October and November 2026 price sources, and the
  family prices linearly, so $0.0504 per vCPU-hour stands at every
  size. S3 Standard at $0.023 per GB-month, PUT at $0.005 per 1,000,
  GET at $0.0004 per 1,000, egress at $0.09 per GB, and gp3 at $0.08
  per GB-month all confirmed against the AWS price list as published
  in October 2026.

## Correctness

Wrong results fail a workload regardless of speed, so the checks come
first. Every check passed:

- Global: count and net revenue over all 2,000,000 sales_fact rows
  match the Rust accumulators exactly (float tolerance 1e-6 relative).
- Full result sets compared cell by cell: revenue by region and month
  (W2.1, W2.2), revenue by category and segment (W3.1), funnel counts
  with distinct sessions (W2.5).
- Scalar and shape checks: month-to-date totals and distinct customers
  (W2.4), the stand-in customer's month-to-date totals (W4.1), East
  month-to-date revenue (W6.5), deduped order count for the last day
  (W1.1: 99,500 of 100,000 landed rows in the window), daily aggregate
  row count (W1.3: 36 day and region pairs), feature row counts equal
  to the distinct customer count (W5.1, W5.2, W5.4s: 2,000).
- The three data quality checks (W1.4a to W1.4c) and the orphan join
  check (W6.7) returned 0, as the data requires.
- Write path: the W1.3 aggregate (36 rows) and the W5.4s feature set
  (2,000 rows) landed into their Iceberg tables by replace_all, and a
  copy-on-write DELETE on a scratch table removed exactly the East
  rows (50,000 of 200,000).
- Concurrency: 30 concurrent W4.1 home-screen queries (the W4 peak)
  all completed, 0.73 seconds wall at this scale, through one tenant
  resource group.

## Measured runtime

Medians at divisor 1000, seconds, release build, 2 partitions. Full
per-run values are in the harness JSON the sweep saved with this run.

| Stmt | Seconds | Rows | Stmt | Seconds | Rows |
|---|---|---|---|---|---|
| W1.1 dedupe | 0.303 | 99,500 | W4.1 home screen | 0.033 | 1 |
| W1.3 daily rebuild | 0.072 | 36 | W4.2 top products | 0.043 | 10 |
| W1.4a DQ nulls | 0.001 | 1 | W4.3 trend | 0.040 | 13 |
| W1.4b DQ negatives | 0.240 | 1 | W4.4 activity | 0.211 | 5 |
| W1.4c DQ orphans | 0.195 | 1 | W5.1 feature base | 0.081 | 2,000 |
| W2.1 region month | 0.148 | 52 | W5.2 ranked | 0.086 | 2,000 |
| W2.2 one region | 0.131 | 13 | W5.3 trend | 1.308 | 2,000 |
| W2.3 top products | 0.044 | 10 | W5.4s merge source | 1.354 | 2,000 |
| W2.4 headline MTD | 0.029 | 1 | W6.1 schema tables | 0.600 | 8 |
| W2.5 funnel | 0.373 | 5 | W6.2 schema columns | 0.000 | 8 |
| W3.1 category segment | 0.143 | 20 | W6.3 sample | 0.432 | 100 |
| W3.2 top customers | 0.101 | 12 | W6.4 probe | 0.025 | 4 |
| W3.3 discount hunt | 0.049 | 4 | W6.5 quoted number | 0.058 | 1 |
| W3.4 month growth | 0.185 | 25 | W6.6 follow-up | 0.061 | 20 |
|  |  |  | W6.7 join sanity | 0.028 | 1 |

The slow shapes are the clickstream CTEs (W5.3, W5.4s, about 1.3
seconds over 6,000,000 events). The window shapes B1 worried about
are not slow here: the rank in W3.2 takes 0.101 seconds and the
percentile and decile windows in W5.2 take 0.086 seconds at this
scale.

## From seconds to monthly cost

The conversion, stated so it can be reworked:

- Full-scale seconds = median x 1000 (linear in rows; the second
  scale point says this is conservative for scans and about exact
  for windows).
- Instance seconds = full-scale seconds x (2 / priced vCPUs). The
  harness ran 2 partitions; the priced instances have 4 to 16 vCPUs.
  This assumes near-linear partition scaling, stated as an assumption.
- Batch workloads pay active time: W1 runs 30 nights, W5 big builds
  run 4 times. Every node start also pays EC2's 60 second minimum.
  The measured engine cold start (catalog connect, provider
  registration, first query) is 0.005 seconds, so the 60 second
  billing minimum is the whole start cost.
- W2 and W3 have no execution counts in the run mix, only Snowflake
  active hours. The scorecard costs them at the parity bound: the
  mix's active hours at our instance price, plus start minimums.
  That is the model's own number with starts added, and it is
  conservative: our measured per-pass times, scaled the same way,
  are 182 seconds for a W2 pass against the mix's implied 15 minutes
  of Snowflake active time per session, and 120 seconds for a W3
  pass against an implied 9 minutes per session. At the session
  floor (308 W2 passes, 110 W3 passes) W2 would cost $8.34 and W3
  $5.91 instead of $33.12 and $44.35.
- W4 and W6 pay window hours, as the model says: 240 and 176 hours.
  Runtime only has to prove the node keeps up, and at peak
  concurrency it did.
- W5's daily refresh could not be measured (W5.5 cannot run). Its
  15 hours a month are carried at the model's figure and flagged in
  the scorecard. It is the only carried number in the compute lines.
- Storage: measured Parquet bytes at scale, times 1000, at $0.023
  per GB. S3 requests: computed from the measured file layout, not
  the model's $10 allowance (see Soft spots). Control plane: $82 as
  modeled. Egress: none, local run.

## Scorecard

The same numbers are filled into docs/COST-MODEL.md Section 6.

| Workload | Snowflake | Target | opensnow measured | Ratio | Pass / fail |
|---|---|---|---|---|---|
| W1 Nightly ELT | $1,452 | $145.20 | $1.08 | 1340x | pass |
| W2 BI dashboards | $660 | $66.00 | $33.12 | 19.9x | pass |
| W3 Ad-hoc analysis | $1,320 | $132.00 | $44.35 | 29.8x | pass |
| W4 Embedded analytics | $2,940 | $294.00 | $96.77 | 30.4x | pass |
| W5 ML feature builds | $666 | $66.60 | $3.50 | 190x | pass |
| W6 Agent queries | $540 | $54.00 | $35.48 | 15.2x | pass |
| Compute subtotal | $7,578 | $757.80 | $214.30 | | |
| Storage (at cost) | $92 | $9.20 | $1.87 | | |
| S3 requests | n/a | n/a | $0.48 | | |
| Control plane | n/a | n/a | $82.00 | | |
| **Total** | **$7,670** | **$767.00** | **$298.65** | **25.7x** | **pass** |

Per-workload notes:

- **W1, $1.08.** Read side per night at full scale: 101 seconds on
  the priced m7i.4xlarge, against Snowflake's 2.0 active hours. Plus
  30 x 60 seconds of start minimums ($0.40 of the $1.08). The MERGE
  write time is not in this number because MERGE cannot run; even if
  the write side cost as much as the whole read side again, W1 would
  land near $2 against a $145.20 target.
- **W2, $33.12.** Parity bound: 77 active hours plus 308 start
  minimums (5.13 hours) at $0.4032. Floor at measured speed: $8.34.
- **W3, $44.35.** Parity bound: 99 active hours plus 660 start
  minimums (11 hours). Floor at measured speed: $5.91.
- **W4, $96.77.** Window hours, unchanged from the model. The burst
  check is the new evidence: 30 concurrent queries complete cleanly
  on one node.
- **W5, $3.50.** Big builds measured: 354 seconds per build at full
  scale on the m7i.4xlarge, $0.32 for four builds, plus $0.15 of
  start minimums. The daily refresh is carried at the model's 15
  hours ($3.02) because W5.5 cannot run.
- **W6, $35.48.** Window hours, unchanged from the model. Our
  thinnest ratio, as the model predicted: the win on W6 is the hour
  price ($0.2016 against $3 of Snowflake credits), not the hours.

## Margin check

Measured cost is $298.65 a month, $3,584 a year. At 70 percent gross
margin the price that carries it is $3,584 / 0.30 = about
**$11,900 a year**, inside the business case's $10k to $30k band.
The model needed about $18k. At $30k the measured margin is 88
percent. At the $10k floor the margin is 64 percent: the floor still
does not carry this exact workload set at 70 percent, but the gap
narrowed from the model's 46 percent. With storage charged at
Snowflake's 4.0 TB figure instead of the measured size (see Soft
spots), the total is $388.78, the carrying price about $15,600, also
inside the band.

## Statement ledger

33 statements. 1 ran as written, 28 ran with a rewrite, 4 could not
run. The rewrites are mechanical, and the compat layer owns every one
of them:

- DATEADD to date or timestamp minus INTERVAL (docs/COMPATIBILITY.md
  SF04): 13 statements.
- DATE_TRUNC results cast to DATE: 9 statements. A new finding, in
  two parts. DataFusion's date_trunc on a date returns a nanosecond
  timestamp, and the Iceberg provider cannot push a timestamp bound
  into a date column ("Can't convert datum from timestamp_ns type to
  date type"). Grouped months need the same cast to come back as
  dates. One cast rule in the compat layer covers both.
- CREATE OR REPLACE TEMP TABLE wrappers dropped, SELECT bodies run:
  5 statements (W1.1, W1.3, W5.1, W5.2, W5.3). The prototype has no
  temp tables; the bodies run unchanged underneath.
- INFORMATION_SCHEMA: 2 statements (W6.1, W6.2). The engine session
  does not enable information_schema, so both fail as written. The
  equivalents ran against the Iceberg catalog API and are timed in
  the table above (W6.1's 0.600 seconds is mostly counting 6M
  clickstream rows from a snapshot scan; the catalog itself is
  fast). An agent-facing schema surface needs this fixed or routed
  to the catalog; the MCP layer already reads the catalog directly.
- One each: temp table replaced by an inline subquery (W1.4b, W1.4c
  count as this pattern), SAMPLE (100 ROWS) rewritten as a
  deterministic first 100 (W6.3), TRY_CAST type name NUMBER to
  DECIMAL (W6.4), the MERGE source run as a plain SELECT (W5.4s),
  and a window ORDER BY that repeats the aggregate expression
  because DataFusion does not resolve the alias there (W3.2).

Could not run, with the reason as the finding:

- **W1.2, W1.3b, W5.4 MERGE.** The prototype has no MERGE. The
  storage writer supports single-table DELETE and UPDATE only. The
  merge read sides are timed (W1.1, W1.3, W5.4s) and the landing path
  is proven by replace_all, but a nightly ELT product needs a real
  merge or an explicit overwrite pattern per table.
- **W5.5 UPDATE with FROM.** The writer's UPDATE takes scalar
  assignments over the target table's own columns. A join-driven
  refresh cannot run, so the refresh's hours are carried, not
  measured.

One more build finding from the harness itself: timestamp columns
must be declared TIMESTAMP(6). A bare TIMESTAMP becomes a nanosecond
timestamp, the provider creates a format v3 table for it, and the
copy-on-write writer refuses v3 ("copy-on-write currently supports
unpartitioned Iceberg v2 tables"). Microsecond timestamps keep
tables at v2 and everything works. The writer's error message names
the wrong suspect (it blames partitioning), which cost this run a
diagnosis cycle. Worth a fix in the error text or the writer.

## Instance sizes

The scorecard prices the model's picks (Section 2, rule 3). The run
does not replace them with measurements, and says so: at 1:1000
scale every workload, including the 30-query burst, completed inside
a 2 GiB memory pool on 2 partitions. That contradicts nothing in the
picks, but a 2B-row sort at full scale is a different memory story
(spill exists; spike B1 proved a 1M-row sort under a 64 MB pool).
Costing the picks while runtime is measured is the conservative
split: if a workload needed the next size up, its line would roughly
double, and the total would still clear the line (the sensitivity in
COST-MODEL Section 5, point 2, works out the W4 case: total about
$548 at modeled hours; at measured hours the same correction adds
about $97).

## Soft spots, revisited

From docs/COST-MODEL.md Section 5, in its order:

1. **Runtime parity.** No longer assumed for the batch workloads:
   W1's read side runs 71x under Snowflake's active hours at full
   scale, W5's big builds 31x under (354 seconds against 3.0 hours).
   W2 and W3 are costed at parity anyway, with measured per-pass
   times 4.5x to 5x under the mix's implied Snowflake pace as
   supporting evidence. The window-function worry did not show up:
   window shapes were mid-pack, and the 10x scaling point grew them
   no faster than the scans.
2. **Instance sizes are picks.** Still true. See above.
3. **Start and stop.** Now measured. Engine cold start is 0.005
   seconds; the cost is EC2's 60 second minimum per start, $7.06 a
   month across W1, W2, W3, and W5, now counted in the scorecard.
   The product answer (shared nodes through the business day) would
   move W2 and W3's start costs toward zero; the gate does not need
   it.
4. **S3 requests.** Computed from the measured layout instead of the
   $10 allowance: about 1,039 data files at full scale (75 MiB
   writer target), about 1.15M GETs and 5.2k PUTs a month on the
   stated execution floors, $0.48 total. The small-file warning
   stands for data that arrives uncompacted; this dataset lands in
   large files because replace_all and the writer roll at 75 MiB.
5. **Parquet size.** Measured: 81.4 GB at full scale, against the
   4.0 TB Snowflake-format estimate. Treat the gap with suspicion in
   both directions. The generated fact columns are narrow and
   dictionary-friendly (4 regions, 500 products, 11 discount values),
   so Parquet compresses them hard; real customer data will store
   larger. Snowflake's 4.0 TB is itself an estimate from column
   widths. The verdict was re-checked at 4.0 TB: $388.78, still a
   pass, and storage is pass-through at the same list price on both
   sides, so the margin does not care.
6. **Control plane.** Carried at the modeled $82, as the brief for
   this run fixed it. It is 27 percent of the measured total, the
   largest single line after W4. That is the prototype shape (one
   tenant, no redundancy), and per-account it falls as tenants are
   added. Flagged, not hidden: if the control plane cost doubled,
   the total would be $380.65, still a pass.

## Kill and park criteria

Park if any two hold (SOLUTION_DESIGN.md Section 8). One by one:

- **Target workloads need multi-node queries the wedge cannot
  serve.** Not held, on this evidence. Every runnable statement
  completed on one node, and the W4 peak of 30 concurrent queries
  completed with it. The honest limit: full-scale data was not run,
  so the single-node ceiling is located somewhere above 1:1000
  scale, unproven at 4 TB.
- **Free-tier cost breaks the $10k to $30k target at 70 percent
  gross margin.** Not held. The measured set carries at about
  $11.9k a year (about $15.6k with storage at 4.0 TB), inside the
  band.
- **MotherDuck closes the open-source and self-host gap before
  opensnow differentiates.** Not evaluated by this run. It is a
  market question and this run collected no evidence on it.
- **Services-layer scope creeps toward Snowflake parity before
  revenue exists.** Not held as a cost matter, and the run sharpens
  the watch item: the parity pressure is now a named list (MERGE,
  temp tables, UPDATE with FROM, INFORMATION_SCHEMA) instead of a
  general worry. None of it blocked the gate's cost question. All of
  it is Phase 2 scope discipline.

Zero criteria held, one not evaluated. The park rule needs two.

## What this run did not measure

- Full-scale data (4 TB). The VM cannot hold or scan it. The verdict
  rests on two scale points and linear extrapolation, stated above.
- MERGE write time, and the W5 daily refresh's hours (carried).
- The instance size each workload needs at full scale.
- Anything on AWS. No instance was started; all prices are list
  prices applied to local measurements.
- Result egress. None in a local run; the model's flag stands.

## Reproduce

```sh
export CARGO_HOME="$PWD/spikes/B2-iceberg/cargo"
export RUSTUP_HOME="$PWD/spikes/B2-iceberg/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
cargo test -p opensnow-gate1 --offline          # harness at 1:200000, all checks
cargo build --release -p opensnow-gate1 --offline
target/release/opensnow-gate1 --divisor 1000    # the run in this report
target/release/opensnow-gate1 --divisor 10000   # the scaling point
```
