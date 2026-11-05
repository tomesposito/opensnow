# Business Case

Written Nov 5, 2026. Evidence in docs/research/03-business-case-research.md. Scored on the same 90-point rubric used for the Oct 31, 2026 market deep dive and the Principal PM case.

## Verdict: GO SMALLER — 59/90

Build the wedge, not the clone. A full Snowflake clone is a no-go: Firebolt raised $269M and still did not buy distribution against Snowflake and Databricks. The smaller version survives because the agent funnel, the open format, and open source do the distribution work that Snowflake buys with a sales force (44% of its revenue goes to sales and marketing).

| Criterion | Score |
|---|---|
| Cornerable wedge | 6/10 |
| Growth | 9/10 |
| Fragmentation | 8/10 |
| Incumbent weakness | 9/10 |
| Ability to pay | 7/10 |
| Data network | 6/10 |
| Build cost / reuse | 6/10 |
| First-100 distribution | 5/10 |
| Low risk | 3/10 |
| **Total** | **59/90** |

Founder fit: 10, counted separately as usual.

## The wedge

- Segment: the priced-out. Teams spending $5k to $150k a year on Snowflake, plus the agent builders and local-first teams Snowflake does not serve at all.
- Engine: borrow the best permissive open-source core. Do not write a storage engine.
- Format: Iceberg, in the customer's bucket, by default.
- Pricing: Snowflake's usage model at a fraction of the price, won on lower gross margin (Principle 2). Permanent free tier at or above MotherDuck parity. No per-seat pricing, ever.
- Experience: the layer users live in ships with the wedge, sequenced by Principle 4 and the decision queue.

## The offer

Serve the median Snowflake workload (about $100k a year) for $10k to $30k a year, all in, starting at $0. The median contract is the target, not the mean: reported spend is heavily skewed (mean product revenue per customer about $336k against a roughly $100k median contract).

## Gates

Written before any build. Both must pass before this case scales up.

- **Gate 1:** a DuckDB-centric prototype serves a real design-partner workload at about a tenth of its Snowflake cost, on Snowflake's own arithmetic, without needing distributed execution.
- **Gate 2:** 10 paying orgs at full price, no discount doing the work, converted from the free tier at 1% or better.

## Kill / park criteria

Park if any two hold:

- Design partners need multi-node queries the wedge cannot serve.
- Free-tier cost breaks the $10k to $30k target at 70% gross margin.
- MotherDuck closes the open-source and self-host gap before opensnow differentiates.
- Services-layer scope creeps toward Snowflake parity before revenue exists.

## What could not be verified

Snowflake bill percentiles and edition mix; deduplicated warehouse-user counts by segment; MotherDuck customers or revenue; BigQuery and Redshift revenue (not disclosed); independent price-performance benchmarks at small scale; Snowflake churn decomposition. The case does not depend on any of these, but the design phase should treat the Snowflake-cost model of a design partner as the first thing to nail down with real bills.
