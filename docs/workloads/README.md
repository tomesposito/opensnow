# Reference Workload Set

This is the workload set Gate 1 is judged against (SOLUTION_DESIGN.md Sections 2.4 and 8). Six workloads that together look like one month in the life of the target segment: a team that runs its analytics on Snowflake today and pays close to the median Snowflake bill for it.

The set is a model, built from documented Snowflake usage patterns. It is not telemetry from a real account. Every price input traces to docs/research/01-snowflake-research.md. Where the research has no price, this document says so and states the assumption instead of inventing a number.

Gate 1 passes when the Phase 1 prototype serves this whole set, on our own engine, single node, at about a tenth of the Snowflake cost computed below. The last section works out what a tenth is.

## Pricing inputs

From docs/research/01-snowflake-research.md, US AWS on-demand list prices:

- Warehouse credits per hour, Gen1: X-Small 1, Small 2, Medium 4, Large 8, X-Large 16. Each size doubles the last.
- Price per credit by edition: Standard $2, Enterprise $3, Business Critical $4, VPS about $6.
- Billing is per second, with a 60 second minimum every time a warehouse starts or resumes.
- Auto-suspend on a new warehouse defaults to 600 seconds. The warehouse bills for the whole idle tail before it suspends.
- Storage is $23 per TB per month, on-demand.
- Cloud services are free up to 10 percent of daily warehouse credits and billed above that.

## Assumptions

These are choices, stated so anyone can rework the arithmetic with different ones.

- **Edition: Enterprise, $3 per credit.** The research notes most production organisations are forced onto Enterprise because multi-cluster warehouses, masking, and 90-day Time Travel are not optional at scale. A team in the target segment is assumed to be one of them.
- **Gen1 warehouse rates.** Gen2 warehouses cost about 35 percent more credits per hour on AWS per the research. Gen1 is used throughout. That understates the Snowflake cost.
- **On-demand prices, no capacity contract.** Prepaid capacity cuts roughly 20 to 35 percent per the research. The median account is modeled as paying list. This also understates nothing and overstates nothing for an on-demand account; a discounted account would lower the totals below.
- **Month: 30 days, 22 of them weekdays.**
- **Cloud services: $0.** Every workload below is assumed to stay inside the free 10 percent daily allowance. A metadata-heavy account can breach it. The agent workload (W6) is the most likely candidate.
- **Serverless features: $0.** No Snowpipe, no Automatic Clustering, no materialized view maintenance, no Cortex. A real account pays some of these. Another understatement.
- **Billed time includes the idle tail.** For each workload the run mix states active query time, the auto-suspend setting, and the tail time the warehouse bills while idle. Billed time is active plus tail. The 60 second minimum binds only when a whole run segment (start to suspend) lasts under a minute; the per-workload notes say where that happens.

## The shared dataset

All six workloads run against one small data platform, the way one team's Snowflake account actually looks. Table sizes are stored bytes in Snowflake's native format, estimated from row counts and column widths. The sales tables extend the shape spike B1 tested (spikes/B1-engine/FINDINGS.md), scaled up.

| Table | Rows | Stored size | Notes |
|---|---|---|---|
| sales_fact | 2,000,000,000 | 1.8 TB | order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount |
| clickstream_events | 6,000,000,000 | 1.5 TB | event_id, session_id, customer_id, event_type, page, event_ts |
| orders_raw | about 350,000,000 | 300 GB | Landing table, 50,000,000 new rows a day, 7 days retained. Same columns as sales_fact plus loaded_at |
| agg and feature tables | varies | 400 GB | Daily aggregates and ML feature tables the workloads build |
| dim_customer | 2,000,000 | 2 GB | customer_id, customer_name, segment, country, signup_date |
| dim_product | 500,000 | 0.5 GB | product_id, category, product_name, brand |

Total stored: about 4.0 TB. Storage cost: 4.0 x $23 = **$92 per month**. Storage is one shared line for the whole account, listed once in the summary, not per workload.

## W1: Nightly ELT

**What it is.** The nightly batch job. Raw orders land in orders_raw during the day. At night the job deduplicates them, merges them into sales_fact, refreshes the daily aggregate tables, and runs data quality checks. Every team in the segment has some version of this job.

**Queries.** nightly-elt.sql. A QUALIFY dedupe (the window shape from spike B1, Q5), a MERGE into sales_fact, a CTE rollup that rebuilds the daily revenue aggregate (B1 Q6 shape), and count checks.

**Run mix.** One run a night, 30 nights a month. One session, no concurrency. A Large warehouse, the size a user picks when the window matters: the job has to finish before the dashboards refresh. Active time 2.0 hours a night. Auto-suspend 60 seconds, so the tail is 1 minute a night.

**Cost.**
- Billed time per night: 2.0 hr active + 1/60 hr tail = 2.02 hr
- Billed time per month: 2.02 x 30 = 60.5 hr
- Credits: 60.5 hr x 8 credits/hr = 484 credits
- Cost: 484 x $3 = **$1,452 per month**

## W2: BI dashboards

**What it is.** The morning dashboards. Twelve people in sales and operations open the same set of dashboards through the business day: revenue by region and month, top products, funnel counts. Short queries, repeated with different filters. This is the workload the default auto-suspend setting taxes hardest.

**Queries.** bi-dashboards.sql. Group by region and month (B1 Q2), a top 10 sort (B1 Q4), and a filtered scan aggregate (B1 Q1), each with the filter changes a dashboard tool sends.

**Run mix.** Weekdays only. A Small warehouse on the new-warehouse default auto-suspend of 600 seconds; nobody on this team changed it. About 14 resume sessions a weekday. Active query time totals 3.5 hours a weekday. Nine of the sessions are followed by a full 10 minute idle tail before the next query lands, which adds 90 minutes, 1.5 hours of billed tail; the remaining sessions resume inside the tail window and add nothing. Total billed: 5.0 hours a weekday. Every session runs longer than a minute once the tail counts, so the 60 second minimum never adds anything on top.

**Cost.**
- Billed time per month: 5.0 hr x 22 weekdays = 110 hr
- Credits: 110 hr x 2 credits/hr = 220 credits
- Cost: 220 x $3 = **$660 per month**

Note what the tail costs: 1.5 of the 5.0 billed hours is idle time, $198 of the $660. That is the research's central complaint about the meter, in one line of this model.

## W3: Ad-hoc analyst analysis

**What it is.** Three to five analysts asking questions nobody scheduled. Joins across the fact and dimension tables, rankings, one-off investigations that start from a dashboard number and dig. Long queries, low concurrency, working hours only.

**Queries.** adhoc-analysis.sql. A join aggregated by category (B1 Q3), a window rank of top customers per region (B1 Q5), and a filtered scan (B1 Q1) with the kind of nested conditions an analyst writes by hand.

**Run mix.** Weekdays only. A Medium warehouse, auto-suspend set to 60 seconds by an analyst who read the billing docs. Five analysts. Each analyst's sessions bill 1.0 hour a day in total: about 0.9 hours of active query time plus about 0.1 hours of 60 second tails across roughly six sessions. No session is short enough for the 60 second minimum to bind past its tail.

**Cost.**
- Billed time per day: 5 analysts x 1.0 hr = 5.0 hr
- Billed time per month: 5.0 x 22 = 110 hr
- Credits: 110 hr x 4 credits/hr = 440 credits
- Cost: 440 x $3 = **$1,320 per month**

## W4: Embedded customer-facing analytics

**What it is.** The team's product shows charts to its own customers. A small B2B SaaS: customers log in during their workday and every screen fires a few aggregate queries against sales_fact and clickstream_events. Many small queries, steady all day, spiky at the top of the hour. The queries are simple. The cost comes from the warehouse never getting a chance to stop.

**Queries.** embedded-analytics.sql. Selective filtered aggregates (B1 Q1 shape with tight predicates), a top 10 per customer (B1 Q4), and a per-customer monthly rollup (B1 Q2).

**Run mix.** Every day. A Medium warehouse on the default 600 second auto-suspend. Customer activity runs 8 AM to 4 PM in the customers' time zones, and inside that window a 10 minute idle gap long enough to suspend almost never happens. Billed time: the full 8 hour window plus one final 10 minute tail after the last customer query of the day, 8.17 hours a day, 245 hours a month. Concurrency peaks around 30 simultaneous queries; the model prices the single Medium the team actually runs and accepts the queueing, which matches the design's v1 scope (no multi-cluster).

**Cost.**
- Billed time per month: 8.17 hr x 30 = 245 hr
- Credits: 245 hr x 4 credits/hr = 980 credits
- Cost: 980 x $3 = **$2,940 per month**

## W5: ML feature builds

**What it is.** A data scientist builds customer features for a churn model: recency, frequency, spend percentiles, activity trends. Four big builds a month, each chewing through the full fact and clickstream tables, plus a small daily refresh of yesterday's features.

**Queries.** ml-feature-builds.sql. Window percentiles and ranks over the fact table (B1 Q5 at full scale), a CTE that compares each month against the trailing average (B1 Q6), a large join to clickstream (B1 Q3), and the MERGE that writes the feature table.

**Run mix.** The big builds run on an X-Large warehouse, the size a user picks to make a full-table job finish: 4 builds a month, 3.0 active hours each, one session each, auto-suspend 60 seconds (the tail rounds away at this scale). The daily refresh runs on a Small warehouse: 0.5 active hours a day, 30 days.

**Cost.**
- Big builds: 4 x 3.0 hr = 12 hr; 12 hr x 16 credits/hr = 192 credits
- Daily refresh: 0.5 hr x 30 = 15 hr; 15 hr x 2 credits/hr = 30 credits
- Total credits: 192 + 30 = 222 credits
- Cost: 222 x $3 = **$666 per month**

## W6: Agent-driven queries

**What it is.** An AI agent works against the warehouse during the day: a coding agent answering questions about the business, checking numbers, exploring schema. It issues many small commands in bursts, with pauses while it reads results and thinks. This is the segment the design says Snowflake skips, and the workload its meter handles worst.

**Queries.** agent-queries.sql. Schema exploration against INFORMATION_SCHEMA, small probe aggregates (B1 Q1 at tiny scale), TRY_CAST data checks, and a sample-based look at a new table. Short statements, most under two seconds each.

**Run mix.** Weekdays only. An X-Small warehouse on the default 600 second auto-suspend. Across an 8 hour agent workday the gaps between bursts rarely reach 10 idle minutes, so the warehouse bills the full window plus one final 10 minute tail: 8.17 hours a day. Concurrency is 1; the agent is one user who never sleeps between commands for long.

**Cost.**
- Billed time per month: 8.17 hr x 22 = 180 hr
- Credits: 180 hr x 1 credit/hr = 180 credits
- Cost: 180 x $3 = **$540 per month**

The sensitivity is worth stating. If the agent's pauses did stretch past 10 minutes, each isolated command would pay far more per second of work: the research documents a one-second query on a default warehouse billing 601 seconds (the 60 second minimum is not what binds; the 600 second tail after resume is). The direction of the agent era is more commands from more agents, and every shape of that traffic pays the tail tax on this meter.

## Summary

| Workload | Warehouse | Credits per month | Snowflake cost per month |
|---|---|---|---|
| W1 Nightly ELT | Large | 484 | $1,452 |
| W2 BI dashboards | Small | 220 | $660 |
| W3 Ad-hoc analyst analysis | Medium | 440 | $1,320 |
| W4 Embedded customer-facing analytics | Medium | 980 | $2,940 |
| W5 ML feature builds | X-Large + Small | 222 | $666 |
| W6 Agent-driven queries | X-Small | 180 | $540 |
| **Compute subtotal** | | **2,526** | **$7,578** |
| Storage, 4.0 TB at $23/TB | | | $92 |
| **Total** | | | **$7,670 per month** |

That is **$92,040 a year**, close to the median Snowflake account of about $100k a year that the business case targets (docs/BUSINESS-CASE.md).

**The Gate 1 target line: a tenth of $7,670 is $767 per month, about $9,200 a year.** Gate 1 passes when the prototype serves this set at or under that line, costed on our real AWS costs (the slice 0.3 cost model), on the same run mixes. Note that a tenth lands just under the business case's $10k to $30k all-in offer band. Serving the set at a tenth of Snowflake's price and selling it inside the band are the same claim, checked two ways.

Two honesty notes on the total. First, it is understated on purpose in the places listed under Assumptions: Gen1 rates, no serverless features, cloud services inside the free allowance. A real account with this shape likely pays more, which makes the tenth a harder target, not an easier one, relative to what the team actually spends today. Second, the run mixes are the softest inputs. Active hours and tail counts are modeled, not measured. Slice 0.3's cost model should record which of these numbers the Gate 1 run treats as fixed inputs, so the gate is judged on the same month this document describes.
