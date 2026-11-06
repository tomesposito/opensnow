# Cost Model and Gate 1 Scorecard

This is the cost model Gate 1 is judged against (slice 0.3; SOLUTION_DESIGN.md Sections 2.4 and 8). It takes the run mixes from docs/workloads/README.md as given, prices our side on real AWS list prices, and works out three things: what serving the workload set should cost us, whether that cost clears the Gate 1 line (a tenth of the Snowflake cost, $767 per month), and whether it clears the business case's margin ceiling (the $10k to $30k a year offer band at about 70 percent gross margin).

The last section is the scorecard the Phase 1 Gate 1 run fills in.

## Conventions

- **Region: us-east-1** (US East, N. Virginia). It is the reference region for AWS list prices, it is where the Snowflake prices in docs/workloads/ come from, and nothing in the wedge needs a specific region yet. A customer in another region changes both sides of the comparison by roughly the same factor.
- **On-demand list prices on our side too.** Slice 0.1 priced Snowflake at list with no capacity contract. We do the same: no Savings Plans, no reserved instances, no spot. Those levers exist later and only help. Gate 1 is judged on the conservative version.
- **Prices checked Nov 5, 2026.** Each price below names its source. AWS changes prices; the scorecard run should re-check this table before it fills in its numbers.
- **Month: 30 days, 22 of them weekdays**, matching docs/workloads/.

## 1. AWS unit costs

### Compute (EC2, Linux, on-demand, us-east-1)

The engine is a single node in v1. The instance family is m7i: general purpose, 4 GiB of memory per vCPU, current generation Intel. Four sizes cover every workload in the set.

| Instance | vCPU | Memory | Price per hour |
|---|---|---|---|
| m7i.large | 2 | 8 GiB | $0.1008 |
| m7i.xlarge | 4 | 16 GiB | $0.2016 |
| m7i.2xlarge | 8 | 32 GiB | $0.4032 |
| m7i.4xlarge | 16 | 64 GiB | $0.8064 |

The family prices linearly: $0.0504 per vCPU-hour at every size. Source: AWS on-demand price list for us-east-1, as published by usage.ai (rates checked Oct 18, 2026 against the AWS EC2 on-demand page) and cross-checked against DoiT's EC2 price index for the same sizes (accessed Nov 5, 2026). The two sources agree at every size in the table.

EC2 bills per second with a 60 second minimum, so a node that runs for 10 minutes costs 10 minutes. There is no 600 second idle tail. A stopped node costs nothing but its disk.

### Storage and requests (S3 Standard, us-east-1)

| Item | Price |
|---|---|
| Storage, first 50 TB | $0.023 per GB-month |
| GET and other read requests | $0.0004 per 1,000 |
| PUT, COPY, POST, LIST requests | $0.005 per 1,000 |
| Data transfer, S3 to EC2 in the same region | $0 |
| Data transfer out to the internet | $0.09 per GB (first 10 TB tier) |

Source: AWS S3 pricing page rates for US East, as reproduced by the S3 cost references at cloudfiles.io (verified Jul 31, 2026) and calculatorzilo.com (checked Oct 2026), accessed Nov 5, 2026. The rates agree across sources.

Two things worth noticing. First, storage at list is $23 per TB-month, the same number Snowflake charges for storage. There is no margin in storage and the model takes none. Second, GET requests are nearly free and PUT requests are cheap, but neither is free at scale: 10 million GETs cost $4 and 1 million PUTs cost $5. File layout decides whether request costs stay in that range (see Soft spots).

### Disk for the control plane

gp3 volumes at $0.08 per GB-month (AWS list, us-east-1). The model carries 100 GB for the control plane node: $8 per month.

## 2. How a run mix becomes our cost

The mapping rules. Each one is an assumption, stated so the Gate 1 run can be judged against it.

1. **We pay for active time, not windows.** Snowflake bills active time plus idle tails, because its warehouses stay up for 600 seconds after the last query by default. Our nodes scale to zero: a workload pays for the hours its queries actually run, plus a small stop allowance where a nightly job needs the node to shut down cleanly. The one exception is W4 and W6, where queries arrive all day and the node stays up for the window. There the win is not hours. It is the price per hour (see W6 below).
2. **Each workload is costed standalone, on its own instance.** That is the same convention slice 0.1 used for Snowflake warehouses, and it is the conservative one. In the real product, daytime workloads share nodes through per-tenant resource groups, which only lowers the total. If the model clears the line standalone, sharing is headroom, not a requirement.
3. **Instance size per workload is picked by data scanned and concurrency, not by Snowflake's size names.** A Snowflake credit is not a vCPU and the two do not convert. The picks: W1 and the W5 big builds get m7i.4xlarge (they chew through terabytes and have a window to hit); W2, W3, and W4 get m7i.2xlarge; the W5 refresh and W6 get m7i.xlarge. These picks are assumptions. The Gate 1 run measures the size each workload actually needs.
4. **Active hours start from the Snowflake run mix.** The model assumes our engine takes the same active hours per workload that slice 0.1 modeled for Snowflake. This is the softest assumption in the document (see Soft spots). The scorecard exists to replace it with measured runtime.
5. **Storage sits in the customer's own bucket by default** (SOLUTION_DESIGN.md Section 2.2), so storage never touches our bill in the default deployment. The model still carries it as a line, priced at cost, because Gate 1's prototype may run in our account and because a hosted-bucket variant needs the number. At cost means at cost: $94 a month for the 4.0 TB set, against the $92 Snowflake charges. Nobody makes money on this line, on either side.
6. **Shared lines are counted once**: the control plane (catalog, metering, API) on one always-on m7i.large at 730 hours a month, $73.58, plus its $8 disk, rounded to $82. Plus a $10 a month S3 request allowance (see Soft spots for why it is an allowance and not a computation).
7. **Result egress is not in the model.** Query results are small next to scanned data and same-region traffic is free. If a customer pulls large result sets out to the internet, egress at $0.09 per GB applies: 1 TB out is $90. The product bills that through or caps it; Gate 1 does not test it. Flagged, not hidden.

## 3. Per-workload model

Snowflake costs and targets come from docs/workloads/README.md. The target per workload is a tenth of its Snowflake cost.

| Workload | Instance | Node hours per month | Our modeled cost | Snowflake cost | Target (a tenth) | Ratio |
|---|---|---|---|---|---|---|
| W1 Nightly ELT | m7i.4xlarge | 60.5 | $48.79 | $1,452 | $145.20 | 29.8x |
| W2 BI dashboards | m7i.2xlarge | 77 | $31.05 | $660 | $66.00 | 21.3x |
| W3 Ad-hoc analyst analysis | m7i.2xlarge | 99 | $39.92 | $1,320 | $132.00 | 33.1x |
| W4 Embedded analytics | m7i.2xlarge | 240 | $96.77 | $2,940 | $294.00 | 30.4x |
| W5 ML feature builds | m7i.4xlarge + m7i.xlarge | 12 + 15 | $12.70 | $666 | $66.60 | 52.4x |
| W6 Agent-driven queries | m7i.xlarge | 176 | $35.48 | $540 | $54.00 | 15.2x |
| **Compute subtotal** | | | **$264.71** | **$7,578** | **$757.80** | |
| Storage, 4.0 TB in S3 at cost | | | $94 | $92 | $9.20 | |
| S3 requests (allowance) | | | $10 | | | |
| Control plane (always on) | m7i.large | 730 | $82 | | | |
| **Total** | | | **$451 per month** | **$7,670** | **$767.00** | **17.0x** |

The arithmetic, per workload:

- **W1:** 2.0 active hours a night x 30 nights = 60 hr, plus 0.5 hr a month of stop allowance (about a minute a night). 60.5 hr x $0.8064 = $48.79.
- **W2:** 3.5 active hours a weekday x 22 = 77 hr x $0.4032 = $31.05. The 1.5 hours a day of idle tail in the Snowflake mix ($198 a month there) costs us nothing.
- **W3:** 5 analysts x 0.9 active hours a day x 22 = 99 hr x $0.4032 = $39.92.
- **W4:** the node is up for the 8 hour customer window, 30 days: 240 hr x $0.4032 = $96.77. Our suspend tail is seconds, so there is no tail line.
- **W5:** big builds 12 hr x $0.8064 = $9.68; daily refresh 15 hr x $0.2016 = $3.02. Total $12.70.
- **W6:** the node is up for the 8 hour agent workday, 22 days: 176 hr x $0.2016 = $35.48. Compare the hour price: Snowflake's X-Small bills 1 credit an hour at $3. The node that replaces it costs $0.2016 an hour. W6 is the workload where Snowflake's meter looks worst and it is still our thinnest ratio, because we pay for the same window hours. If W6 shared the daytime node with W2 and W3 instead of standing alone, its marginal cost would fall toward zero. The standalone convention is what makes it look tight.

**The model lands at $451 a month against a $767 line.** That is 59 percent of the line, with every workload under its own target except the storage line, which cannot be under a tenth because S3 list price is the price Snowflake itself charges. Storage is pass-through and the gate is judged on the total, the way slice 0.1 stated it.

## 4. The margin ceiling

The business case offers the median workload at $10k to $30k a year, all in, at about 70 percent gross margin. Gross margin of 70 percent means cost is at most 30 percent of price. That sets a cost ceiling per price point:

| Price per year | Cost ceiling per year (30 percent) | Cost ceiling per month |
|---|---|---|
| $10,000 | $3,000 | $250 |
| $18,000 | $5,400 | $450 |
| $20,000 | $6,000 | $500 |
| $30,000 | $9,000 | $750 |

Read against the model:

- Modeled cost is $451 a month, $5,409 a year. The price that carries it at exactly 70 percent margin is $5,409 / 0.30 = about **$18,000 a year**, inside the band.
- At the top of the band, $30k, the modeled cost gives a margin of 1 - 5,409/30,000 = 82 percent.
- At the bottom of the band, $10k, the same cost gives 46 percent. The bottom of the band does not carry this workload set at 70 percent margin. It carries a smaller account, or the same account at a lower margin. The band and the margin target only meet from about $18k up, for a workload set of exactly this size.
- Note where the Gate 1 line sits in this table: $767 a month is $9,204 a year of cost, which needs a $30,680 price to clear 70 percent. Passing Gate 1 exactly at the line is not the same as clearing the margin ceiling inside the band. The model clearing the line with room ($451 against $767) is what makes the offer work. Gate 1 should report both numbers.

This ceiling covers infrastructure cost only: compute, storage, requests, control plane. Support, billing operations, and the free tier's cost sit outside it. The business case's kill criteria use the same 70 percent figure, so the comparison is apples to apples as far as it goes, and it goes as far as infrastructure.

## 5. Soft spots

Flagged rather than smoothed over, in the order they can hurt:

1. **Runtime parity is assumed, not measured.** Rule 4 gives our engine Snowflake's active hours. Spike B1 says that is plausible for scans and group-bys at 1M rows and optimistic for window functions, which ran about 2.2x the DuckDB yardstick. W1 and W5 are window-heavy and run at terabyte scale, which no spike has touched. Sensitivity: the shared lines are $186 a month, so the compute budget under the line is $767 - $186 = $581, against $265 modeled. The model stays under the Gate 1 line until our runtime is about 2.2x Snowflake's across every workload at once (581 / 265 = 2.19). That is real headroom, and it is still an assumption until the Gate 1 run replaces it.
2. **Instance sizes are picks, not measurements.** If W4's 30 concurrent queries need an m7i.4xlarge instead of a 2xlarge, W4 doubles to about $194 and the total rises to about $548. Still under the line, but the direction is one-way: every size correction so far imagined goes up.
3. **Start and stop behavior is idealized.** The model assumes nodes start when work arrives and stop when it ends, with sub-minute overhead. Real EC2 starts take about a minute, and per-session start/stop for W2's 14 sessions a day would add node hours the model does not carry. The product answer is shared nodes that stay up through the business day, which moves cost from W2's line into shared capacity. Either way, some hours land somewhere that the model currently prices at zero.
4. **The S3 request line is a $10 allowance.** Request cost depends on file counts: a query that touches 10,000 small files pays 10,000 GETs. Compaction keeps files large (the design targets 128 to 512 MB), and at those sizes the allowance is generous. Before compaction works, or on a table that arrives as small files, this line can grow by an order of magnitude. The research's warning stands: 100k small files can cost about $800 a day in requests at high query volume. That extreme is not this workload set, but the mechanism is real.
5. **Parquet size is not Snowflake size.** The 4.0 TB dataset is measured in Snowflake's native format. Iceberg plus Parquet will store the same rows at a somewhat different size, in either direction. The storage line moves with it. It is a pass-through line, so the margin does not care; the Gate 1 total does, a little.
6. **The control plane is one small node.** That prices the Gate 1 prototype, one tenant shape, no redundancy. A production control plane with failover costs more. It is shared across every tenant on the platform, so its per-account cost falls as tenants are added. At one account it is $82 of a $451 model, which flatters nothing and should be read as a prototype cost, not a steady-state one.

## 6. Gate 1 scorecard

The Phase 1 run fills in the measured columns on the same run mixes. Snowflake cost and target are fixed by slice 0.1. Ratio is Snowflake cost divided by measured cost. The gate passes when the measured total is at or under $767 per month and the margin check in Section 4 is reported alongside it.

| Workload | Snowflake cost per month | Target (a tenth) | opensnow measured cost per month | Ratio | Pass / fail |
|---|---|---|---|---|---|
| W1 Nightly ELT | $1,452 | $145.20 | | | |
| W2 BI dashboards | $660 | $66.00 | | | |
| W3 Ad-hoc analyst analysis | $1,320 | $132.00 | | | |
| W4 Embedded customer-facing analytics | $2,940 | $294.00 | | | |
| W5 ML feature builds | $666 | $66.60 | | | |
| W6 Agent-driven queries | $540 | $54.00 | | | |
| Compute subtotal | $7,578 | $757.80 | | | |
| Storage (at cost, pass-through) | $92 | $9.20 | | | |
| S3 requests | n/a | n/a | | | |
| Control plane | n/a | n/a | | | |
| **Total** | **$7,670** | **$767.00** | | | |

### Fixed inputs and modeled inputs

Slice 0.1 asked this document to record which of its numbers the Gate 1 run treats as fixed. Here is the split.

**Fixed: the run treats these as given and does not re-litigate them.**

- Run mixes, per workload, exactly as docs/workloads/ states them: sessions per day, active hours, days per month (30 days, 22 weekdays), and the suspend settings on the Snowflake side (600 second default for W2, W4, W6; 60 seconds for W1, W3, W5).
- Dataset: the shared 4.0 TB set, table by table, with the row counts in slice 0.1.
- Concurrency shapes: W4 peaking at about 30 simultaneous queries, W6 at 1, W3 at 5 analysts.
- Snowflake price inputs: Enterprise edition at $3 per credit, Gen1 credit rates, $23 per TB storage, cloud services and serverless features at $0.
- The Gate 1 line itself: a tenth of $7,670 is $767 per month, judged on the total.

**Modeled: the run measures these and the scorecard reports what it finds.**

- Our engine's actual runtime per workload, against the Snowflake active hours the model borrowed.
- The instance size each workload actually needs, against the picks in Section 2, rule 3.
- Idle node hours on our side: what scale-to-zero actually achieves across a business day, including start overhead per session.
- S3 request counts per workload, and the file layout that produces them.
- Stored size of the dataset in Iceberg and Parquet, against the 4.0 TB Snowflake figure.
- Result egress per workload, if any leaves the region.

If the run wants to change a fixed input, that is a finding about the workload set, and it goes back through slice 0.1 as an amendment. The gate is judged on the month slice 0.1 describes.
