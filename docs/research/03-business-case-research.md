# opensnow Business Case — Market / Pricing / GTM / Funding Evidence

**Scope:** Business/market/pricing only, per mission. Snowflake product research (FY2026 revenue $4.684B, product $4.472B, NRR 125%, 733 customers >$1M TTM, ~13,328 customers, $2–6/credit) and technical research (DuckDB/DataFusion/ClickHouse/Iceberg) taken as given and not repeated except where a filing number is needed for a business ratio.
**Date read:** 2026-11-05 (Thu). **Flags:** `verified live` = opened on the vendor/primary page in this run; `index` = search-index result only, not opened live. Vendor/analyst estimates are flagged as such. Low-quality market-research mills (Emergen, MRFR, Dataintelo, Verified Market Research etc.) are reported *as a range* and should not be quoted singly in a pitch — see §1.1.

## Summary — the business case in 10 lines

1. **The market is large under every definition, but definitions move the number 5–20×.** Gartner's audited-adjacent DBMS total was **$119.7B in 2024, +13.4%, 64% cloud** [verified live]. Narrow "cloud warehouse / DWaaS" mills put 2025 at **$6.5–14B**; broad "data warehousing (incl. on-prem + services)" at **$32–39B in 2025**; lakehouse alone at **$11–14B in 2024–25**. Anyone quoting one TAM without a definition is misleading you.
2. **The centre of gravity has already moved.** Databricks' self-reported run-rate overtook Snowflake in 2025–26: **$4.8B (Dec 2025, +55%) → $5.4B (Feb 2026, +65%) → >$7B (Aug 2026, +80%)**, with data warehousing alone >$1B run-rate and AI products $1.4B. Snowflake FY26 audited product growth was ~29–30%.
3. **Median Snowflake spend is ~$97–100k/yr (Vendr, 622–633 deals), but the floor excludes the bottom.** Capacity contracts start at **$25,000/yr**; on-demand has no minimum but no discount, a 60-second billing minimum, and Enterprise (the edition most mid-market/enterprise buyers actually deploy) at $3–4/credit. That is the priced-out segment.
4. **Price pain is documented, not anecdotal:** Instacart payments $13M→$28M→$51M (2020–22, SEC S-1) then an anticipated ~$15M in 2023 — Snowflake itself says usage was flat ~$28M and the gap was "savings through optimization" (i.e. ~50% waste was removable); Gartner clients report they cannot predict Snowflake spend and hire FinOps vendors for it; an entire cost-optimisation tooling category exists.
5. **Named migrations exist but are mostly vendor-published:** Definite Snowflake→DuckDB >70% saving (via MotherDuck), Appcues/Buildkite →ClickHouse Cloud, plus ClickHouse-vs-Snowflake list-price modelling of 40–55% savings. Treat percentages as vendor claims, direction as real.
6. **The bottom is already being served — by free/cheap DuckDB-shaped things.** DuckDB: **~25M PyPI downloads/mo (Oct 2025, primary) → 50M+ /mo (Aug 2026)**, Stack Overflow usage 1.4%→3.3% in a year, 40k GitHub stars. MotherDuck monetises exactly that with a permanent free tier.
7. **Free-tier norm in this category is now set:** MotherDuck 10 GB + 10 Pulse-hours/mo free, no card; BigQuery 1 TiB queries + 10 GB/mo free forever; Neon 100 CU-hours + 0.5 GB × 100 projects. ClickHouse Cloud is the outlier — *no* permanent free tier, $300/30 days only. An opensnow without a permanent free tier would be below market norm.
8. **Snowflake's cost structure is the attack surface:** FY26 GAAP sales & marketing **$2.062B = 44.0% of revenue**, R&D **$1.969B = 42.0%**, stock comp **34% of revenue** (down from 41% FY25). Non-GAAP S&M is lower (~32–35%) because it strips that stock comp — quote GAAP and non-GAAP together or not at all.
9. **Building a warehouse is not cheap, but the engine no longer has to be built:** Firebolt raised **$269M** to unicorn ($1.4B, Jan 2022) before meaningful revenue; MotherDuck raised **$100M** to build *only the hosted layer* on DuckDB; DuckLabs built DuckDB itself with **~30 people, zero VC**, on support/feature-prioritisation contracts; Bauplan launched a lakehouse on **$7.5M seed**. The spread is the strategy choice.
10. **AI/agent spend is real but still small as revenue, huge as query volume:** Snowflake AI run-rate $100M (Q3 FY26, a quarter early; ~2% of product revenue), Databricks AI $1.4B run-rate, agents now out-query humans on Databricks and launch >17M databases/day (a16z, interested source), 80%+ of Neon databases are created by agents. Per-second, scale-to-zero, no-minimum billing is the pricing shape agents reward.

---

## Findings

### 1. Market Size — cloud data warehouse / data platform / lakehouse

#### 1.1 Estimates disagree by definition — table of what was actually claimed

| Definition / source | 2024/2025 size | Forecast | CAGR | Flag |
|---|---|---|---|---|
| **Gartner — all DBMS** (licences + cloud, relational + non-relational) | **$119.7B in 2024**, +13.4% YoY; cloud dbPaaS = 64% of spend vs 36% on-prem | Gartner via secondary reporting: ~$161B by 2026 | ~13–16% implied | Primary abstract `verified live`: https://www.gartner.com/en/documents/6494271 |
| Snowflake's own platform TAM (built from Gartner infra + application software forecasts, Mar 2024) | $152B CY24 | **$342B** | — | Vendor claim, `index`: https://investors.snowflake.com/files/doc_financials/2025/q4/Q4-FY2025-Investor-Presentation_vFF.pdf — includes DBMS + data-management software ex-DBMS + application software; *not* a warehouse market |
| Grounded platform TAM (PitchGrade synthesis: cloud warehousing $14B + data engineering $8B + ML/AI platform $11B + real-time $6B) | **~$43B total in 2025**; argues credible competing TAM is $40–60B, Snowflake's $342B "implausibly large" | 2025–29 | ~+22% blended (warehouse +18%) | Analyst blog, `index`: https://pitchgrade.com/research/snowflake-vs-databricks |
| Data warehousing, broad (incl. on-prem, software + services) — Emergen | $38.42B in 2025 | $162.84B by 2035 | 15.6% (2026–35) | Mill estimate, `index`: https://www.emergenresearch.com/industry-report/data-warehousing-market |
| Data warehousing, broad — other mills | $31.8B (Dataintelo) / $32.38B (TechSci) / $37.6B (Verified) / $39.18B (9cv9 synthesis) in 2025 | $60–89B by 2031–34 | 8.2–12.2% | Mill estimates, disagree ±25% on the *same* year, `index`: https://dataintelo.com/report/global-data-warehouse-software-market |
| Cloud data warehouse only | $11.56B in 2025 → $14.94B in 2026 (9cv9 synthesis of forecasts) | $49.12B by 2031 / $31.7B by 2030 in rival forecasts | 21.5–26.9% | Synthesis, `index`: https://blog.9cv9.com/top-100-data-warehouse-software-statistics-data-trends-in-2026/ |
| Cloud DWaaS only (narrowest) — MRFR | **$6.52B in 2025** | $51.09B by 2035 | 23.5% | Mill estimate, `index`: https://www.marketresearchfuture.com/reports/data-warehouse-as-a-service-market/companies |
| Cloud DWaaS — Emergen (same label, different number) | $8.65B in 2025 → $11.20B in 2026 | $58.74B by 2035 | 20.2% | Mill estimate — **32% above MRFR for the identical 2025 label**, `index`: https://www.emergenresearch.com/de/industry-report/data-warehouse-as-a-service-market |
| MarketsandMarkets — DWaaS (old, still cited) | $4.7B in 2021 | $12.9B by 2026 | 22.3% | Dated 2021 forecast, `index`: https://www.marketsandmarkets.com/report-search-page.asp?rpt=warehouse%2Dautomation%2Dsoftware%2Dmarket&offset=20 — do not use as a 2025 number |
| Data lakehouse — Insight Partners | $11.27B in 2024 | $42.89B by 2031 | 18.5% (2025–31) | `index`: https://www.theinsightpartners.com/reports/data-lakehouse-market |
| Data lakehouse — Future Market Insights | $14B in 2025 | $112.6B by 2035 | 23.2% | Mill estimate, `index`: https://www.futuremarketinsights.com/reports/data-lakehouse-market |
| Data lakehouse — Business Research Co. (via USDSI) | $12.58B in 2026 | $27.28B by 2030 | 21.4% | `index`: https://www.usdsi.org/data-science-insights/how-will-data-lakehouses-transform-your-analytics-and-ml-workflows |
| Database market, all models — Dataintelo | $105.0B in 2025, cloud 55% of revenue | $303.1B by 2034 | 12.5% | Mill estimate, broadly consistent in scale with Gartner's $119.7B for 2024, `index`: https://dataintelo.com/report/global-database-market |

**Read:** Use Gartner $119.7B (2024, all DBMS, 64% cloud) as the anchor a CFO will accept, $14B-ish for cloud warehousing narrowly defined, and $40–60B for the platform TAM Snowflake/Databricks actually contest. The $342B Snowflake TAM and the $6.5B DWaaS figure are both true *under their definitions* and both useless without them.

Gartner 2025 update (via The Register, Apr 2026): top-5 DBMS vendors by revenue unchanged since 2022 — AWS, Microsoft, Oracle, Google, IBM in that order — with Snowflake, Databricks and MongoDB gaining share slowly from below; drivers in 2025 remained "cloud, analytics, and AI." `index`: https://www.theregister.com/software/2026/04/21/the-dbms-chart-that-shows-oracles-crown-is-slowly-slipping/5226401

#### 1.2 Snowflake vs Databricks vs BigQuery vs Redshift — revenue / share

| Vendor | Latest hard / reported number | Notes |
|---|---|---|
| Snowflake | FY26 (ended 31 Jan 2026) total $4.684B, product $4.472B, +29–30% (given; filing-confirmed in §5) | Audited. Q2 FY27 product +37% to $1.49B and FY27 guide raised to $6.07B in later reporting — growth re-accelerating, `index`: https://www.tradingnews.com/news/snowflake-blows-through-its-52-week-high-at-372-usd-as-coco-reaches-60-percent-of-customers |
| Databricks | Run-rate: $4.0B Sep 2025 (>50%) → $4.8B Dec 2025 (>55%) → **$5.4B Feb 2026 (>65%)** → **>$7B Aug 2026 (>80%)**; AI products $1.4B run-rate Feb 2026; warehouse (Databricks SQL) >$1B run-rate end-2025, >$1.5B by Aug 2026; NRR >140% (company-reported); >20,000 orgs, >800–1,000 customers >$1M | **Private, self-reported run-rate ≠ audited TTM revenue.** Series L Dec 2025: $4B+ at $134B valuation; Aug 2026: $5B round at $190B. `index`: https://www.saastr.com/growth-may-have-slowed-for-some-but-holy-cow-not-for-databricks/ and https://databricks.techfabric.com/guide/databricks-by-the-numbers/ and https://okara.ai/blog/databricks-revenue-growth |
| ClickHouse Inc | Run-rate >$350M Sep 2026, up from ~$200M end-2025; Cloud alone >$250M run-rate May 2026, "more than triple a year earlier"; Cloud ARR +250% YoY at Series D (Jan 2026); 3,000 Cloud customers Jan 2026 → >4,000 by May–Sep 2026 | Company-reported, `index`: https://techcrunch.com/2026/01/16/snowflake-databricks-challenger-clickhouse-hits-15b-valuation/ and https://venturebeat.com/business/clickhouse-appoints-mike-scarpelli-former-snowflake-and-servicenow-cfo-to-board-of-directors |
| BigQuery / Redshift | **No audited product revenue disclosed** — both buried in Google Cloud ($43.2B FY25 per MRFR table) / AWS totals | Any BigQuery or Redshift "revenue" or share number in a mill report is modelled, not reported |
| Share estimates (technographics, not revenue) | 6sense: Snowflake 21.07% of data-warehousing *installations tracked*, BigQuery 14.11%, Redshift 13.83%, dbt 8.55% (dbt is not a warehouse — shows the method's limits); VMR: Snowflake 35.1% of "pure-play cloud warehousing", BigQuery 27.8%, Databricks 12.5% | **Disagreement is definitional.** 6sense counts detected companies; VMR claims revenue share in a narrower segment. Neither is Gartner revenue share. `index`: https://technographics.6sense.com/data-warehousing/snowflake-market-share and https://www.verifiedmarketresearch.com/blog/top-7-data-warehouse-as-a-service-companies-offering-substantial-stability-for-businesses/ |

#### 1.3 How many companies use a cloud warehouse, by size

No authoritative census exists. Best available proxies, all flagged:

- Snowflake itself: ~13,328 customers at FY26-end (given), of which Forbes Global 2000 = 790, >$1M TTM = 733, >$10M = 56. So **~94% of Snowflake's own customers spend <$1M/yr and the long tail is the business**.
- 6sense technographics: **34,708 companies** detected using Snowflake as a data-warehousing tool in 2026 — ~2.6× Snowflake's customer count, because it counts detections/mentions, not contracts. By employees (partial table): 100–249: 8,161; 1,000–4,999: 5,272; 20–49: 5,085; 250–499: 4,257; 10,000+: 3,507. US = 64.2% of detections. `index`: https://technographics.6sense.com/data-warehousing/snowflake-market-share
- Databricks: >20,000 organisations, ~70% of Fortune 500 (company-reported, `index` above).
- ClickHouse Cloud: >4,000 customers in ~4 years from GA (company-reported).
- **Could not verify:** a credible SMB/mid-market/enterprise split for *all* cloud-warehouse users (BigQuery + Redshift + Snowflake + Databricks deduplicated). Vendors don't disclose it; mills invent it. See Could Not Verify.

#### 1.4 Spend share, data volumes, AI as growth driver

- Warehouse is only one of four bills: in worked mid-market examples the warehouse contract (~$100k median) is dwarfed by people. One synthesis (MotherDuck TCO analysis cited second-hand): true Snowflake TCO ≈ **2.4× the contract** once 60-second minimums, compute overhead and egress are counted; full stack for a ~200-person company modelled at $760–924k/yr of which 3 data engineers = $480–600k. Vendor-adjacent, directional only. `index`: https://aiadopters.club/p/your-data-vendor-is-charging-you
- Independent build/run guide (Oct 2026): small build (20–150 people) run cost $5–60k/yr before staff; mid-market example stack (Snowflake Enterprise + Fivetran ~$40k + dbt + Power BI); enterprise run $250k–$1M+/yr before staff. Fivetran median contract ~$44.8–45.9k (Vendr). `index`: https://www.dharmops.com/blog/data-warehouse-cost-2026
- Data volume backdrop: IDC Global DataSphere — 33 ZB (2018) → 175 ZB (2025) in the classic forecast; newer IDC-cited figures ~149 ZB in 2024 → 394 ZB by 2028. Both are IDC forecasts quoted second-hand; the *direction and order of magnitude*, not the exact ZB, is usable. `index`: https://www.komprise.com/glossary_terms/zettabyte/
- Lakehouse shift as spend migration: Dremio/McKnight survey of 563 data decision-makers (vendor-sponsored): 55% run the majority of analytics on lakehouses, projected 67% within 3 years; 41% of lakehouse users migrated *from cloud data warehouses*; cost efficiency was the #1 driver (19%). A second Dremio survey summary claims 56% expect >50% analytics cost savings from lakehouse moves. Vendor-sponsored — use as sentiment, not fact. `index`: https://www.Dremio.com/newsroom/why-data-lakehouses-are-poised-for-major-growth-in-2025/
- Gartner via synthesis: >75% of new enterprise warehouse deployments expected cloud-native by 2027; 78% of organisations use AI in at least one function (McKinsey 2026). `index`: https://dataintelo.com/report/global-data-warehouse-software-market

### 2. Customer Spend Distribution & Pain

#### 2.1 Snowflake spend distribution

| Segment | Annual spend | Source |
|---|---|---|
| Median contract | **$96,594** (CostBench/Vendr snapshot, 622 deals, median negotiated discount 8%) / **~$100,000** (Vendr, 633 purchases, cited in multiple secondaries) | `index`: https://costbench.com:443/compare/google-bigquery-vs-snowflake/ |
| Small / PoC | <$25k/yr | Vendr `verified live`: https://www.vendr.com/marketplace/snowflake |
| Mid-market | $25–100k/yr; Vendr comparison tables put "typical mid-market" at **$50–150k** | Vendr, as above |
| Enterprise | $100–500k/yr | Vendr, as above |
| Strategic | >$500k/yr, multi-million; Business Critical / VPS, custom | Vendr, as above |
| Top tail (Snowflake-disclosed) | 733 customers >$1M TTM; 56 >$10M (given, FY26) | Given + filing |
| Capacity minimum | **$25,000/yr** — on-demand has no minimum; capacity (prepaid, 20–30% off credits per Definite/Flexera) starts here. Storage drops $40/TB-mo on-demand → $23/TB-mo on capacity at higher tiers | `index`: https://www.definite.app/blog/understanding-snowflake-pricing and https://www.flexera.com/blog/finops/ultimate-snowflake-cost-optimization-guide-reduce-snowflake-costs-pay-as-you-go-pricing-in-snowflake/ |
| Edition mix | Enterprise is "the most commonly deployed edition among mid-market and enterprise buyers" (Vendr); Standard $2.00–2.50/credit, Enterprise $3.00–4.00, Business Critical $4.00–5.00 (Vendr ranges; given range $2–6/credit covers VPS/regions/AI credits) | Vendr `verified live` |

Derived (arithmetic on given numbers, not a source claim): FY26 product revenue $4.472B ÷ ~13,328 customers ≈ **$336k mean product revenue/customer** vs ~$100k median contract — a heavily right-skewed distribution; the mean is ~3.4× the median. That skew *is* the pricing story: a sales-led model rationally chases the tail, and rationally neglects the median-and-below.

**Could not verify:** a true percentile distribution (p25/p75) of Snowflake bills by company size, or % of customers on each edition. Vendr gates percentiles behind its product; Snowflake does not disclose edition mix. See Could Not Verify.

#### 2.2 Price sensitivity — evidence

- **NRR compression:** 125% in FY26 (given) vs 126% FY25, 127% in Q2 FY25, and low-130s earlier (one secondary notes 131%→126% in 12 months). Snowflake called 125% "stable/healthy" in Q4 FY26; the multi-year direction is down ~600–1,000 bps from peak-era retention, consistent with customers optimising consumption rather than expanding unchecked. `index`: https://www.owler.com/reports/snowflake--inc./snowflake-q4-earnings-beat-estimates--revenues-up-/1764930947658
- **Instacart (the canonical named case, SEC-sourced):** S-1 disclosed payments of ~$13M (2020), ~$28M (2021), ~$51M (2022), with ~$15M anticipated for 2023. Snowflake's rebuttal: actual *usage* was $28M in both 2021 and 2022 and $11M in H1 2023 — i.e. payments ≠ usage, and optimisation cut the cost of the same workloads by ~50% in its own worked example. Both readings prove the point for opensnow: even a delighted, expanding customer could halve unit cost once it staffed optimisation. `index`: https://www.theregister.com/on-prem/2023/08/31/snowflake-gets-sensitive-about-instacarts-100m-payments/521146?td=amp-keepreading
- **Predictability complaint at analyst level:** Gartner reported clients find Snowflake spending hard to predict and use FinOps vendors to manage it (via The Register, Aug 2024). VMR survey (weak source): 45% of surveyed users reported budget overruns from consumption pricing, cost-predictability sentiment 6.2/10. `index`: https://klse.i3investor.com/web/blog/detail/future_tech/2024-08-16-story-h-154158499-Warren_Buffett_ditches_his_near_1B_Snowflake_investment and VMR link in §1.2.
- **60-second minimum as a quantified tax:** Snowflake (and Redshift Serverless) bill a 60-second minimum per warehouse activation/query; a 3-second query pays 20× its runtime. For dashboard-heavy BI (8 charts = 8 queries) this is structural, not user error. MotherDuck (per-query, 1 CU-second minimum on Pulse) and Firebolt (per-second, no minimum stated) both market explicitly against it. `index`: https://www.dremio.com/blog/how-to-reduce-amazon-redshift-costs-by-40-60-with-dremios-agentic-lakehouse/ (for Redshift's identical minimum) and MotherDuck pricing `verified live`.
- **Optimisation industry:** a standalone category of Snowflake cost tools (Keebo, Select, Sundeck-type products, FinOps consultancies promising 30–50% reductions; Flexera: "with right-sizing, auto-suspend and query optimisation, teams have documented 30–50% bill reductions") exists only because default consumption is wasteful. `index`: Flexera link above.
- **Cost-driven migrations — named cases (all vendor/consultancy-published, flag accordingly):**
  - *Definite* — Snowflake → self-hosted DuckDB, **>70% warehouse cost reduction**, modelled 55–77% cheaper on equivalent VMs (via MotherDuck guide). `index`: https://motherduck.com/learn/reduce-cloud-data-warehouse-costs-duckdb-motherduck/
  - *Appcues* — Snowflake+Airflow → ClickHouse Cloud: p95 latency >20s → sub-second serving for customer-facing dashboards, retired Airflow rollups, lower spend (no % disclosed). *Buildkite* — →ClickHouse Cloud: net infra spend down "five figures per month" while ingestion quadrupled. Both via ClickHouse CostBench series. `index`: https://clickhouse.com/blog/why-clickhouse-cloud-wins-at-real-time-analytics
  - *Unnamed "world's largest ad tech platform"* — Snowflake → ClickHouse via ChistaDATA, billions of impressions/day, PB scale; driver stated as "limitations of Snowflake's pricing model, query performance and scalability"; no % disclosed. `index`: https://chistadata.com/from-snowflake-to-clickhouse-how-chistadata-enabled-the-worlds-largest-ad-tech-platforms-migration-and-built-an-optimal-real-time-analytics-infrastructure/
  - List-price modelling (independent consultant, 2023 prices): ClickHouse Cloud 40–55% cheaper than Snowflake across 3 modelled workloads. `index`: https://newcadence.io/insights/comparing-clickhouse-snowflake-pricing
  - **Counterweight:** no credible public evidence of mass enterprise *churn* off Snowflake — NRR 125% and +2,332 net new customers in FY26 mean pain expresses as optimisation, workload-splitting (real-time/serving workloads to ClickHouse, small workloads to DuckDB) and renewal leverage, not wholesale exit. Opensnow's wedge is workload-by-workload, not rip-and-replace.

#### 2.3 The underserved bottom

- What the bottom uses instead: Postgres (+ extensions), DuckDB local, BigQuery on-demand (free 1 TiB/mo does real work at small scale), SQLite, MotherDuck free tier, ClickHouse OSS self-hosted. The common trait: **$0 to start, no sales call, no $25k commitment.**
- MotherDuck's stated target segment (Wing VC profile): "application developers building data-intensive B2B applications" and small-data teams underserved by warehouses built for petabytes; PLG-first, design-partner-led. Early traction at Series B (2023): ~2,000 users at beta-end Sep 2023 → 3,000+ a month later. No current customer/revenue disclosure found — see Could Not Verify. `index`: https://www.wing.vc/content/a-closer-look-at-motherducks-unique-monetization-of-duckdb
- DuckDB adoption (primary, `verified live` for Oct 2025 post): **almost 25M PyPI downloads/month**, Stack Overflow usage **1.4% (2024) → 3.3% (2025)**, top-4 most admired DB, est. 20+ Fortune-100 users. `verified live`: https://duckdb.org/2025/10/09/benchmark-results-14-lts — Aug 2026 update: **50M+ PyPI/month, >1M downloads/day, 40,000 GitHub stars, 8M unique visitors/month to duckdb.org, 2 PB/month extension-download traffic**. `index`: https://duckdb.org/2026/08/05/github-40k-stars
- ClickHouse OSS adoption: "hundreds of millions of downloads over the past decade" (company, 2026); ~39.5–47.6k GitHub stars depending on snapshot date/source; users named by ClickHouse include Meta, Tesla, Sony, Cursor, Cloudflare, Uber, Spotify, GitLab. `index`: https://clickhouse.com/blog/introducing-house-mates and https://www.open-source-tools.com/clickhouse

#### 2.4 AI / agent workloads as new spend

| Signal | Number | Source / flag |
|---|---|---|
| Snowflake AI revenue | **$100M run-rate reached Q3 FY26, a quarter early**; AI influenced ~50% of Q3 bookings; 7,300 customers using AI/ML weekly; Snowflake Intelligence 1,200 accounts (Q3) → 2,500 (Q4) | Company via Zacks, `index`: https://www.zacks.com/stock/news/2810471/snowflake-rides-on-growing-customer-base-more-upside-ahead |
| Snowflake AI pricing | AI Credits at **$2.00 each** (global routing) / $2.20 regional, separate from platform credits, capacity discounts don't apply (Keebo/Definite, Apr 2026 schedule) | `index`: https://keebo.ai/blog/snowflake-pricing/ |
| Databricks AI revenue | $1B run-rate Sep 2025 → **$1.4B Feb 2026** (~26% of total run-rate) | Company via SaaStr, `index` §1.2 |
| Agents vs humans | Agents now send **more queries than people** across Databricks and launch **>17M databases/day**; 3 years ago all interactions were human | a16z essay (a16z is a Databricks investor/board — interested source), via `index`: https://theinference.org/article/agents-launch-17-million-databases-a-day-on-databricks-the-line-they-feed-makes-100-million |
| Agent-created databases (Neon) | **~80% of Neon databases created by AI agents** (up from 30% at GA; also stated as >80% of databases provisioned on Neon created automatically by agents) | Databricks/Neon via secondaries, `index`: https://techsy.io/en/blog/neon-vs-planetscale-vs-turso |
| Lakebase (agent/OLTP line) | Crossed **$100M run-rate** by Aug 2026 reporting — i.e. ~1.4% of Databricks' $7B; usage ≫ revenue so far | `index`: theinference.org link above |
| Implication | Agent workloads are high-query-count, bursty, latency-sensitive, and created programmatically — they punish 60-second minimums, cold starts >seconds, and per-seat pricing, and reward per-second/scale-to-zero/per-query pricing and instant provisioning | Synthesis of above |

### 3. Competitor Business Models & Pricing (current, checked)

#### 3.1 Pricing table — one row per competitor

| Product | Model | Entry / free | Headline rates (US, list) | Flag / source |
|---|---|---|---|---|
| **MotherDuck** | Per-second compute (Pulse per-query, 1 CU-sec min) + storage + org platform fee | **Lite FREE forever: 10 GB storage, 10 Pulse hrs/mo, 3 internal users, 2 service accounts, no card** | Business **$250/org/mo + usage** (≤10 internal users, 99.9% SLA); storage **$0.04/GB-mo (= $40/TB-mo)**; Pulse $0.60/CU-hr, Standard $2.40/hr, Jumbo $4.80, Mega $12, Giga $24 (later docs show Giga to $36 — price moving up at top end); AI $1.00/AI Unit; Enterprise custom (PrivateLink, HIPAA, BYOC, fixed capacity) | `verified live`: https://motherduck.com/product/pricing/ |
| **ClickHouse Cloud** | Compute + storage metered separately, per-minute, scale-to-zero (Scale/Ent) | **No permanent free tier. $300 credits / 30 days** | Compute **$0.2181 (Basic) / $0.2985 (Scale) / $0.3903 (Enterprise) per unit-hr** (unit = 8 GiB RAM + 2 vCPU); storage **$25.30/compressed TB-mo** all tiers; egress from $0.1152/GB public; worked examples: Basic ~$66.52/mo at 6h/day, $186.27 at 24/7 (single 8 GiB replica); Scale from ~$499/mo | Page `verified live`: https://clickhouse.com/pricing (rates themselves in JS calculator, not in fetched text); rates via teardown citing clickhouse.com 8 Aug 2026, `index`: https://selfhost.dev/blog/clickhouse-pricing-real-cost-of-running-it/ |
| **Databricks SQL** | DBU, per-second; Classic/Pro = DBU + your cloud VM bill; Serverless = DBU incl. infra | No permanent free tier (trial only) | SQL Classic **$0.22/DBU**, Pro **$0.55/DBU**, **Serverless $0.70/DBU US (~$0.91 EU)**; Serverless 2X-Small warehouse = 4 DBU/hr = $2.80/hr; Standard tier sunset on AWS/GCP Oct 2025 | `index`: https://www.flexera.com/blog/finops/databricks-pricing-guide/ and https://www.owox.com/blog/articles/databricks-pricing |
| **BigQuery** | (a) On-demand per byte scanned; (b) Editions capacity per slot-hour | **1 TiB queries + 10 GB storage free every month, forever** (also Sandbox without billing account) | On-demand **$6.25/TiB scanned** (US, since Jul 2023); Editions pay-as-you-go: **Standard $0.04, Enterprise $0.06, Enterprise Plus $0.10 per slot-hour**, 1-/3-yr commits discount (Ent/Ent+ up to ~40%); storage $0.02/GB-mo active ($20/TB), $0.01 long-term; break-even ≈156 slot-hrs per TiB on Standard | `index`: https://clickhouse.com/blog/how-cloud-data-warehouses-bill-you and https://tecyfy.com/blog/bigquery-on-demand-vs-editions |
| **Firebolt** | Per-second engines, Firebolt Units (FBU) + pass-through storage | $200 trial credit cited in reviews; **Firebolt Core = free self-hosted engine**; no permanent hosted free tier found | **Standard $0.23/FBU-hr, Enterprise $0.35/FBU-hr (US)**; non-US $0.28/$0.42; compute-optimised S node (8 vCPU/16 GB) = 4 FBU = **$0.92/hr Standard**; storage-optimised S = $1.84/hr; storage ~$27/TiB-mo (S3 pass-through, ~no markup); default CREATE ENGINE trap: 2× M storage nodes = $7.36/hr | Docs `index`: https://firebolt-ailea-billing-payment-methods.mintlify.app/overview/billing and review `index`: https://panoply.io/tools/firebolt/ |
| **Redshift Serverless** | RPU-hours, per-second, **60-sec minimum** | $300 trial credit / 90 days (per reviews) | **$0.375/RPU-hr** US-East; min base 4 RPU = **$1.50/hr active**; 1 RPU = 16 GB RAM; managed storage $0.024/GB-mo; provisioned RA3 from ~$1.086/node-hr (xlplus) | `index`: https://www.cloudzero.com/blog/redshift-pricing/ |
| **Snowflake** (for comparison, given + Vendr) | Credits, per-second after **60-sec minimum** + storage + cloud services | $400 trial credits, **no permanent free tier** | $2 Standard / $3 Enterprise / $4 Business Critical per credit (US on-demand anchor; to $6 VPS/regions); storage $40/TB-mo on-demand, $23 capacity | Given + Vendr `verified live` |
| **Neon** (Postgres analytics-adjacent / agent DB) | CU-hours + storage, scale-to-zero after ~5 min, no minimum on paid | **Free: 100 CU-hrs + 0.5 GB per project × 100 projects** (later reporting: storage doubled to 1 GB/project Oct 2026) | Launch **$0.106/CU-hr**, Scale **$0.222/CU-hr** (1 CU ≈ 1 vCPU + 4 GB); storage **$0.35/GB-mo** (cut from $1.75 post-Databricks acquisition); ~$5/mo minimum in some summaries | `index`: https://draxlr.com/blogs/supabase-vs-neon/ |
| **DuckDB (OSS)** | Free, MIT, in-process | Free forever | $0 — you pay only your own compute/storage. Monetisation is *around* it, not *for* it (see §3.2) | DuckDB Foundation model, §5 |

**Pricing-shape observations:** (1) Every challenger except ClickHouse Cloud and Firebolt-hosted now offers a permanent free tier; (2) every challenger markets *billing granularity* (per-second / per-query / no minimum) as a headline feature against Snowflake's 60-second minimum; (3) storage list prices cluster $20–40/TB-mo — nobody wins on storage, they win on compute metering and idle cost ($0 at scale-to-zero); (4) platform fees are returning at the bottom end (MotherDuck $250/org/mo for production) — pure usage with no floor is *not* the norm once a customer is in production.

#### 3.2 Open-source-first business models that worked in data infra

| Company | OSS core | What the paid layer actually is | Revenue / funding anchor | Flag |
|---|---|---|---|---|
| ClickHouse Inc | ClickHouse, Apache 2.0, self-host free | **Hosted only at scale:** ClickHouse Cloud (managed, scale-to-zero, governance, support). No licence fee for OSS | >$350M run-rate Sep 2026; >$1B total raised ($50M A + $250M B at $2B, 2021; $350M C May 2025 at $6.35B; $400M D Jan 2026 at **$15B**); ~800 employees, target 1,000 | `index`: https://solanacompass.com/projects/clickhouse and TechCrunch §1.2 |
| DuckLabs (ex-DuckDB Labs) | DuckDB/DuckLake/Quack, MIT, IP held by non-profit DuckDB Foundation | **Support + feature prioritisation contracts only.** No hosted product, no licence, no VC — "fully owned by the team" | >30 employees by May 2026, bootstrapped 2021–2026 on contracts "from the biggest tech companies to small startups"; **acquired by AWS Aug 2026** (terms undisclosed, team stays in Amsterdam, OSS stays MIT/Foundation) | `index`: https://ducklabs.com/news/2026/05/27/duckdb-labs-becomes-ducklabs and https://duckdb.org/history/ |
| MotherDuck | DuckDB (MIT, not its own OSS) | **Hosted-only:** serverless cloud, sharing/collaboration, hypertenancy, governance, SLA — the DuckLabs partnership gives it core-team access; DuckLabs contracted for engineering | $100M raised (§5); revenue not disclosed | §5 |
| dbt Labs | dbt Core, Apache 2.0 | **Hosted platform:** dbt Cloud/Platform (orchestration, IDE, governance, semantic layer) at ~$100/user/mo list for Cloud entry per reviews; note 2025 shift: new Fusion engine under Elastic Licence 2.0 (source-available, *not* OSS) after conversion difficulty was reported anecdotally | **>$100M ARR Feb 2025** (from $2M four years earlier), >5,400 Cloud customers, 50,000+ teams use dbt weekly (later job posts: 90,000); $414–416M raised, $4.2B valuation Series D Feb 2022; **merger with Fivetran announced Oct 2025** (combined ~$600M ARR, >10,000 customers per secondary reporting) | `index`: https://www.upmarket.co/private-markets/pre-ipo/dbt-labs/ and https://www.integrate.io/blog/dbt-review/ |
| Confluent | Apache Kafka, Apache 2.0 | **Hosted + enterprise platform:** Confluent Cloud (56% of subscription revenue Q3 2025) + Confluent Platform (self-managed enterprise); connectors, Schema Registry, governance, Flink | FY25 guidance ~$1.11B subscription revenue; Q3 2025: subscription $286.3M, Cloud $161M; customers >$100k/yr: 1,521 (2025, via secondary); **IBM acquisition agreed Dec 2025 at ~$11B EV ($31/share)** | `index`: https://www.mitrade.com/au/insights/news/live-news/article-8-1226299-20251028 and https://www.strategycanon.com/casestudy/confluent.html |
| Airbyte | Airbyte OSS, ELv2-ish/open-source connectors (600+; community-built) | **Hosted (Airbyte Cloud, credit/usage pricing) + Self-Managed Enterprise** (SSO, governance, support) — Enterprise hit $1M ARR in 4 months after late-2023 launch per founder interview; Cloud launched early 2023 | **$181–185M raised**, incl. $150M Series B Dec 2021 at unicorn valuation, *before* a revenue model was cracked (founder's own account); 25,000–40,000 companies using (sources disagree, company vs aggregator); ARR estimates $85M (one canvas site, unreliable) to $120M+ (aggregator, unreliable) — **treat ARR as unverified** | `index`: https://bouncewatch.com/company/airbyte and https://www.pmf.show/blog/michel-tricot-airbyte-185m-funding-product-market-fit |
| Neon (Postgres comp) | Neon OSS (Apache 2.0 storage engine) | Hosted serverless Postgres (§3.1) | ~$130M VC pre-exit; **acquired by Databricks May 2025 for ~$1B**; ~$25M ARR at acquisition (Sacra estimate); 18,000+ customers per Databricks | `index`: https://chierhu.medium.com/the-backend-as-a-service-and-managed-serverless-database-landscape-eaa16bccd44e |

**Pattern:** In every durable case the paid layer is *operations + governance + SLA*, never the engine licence. The two purest counter-examples cut opposite ways: DuckLabs proved support-only sustains ~30 people but not a cloud GTM (it sold to AWS); Airbyte proved community can raise $185M *before* monetisation but then monetisation becomes the existential question. dbt's Fusion licence move is the live warning: when OSS→paid conversion is too weak, the vendor eventually fences the new engine.

### 4. Distribution & Go-To-Market in Data Infrastructure

#### 4.1 How the challengers actually grew

- **DuckDB:** zero-marketing, package-manager distribution. PyPI is the channel: 20M+/mo (Jun 2025) → ~25M (Oct 2025) → 50M+ (Aug 2026); website 3M unique visitors/mo (Jun 2025) → 8M (Aug 2026). Growth tracked Stack Overflow usage doubling in a year. No sales force at DuckLabs at all.
- **ClickHouse:** OSS since 2016 (Yandex), company only from 2021; Cloud GA 2023 reached 3,000 customers by Jan 2026 / 4,000 by mid-2026, with named AI-native logos (Cursor, Lovable, Polymarket, Decagon) doing the marketing; 200+ ecosystem integrations and a formal partner programme (House Mates, 60+ partners) launched only in 2026 — community first, channel last.
- **dbt:** community/Slack + weekly-active-teams metric as the top of funnel: 50,000+ teams weekly vs 5,400 paying Cloud customers — an implied **org-level conversion of roughly 1-in-10 weekly-active teams at best** (teams ≠ orgs; directional only, arithmetic on company figures above).
- **Confluent:** creator-led (Kafka's authors), meetup community (60,000+ members, 200+ groups by 2021; Kafka in >70% of Fortune 500), free trial → pilot → platform land-and-expand; partner-sourced deals >25% of new business by Q3 2025.
- **MotherDuck:** PLG + design partners explicitly ahead of revenue ("at this stage our goal is happy customers, not revenue" — Wing profile of Series-B era); now adding sales-led (AE job posts, AWS Marketplace private offers for Enterprise).

#### 4.2 Open-source → paid conversion benchmarks (weak-source territory — flag in any deck)

No audited, category-specific conversion rate is published. The ranges in circulation, all from investor/consultancy blogs citing Bessemer/OpenView/Battery/Boldstart:

- Broad developer tools: **2–5%** free→paid; specialised infra: 5–10%; enterprise-focused: up to 15–20% (Bessemer via Monetizely). `index`: https://www.getmonetizely.com/articles/whats-the-relationship-between-open-source-adoption-and-saas-revenue
- OpenView-cited: successful OSS companies **1–5%**, top performers 8–10% in targeted enterprise segments; HashiCorp ~1.5% of OSS users → commercial pre-IPO; Elastic ~1%; Confluent <1% of community. `index`: https://www.getmonetizely.com/articles/how-to-calculate-open-source-to-commercial-conversion-a-guide-for-saas-executives
- Boldstart via QC Growth: median devtool free→paid **5% within 6 months**, 54% of conversions in first 3 months, 85% within a year; new ARR split ~40% sales-led / 22% sales-assisted / 21% self-serve. `index`: https://www.qcgrowth.com/blog/gtm-for-devtools-at-seed

**Planning implication:** model opensnow at **1–3% of active OSS/self-serve orgs converting** (the infra range with actual named precedents), not the 5–10% blog range. At DuckDB-scale top-of-funnel even 1% is a large business; at 1/100th of DuckDB's funnel it is not — funnel scale, not conversion optimisation, is the binding constraint early.

#### 4.3 Snowflake's sales-led cost — the inefficiency bottom-up attacks

FY26 (filing, §5 for source): GAAP S&M $2,062M = **44.0% of revenue**; FY25: $1,672M = **46.1%**. Non-GAAP S&M: ~34–35% of revenue in H1 FY26, 32% in Q2 FY27 (company releases) — the GAAP/non-GAAP gap is mostly stock comp. Cross-check via SaaS benchmark synthesis: Snowflake 44.0% vs prior 46.1%, public-SaaS median S&M ~39%. `index`: https://vidico.com/news/how-much-do-saas-companies-spend-on-marketing/
Marketplace as alternative distribution: Snowflake itself surpassed **$2B in cumulative AWS Marketplace sales, doubling YoY** (Q3 FY26) — proof that cloud-marketplace procurement (spend committed cloud dollars, skip vendor onboarding) is now a primary enterprise channel, and one a challenger can also list on (MotherDuck Enterprise and ClickHouse Cloud both sell via marketplaces). `index`: Zacks Q3 link in §2.4.

### 5. Build Cost Reality & Funding

#### 5.1 What it cost others to reach a warehouse product

| Company | Raised | For what | Outcome anchor |
|---|---|---|---|
| Firebolt | **$269M total** ($37M to Dec 2020 launch; $127M Series B Jun 2021; $100M Series C Jan 2022 at **$1.4B**) | Building a *new engine + cloud warehouse from scratch* | No new round since Jan 2022; Feb 2026: cut "dozens" of staff despite >$100M cash on hand; revenue estimates ~$26.6M (aggregator, unverified). **The cautionary tale: $269M and a unicorn valuation did not buy distribution against Snowflake/Databricks** |
| MotherDuck | **$100M total** ($12.5M seed + $35M Series A, both 2022; **$52.5M Series B Sep 2023 at $400M post**) — some trackers show a further $33M "Series B+" May 2025 taking total to ~$133M at ~$496M; **disagreement flagged, PitchBook/Forge vs TechCrunch-era $100M** | Hosted layer only — engine (DuckDB) already existed and was MIT | CEO at Series B: "it takes $10M to build a database, another $10M to build a database as a service" (paraphrased in transcript); team 51–200 (PitchBook range); revenue undisclosed |
| ClickHouse Inc | **>$1B total** (see §3.2) | Commercialising an existing OSS engine (built at Yandex over a decade) + building Cloud | >$350M run-rate — the best capital-to-revenue outcome in the challenger set, on an engine someone else funded for 10+ years first |
| Bauplan | **$7.5M seed** (Apr 2025, Innovation Endeavors) | Serverless lakehouse *on Iceberg + existing engines*, Python-first, small team (NY/SF) | >200,000 jobs/week by end-2025 with enterprise design partners (MFE-MediaForEurope) — proof a credible lakehouse *product* (not engine) can launch at seed scale |
| DuckLabs | **$0 VC** | The engine itself (DuckDB) + DuckLake + Quack | ~30 people, support-contract revenue, acquired by AWS 2026 — proof an engine team can be tiny *if* it does no cloud ops, no sales, no support-at-scale |
| Onehouse (adjacent) | $68M total | Managed lakehouse (Hudi creator) on existing formats | `index`: https://insights.tryspecter.com/the-ai-value-chain-2026/ |

Sources: Firebolt `index`: https://en.globes.co.il/en/article-firebolt-raises-100m-becomes-latest-israeli-unicorn-1001400129 and https://forgeglobal.com/firebolt_stock/ ; MotherDuck `index`: https://www.wing.vc/content/a-closer-look-at-motherducks-unique-monetization-of-duckdb and https://forgeglobal.com/motherduck_stock/ ; Bauplan `index`: https://siliconangle.com/2025/04/16/bauplan-raises-7-5m-automate-data-infrastructure-code-ai-software-developers/

#### 5.2 What competing with Snowflake costs the incumbent (filing numbers)

Snowflake Q4/FY26 release (25 Feb 2026), full-year FY26 vs FY25, `index`: https://files.quartr.com/reports/3cfa8-2026-02-25-09-05-09.pdf?ref=U3RvY2tBbmFseXNpcw%3D%3D

| Line | FY26 | FY25 | % of FY26 revenue |
|---|---|---|---|
| Total revenue | $4,683.9M | $3,626.4M | 100% |
| Cost of revenue | $1,537.8M | $1,214.7M | 32.8% (non-GAAP product gross margin 75.8%) |
| **Sales & marketing (GAAP)** | **$2,062.1M** | $1,672.1M | **44.0%** (FY25: 46.1%) |
| **Research & development (GAAP)** | **$1,969.5M** | $1,783.4M | **42.0%** (FY25: 49.2%) |
| General & administrative (GAAP) | $549.7M | $412.3M | 11.7% |
| Stock-based comp | ~34% of revenue (~$1.6B) | 41% of revenue | guided to 27% in FY27 |

Non-GAAP view (Q2 FY27 release): S&M 32%, R&D 20%, G&A 5% of revenue. The ~12–22-point gap between GAAP and non-GAAP on S&M/R&D *is* the stock comp. Any "Snowflake spends X% on sales" claim must state which basis. Headcount ~9,060 (secondary). RPO $9.77B at FY26-end, +42%.

**Read for opensnow:** Snowflake spends ~$4.0B/yr GAAP on S&M+R&D combined to defend $4.7B of revenue. A challenger cannot out-spend or out-feature it; the only viable asymmetry is (a) an engine it didn't pay to build (OSS), (b) a distribution channel that costs ~$0 per user (package managers / OSS funnel), and (c) a billing model whose *shape* (per-second, no minimum, scale-to-zero) is itself the marketing.

#### 5.3 Solo / small-team precedents

- **DuckLabs — the strongest precedent:** ~30 people built and sustained the world's fastest-growing analytical engine, reached AWS acquisition, with no VC and no hosted product. But note what it *didn't* do: cloud operations, enterprise sales, 24×7 support, compliance. MotherDuck exists precisely to do those, and needed $100M to start.
- **Bauplan:** 3 founders + small team, $7.5M, production lakehouse with enterprise customers within a year — by building on Iceberg and existing engines, not a new engine.
- **Definite (as a user, not a vendor):** a small SaaS team replaced Snowflake with self-hosted DuckDB entirely — evidence the *workload* at small scale needs no warehouse vendor at all, which is both opensnow's opportunity (be the hosted version of that) and its threat (the OSS alone is good enough for many).
- **No precedent found** for a solo/2–5-person team reaching material *hosted warehouse revenue* (>~$10M ARR). Every hosted warehouse that reached revenue had ≥$7.5M seed and a team in the dozens, or an acquired/funded engine team behind it. See Could Not Verify.

### 6. Implications for opensnow Pricing

Derived from the comps above — this section is analysis, not sourced fact.

**What "freer" must mean to be credible (free-tier norms):**
- Permanent free tier, no credit card: **≥10 GB storage + ≥10 compute-hours/month** (MotherDuck parity) — BigQuery's 1 TiB scanned/mo is the alternative shape if pricing per byte. Anything less, or trial-only (ClickHouse/Firebolt shape), reads as enterprise-sales-in-disguise to the DuckDB-native audience.
- The OSS engine itself stays free and self-hostable with no licence fence. dbt's Fusion ELv2 move and Airbyte's monetisation struggle show the community punishes late fencing more than early clarity: state on day one that the paid layer is *hosted operations, governance, collaboration, SLA and support* — the ClickHouse/Confluent/dbt-Cloud pattern that demonstrably pays.

**What "cheaper" can charge (comp-anchored bands):**

| Meter | Market band (from §3.1) | opensnow positioning |
|---|---|---|
| Compute, small/bursty | MotherDuck Pulse $0.60/CU-hr; Neon $0.106/CU-hr (OLTP-sized); Redshift min $1.50/hr active; Snowflake XS warehouse $2–4/hr with 60-sec min | Per-second, **1-second (or per-query) minimum** is the headline, not the rate. A Pulse-equivalent at $0.40–0.60/hr with no minimum undercuts Snowflake's *effective* small-query cost by ~10–20× without undercutting headline rates unsustainably |
| Compute, standard warehouse | MotherDuck Standard $2.40/hr; Databricks Serverless SQL from $2.80/hr (2XS); Firebolt S $0.92/hr (compute-opt) | $1.50–2.50/hr per-second for the everyday tier keeps gross margin plausible (Snowflake sustains ~76% non-GAAP product gross margin at $3/credit; DuckDB-class single-node efficiency is the margin source, per vendor benchmark claims — not independently verified here) |
| Per-byte alternative | BigQuery $6.25/TiB scanned | If offered at all, **$2–3/TiB** would be a clear "half of BigQuery" claim — but per-byte punishes exploratory/agent querying (the growth workload, §2.4). Prefer per-second compute as the primary meter; per-byte only if the engine makes scan cost near-zero marginal |
| Storage | $20–40/TB-mo across all vendors; object-storage pass-through ~$23/TB (S3) | Charge at/near pass-through ($0.023–0.04/GB-mo) or bundle on Iceberg-in-customer-bucket at $0 storage margin — BYO-bucket storage is a *lock-in-removal* feature, and storage margin is not where this category is won |
| Platform / seat fee | MotherDuck $250/org/mo for production (≤10 users); dbt Cloud ~$100/user/mo; Supabase $25/mo | **Org fee, not per-seat, for the warehouse itself:** $0 free → ~$25–50/org/mo "Team" (the Neon/Supabase entry psychology) → ~$250/org/mo "Business/Production" (MotherDuck parity: SLA, SSO/SCIM, custom roles, 90-day snapshots). Per-seat pricing taxes exactly the broad readership a warehouse wants; cap *internal active users* by tier (MotherDuck: 3 / 10 / unlimited) instead of metering viewers/end-users (MotherDuck: end users unlimited on all tiers) |
| Enterprise | All vendors: custom, PrivateLink, HIPAA, BYOC, fixed-capacity | Custom from ~$25–50k/yr — deliberately *at* Snowflake's capacity minimum, so a Snowflake customer can switch its minimum commitment 1-for-1 and get the whole workload, not a slice |
| AI / agent usage | Snowflake AI Credits $2/unit, discounts excluded; MotherDuck $1/AI Unit | Meter agents as compute (per-second), not as a separate credit currency. A separate AI-credit SKU is a margin-protection move by an incumbent; a challenger's move is "agents pay the same per-second rate as humans, with 100ms spin-up and scale-to-zero" |

**Model choice — seat vs usage:** Usage-primary + org platform fee (MotherDuck shape) is the only model with a working comp at opensnow's exact position. Pure seat pricing fails for agent/embedded workloads (non-human query volume); pure usage with no floor fails unit economics at the bottom (MotherDuck added the $250 floor for production; Neon has a ~$5 minimum). Free → usage → org-fee-for-production → custom enterprise is the category's convergent ladder.

**The number to beat in the pitch:** median Snowflake contract ~$100k/yr with a $25k entry minimum and a 60-second billing floor. An opensnow that serves the median workload for **$10–30k/yr all-in, starts at $0, and bills by the second** is not claiming a discount — it is claiming the segment Snowflake's own distribution (44% of revenue spent on sales) cannot afford to serve.

---

## Could not verify

1. **Percentile distribution of Snowflake bills by company size / edition mix (% Standard vs Enterprise vs Business Critical).** Vendr gates percentiles; Snowflake does not disclose edition mix. Only segment bands (§2.1) and "Enterprise is most common among mid-market/enterprise" were verifiable.
2. **Deduplicated count of cloud-warehouse users by SMB/mid-market/enterprise across all vendors.** No authoritative source exists; technographic counts (6sense: 34,708 for Snowflake alone) conflict with vendor customer counts (~13,328) by method.
3. **MotherDuck current customers / revenue / ARR.** Not publicly disclosed as of this run; only 2023 user counts (~3,000) and funding. Forge/PitchBook also conflict on total raised ($100M vs ~$133M incl. a claimed $33M 2025 Series B+).
4. **Airbyte ARR.** Aggregator estimates ranged $85M–$120M+ with no primary source; only funding ($181–185M) and the founder's $1M-ARR-in-4-months Enterprise claim are sourced.
5. **BigQuery and Redshift product revenue / true market share.** Neither hyperscaler breaks it out; all such figures in mill reports are modelled.
6. **A solo/small-team (≤5 people) hosted warehouse reaching >$10M ARR.** No example found; smallest credible precedents are DuckLabs (~30 people, no hosted product) and Bauplan (seed-funded team, lakehouse product).
7. **Independent (non-vendor) price-performance benchmarks** for MotherDuck/ClickHouse vs Snowflake at small scale. CostBench is ClickHouse's own; MotherDuck's cost guides are its own. Direction is consistent across sources; magnitudes (70–90% savings, 412× perf/$) are vendor claims.
8. **IDC's current DBMS/cloud-data-platform forecast** in primary form (paywalled; only older IDC workload forecasts and second-hand DataSphere figures were accessible). Gartner's full vendor share table is likewise paywalled — only the $119.7B / 64% cloud abstract is primary-verified.
9. **Snowflake churn rate** (logo or gross revenue churn). Not disclosed; NRR 125% is net of churn but cannot be decomposed publicly.

## Sources

Verified live (opened 2026-11-05):
- Gartner, Market Share: DBMS Worldwide 2024 (abstract) — https://www.gartner.com/en/documents/6494271
- MotherDuck pricing — https://motherduck.com/product/pricing/
- ClickHouse Cloud pricing — https://clickhouse.com/pricing
- Vendr, Snowflake pricing & benchmarks — https://www.vendr.com/marketplace/snowflake
- DuckDB, Adoption Metrics for v1.4 LTS (Oct 2025) — https://duckdb.org/2025/10/09/benchmark-results-14-lts

Index (search results read 2026-11-05, not opened live) — key sources; full URLs in body next to each fact:
- Snowflake FY26 results PDF via Quartr — https://files.quartr.com/reports/3cfa8-2026-02-25-09-05-09.pdf?ref=U3RvY2tBbmFseXNpcw%3D%3D
- Snowflake Q4 FY25 investor presentation (TAM) — https://investors.snowflake.com/files/doc_financials/2025/q4/Q4-FY2025-Investor-Presentation_vFF.pdf
- Databricks run-rate series — https://www.saastr.com/growth-may-have-slowed-for-some-but-holy-cow-not-for-databricks/ ; https://databricks.techfabric.com/guide/databricks-by-the-numbers/ ; https://okara.ai/blog/databricks-revenue-growth
- Market-size mills & syntheses — https://www.emergenresearch.com/industry-report/data-warehousing-market ; https://www.marketresearchfuture.com/reports/data-warehouse-as-a-service-market/companies ; https://dataintelo.com/report/global-data-warehouse-software-market ; https://blog.9cv9.com/top-100-data-warehouse-software-statistics-data-trends-in-2026/ ; https://www.theinsightpartners.com/reports/data-lakehouse-market ; https://pitchgrade.com/research/snowflake-vs-databricks
- Spend/pain — https://costbench.com:443/compare/google-bigquery-vs-snowflake/ ; https://www.theregister.com/on-prem/2023/08/31/snowflake-gets-sensitive-about-instacarts-100m-payments/521146?td=amp-keepreading ; https://www.definite.app/blog/understanding-snowflake-pricing ; https://motherduck.com/learn/reduce-cloud-data-warehouse-costs-duckdb-motherduck/ ; https://clickhouse.com/blog/why-clickhouse-cloud-wins-at-real-time-analytics
- Competitor pricing — https://selfhost.dev/blog/clickhouse-pricing-real-cost-of-running-it/ ; https://www.flexera.com/blog/finops/databricks-pricing-guide/ ; https://clickhouse.com/blog/how-cloud-data-warehouses-bill-you ; https://tecyfy.com/blog/bigquery-on-demand-vs-editions ; https://firebolt-ailea-billing-payment-methods.mintlify.app/overview/billing ; https://www.cloudzero.com/blog/redshift-pricing/ ; https://draxlr.com/blogs/supabase-vs-neon/
- OSS business models / GTM — https://ducklabs.com/news/2026/05/27/duckdb-labs-becomes-ducklabs ; https://www.upmarket.co/private-markets/pre-ipo/dbt-labs/ ; https://www.mitrade.com/au/insights/news/live-news/article-8-1226299-20251028 ; https://bouncewatch.com/company/airbyte ; https://www.getmonetizely.com/articles/how-to-calculate-open-source-to-commercial-conversion-a-guide-for-saas-executives ; https://www.qcgrowth.com/blog/gtm-for-devtools-at-seed
- Funding / build cost — https://en.globes.co.il/en/article-firebolt-raises-100m-becomes-latest-israeli-unicorn-1001400129 ; https://www.wing.vc/content/a-closer-look-at-motherducks-unique-monetization-of-duckdb ; https://siliconangle.com/2025/04/16/bauplan-raises-7-5m-automate-data-infrastructure-code-ai-software-developers/ ; https://techcrunch.com/2026/01/16/snowflake-databricks-challenger-clickhouse-hits-15b-valuation/
- AI/agents — https://www.zacks.com/stock/news/2810471/snowflake-rides-on-growing-customer-base-more-upside-ahead ; https://theinference.org/article/agents-launch-17-million-databases-a-day-on-databricks-the-line-they-feed-makes-100-million ; https://keebo.ai/blog/snowflake-pricing/

*No page encountered instructed this research to do anything outside the mission; no such instructions were followed or needed to be flagged beyond ordinary vendor marketing claims, which are flagged as vendor claims in the body.*
