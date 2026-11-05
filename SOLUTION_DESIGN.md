# opensnow: Solution Design

> **Living document.** This is the project's source of truth for vision, architecture, and technical decisions. It changes as we learn.
>
> - **Status:** Approved Nov 5, 2026. Tom approved the D-01 to D-06 recommendations; the decision queue in section 9 and docs/DECISIONS.md records the rulings.
> - **Last updated:** 2026-11-05
> - **Name:** opensnow (codename, see D-06)

## Changelog

| Date | Change |
|---|---|
| 2026-11-05 | First draft. Vision, wedge scope, layered architecture, dialect position, experience and agent layers, pricing, phasing tied to Gate 1 and Gate 2, decision queue restated as open. |
| 2026-11-05 | Tom approved the D-01 to D-06 recommendations. Decision queue marked DECIDED; DuckLake vs Iceberg evaluation queued as a D-01 sub-study for the Phase 0 storage spike. |
| 2026-11-05 | Operating model set by Tom: built by Tom plus the Muse and Codex loop (as with OpenSight and freeDMS), first cloud AWS, no design partner. Gate 1 re-based on reference workloads we build, priced on published Snowflake rates against our real AWS costs. |

---

## 1. Vision and principles

opensnow is a better, faster, freer version of Snowflake.

Snowflake proved the shape. It separated storage from compute. It deleted the DBA job. It turned time travel, cloning, and sharing into metadata operations. People love it for real reasons, and the research in docs/research/01 lists them: one copy of the data with many independent computers on top, zero tuning, live sharing with no copies, VARIANT plus plain SQL, one product that works the same on three clouds.

Snowflake also got expensive and complicated, and it stayed closed. Cost is the top complaint in every review source the research checked. opensnow keeps the shape and fixes the price, the simplicity, and the openness.

Four principles govern everything below. Tom set them on Nov 5, 2026. They outrank any recommendation in this document.

1. **AI-first, agent-attractive from day one.** Modeled on DuckDB and Supabase. Agents searching for and building with the product are a primary user: machine-readable docs, an MCP server, programmatic provisioning, an instant free start, and SQL an agent can generate reliably.
2. **Win on cost by cutting margin, not by changing the meter.** Snowflake customers are reeling from cost. Keep Snowflake's familiar usage-based pricing model (usage, not per-user) and charge less for the same usage by accepting lower gross margins. Granularity fixes such as the 60-second minimum are secondary.
3. **Keep what makes Snowflake successful.** Simple data sharing, no pricing by user, familiar syntax, trusted performance, zero-tuning elasticity.
4. **Engine plus experience.** A powerful, fast, trustworthy OLAP engine is table stakes. So is the layer on top of it: easy data navigation, SQL worksheets, simple built-in analytics, a built-in dbt engine, and hybrid tables that can serve OLAP or OLTP. A warehouse with a connection string loses even if its engine is faster.

Where a principle and a recommendation collide, the principle wins and the recommendation changes.

---

## 2. Scope: the GO SMALLER wedge

The business case (docs/BUSINESS-CASE.md) scored GO SMALLER at 59/90. A full Snowflake clone scored as a no-go. This section is the wedge that case argues for.

### 2.1 The segment

The wedge serves the priced-out. That means teams spending $5k to $150k a year on Snowflake, plus two groups Snowflake skips entirely: agent builders, and local-first teams. The offer from the case: serve the median Snowflake workload, about $100k a year, for $10k to $30k a year all in, starting at $0.

### 2.2 In scope for v1

- One fast OLAP engine per tenant, sized up and down, with scale to zero (Section 3, pending D-01).
- Iceberg tables in the customer's own bucket as the default storage format.
- Time travel and zero-copy cloning from Iceberg snapshots.
- Live sharing as grants, with consumer-side compute billing.
- Governance (roles, row and column policies, masking) in the base tier.
- The experience layer basics: data navigation, SQL worksheets, simple built-in analytics (Section 5, pending D-03).
- The full agent layer at launch (Section 6).
- Metering on the familiar usage model at lower margin, with per-query cost attribution and hard budgets in the product (Section 7).
- A permanent free tier at MotherDuck parity or better.

### 2.3 Out of scope for v1

- Snowflake parity. Most of the Snowflake surface stays unbuilt until revenue exists. The kill criteria in Section 8 treat parity creep as a failure mode.
- Multi-node execution of a single query. The recommended starting archetype has a single-query ceiling, and the design says so in public (pending D-01).
- Multi-cluster concurrency and cross-region operation.
- Hybrid tables. They ship last in the experience sequence (Section 5, pending D-03).
- Snowpark, container services, native apps, clean rooms, and the rest of the platform surface Snowflake added after the warehouse core.
- Fail-safe as Snowflake defines it. Iceberg has no equivalent. Longer snapshot retention plus backup is the planned substitute, and the gap gets documented for users rather than papered over.

### 2.4 The gates this scope must pass

- **Gate 1:** a DuckDB-centric prototype serves a representative workload set (built by us, modeled on documented Snowflake usage; no design partner, per Tom's ruling on Nov 5, 2026) at about a tenth of its Snowflake cost, on Snowflake's own arithmetic, without needing distributed execution.
- **Gate 2:** 10 paying orgs at full price, no discount doing the work, converted from the free tier at 1% or better.

Section 8 ties the phases to these gates.

---

## 3. Architecture

Status of this whole section: **recommendation, pending Tom's ruling on D-01.** The technical research (docs/research/02) lays out four archetypes. This design recommends one starting point and one exit ramp. Tom rules before the design is final.

### 3.1 Recommendation: Archetype A to start, Archetype B as the exit ramp

**Recommendation:** start with Archetype A. One DuckDB engine instance per tenant, running on shared infrastructure. MotherDuck calls this hypertenancy and runs it in production. The research estimates an MVP warehouse product in about 6 to 12 months for a small team on this archetype, against 12 to 18 months for a DataFusion-native MVP (Archetype B) and 2 to 3 years for a credible multi-node warehouse.

Three reasons, straight from the research:

1. Gate 1 tests exactly this shape. A prototype that serves the reference workload set at a tenth of Snowflake cost either validates the wedge or kills it cheaply.
2. DuckDB gives the best local story. The same engine runs in a laptop, in a browser, and in the hosted service, with the same SQL everywhere. Section 6 needs that for the agent layer.
3. Iceberg storage keeps the exit open. Data lives in an open format the other archetypes read natively. A later move to Archetype B (DataFusion core, custom coordinator) reads the same tables. The storage choice survives the engine choice.

The tradeoff, stated plainly: Archetype A caps a single query at one large node. A modern 32 to 128 vCPU box with NVMe and spilling handles tens to hundreds of GB and many TB-scale scans, per the research. Workloads past that ceiling need Archetype B. Choosing A means choosing that ceiling for v1 and telling users where it is.

Archetype C (adopt ClickHouse) and Archetype D (Iceberg plus Trino plus Spark) stay on the record in docs/DECISIONS.md. The research verdict on C: adopt it as-is for real-time event analytics, and expect a multi-year fight to give it Snowflake semantics. The verdict on D: distributed SQL works on day one, and the integration tax becomes the product.

### 3.2 Layers

Picture four layers. Storage at the bottom. The catalog and control plane above it. Compute above that. Services on top.

#### Storage

- **Apache Iceberg, version 2, is the default table format.** The research is direct on this: default to v2 for interoperability. Iceberg v3 adds deletion vectors, row lineage, and a VARIANT type, and engine support for v3 was still uneven as of Oct 2026. A table moves to v3 only when every reader and writer on that table supports it.
- **Parquet files live in the customer's bucket.** S3, GCS, or Azure Blob. The customer keeps the data; opensnow keeps pointers and metadata. Leaving takes an afternoon because the format is open and the bucket is theirs.
- **Compaction is a permanent service.** The research calls it a tax, and the numbers show why: 100k small files can cost about $800 a day in object storage requests at 10k queries a day, against about $16 a day compacted. So opensnow runs compaction, manifest rewrites, snapshot expiry, and orphan cleanup as a background service from day one. Target file sizes of 128 to 512 MB. This service runs forever. Budget it as operations, and treat skipping it as data corruption on a slow fuse.
- **Snapshots give time travel and cloning.** Every commit creates a new immutable snapshot. Time travel resolves a timestamp to a snapshot. A clone creates a new table whose first snapshot references the same files. Retention policy is the product knob that controls history cost.

#### Catalog and control plane

- **Polaris is the table catalog.** Apache-2.0. It speaks the Iceberg REST API, does RBAC, and vends short-lived credentials so engines read the customer's bucket directly. Snowflake's own Horizon catalog builds on Polaris, which makes it a good exit and migration story.
- **Postgres is the control plane for v1.** Tenants, users, roles, shares, billing state, query history, and locks live in Postgres. DuckLake points the same way: put metadata in a real transactional SQL database and a commit becomes one SQL transaction. Postgres is boring, well understood, and hosted everywhere.
- **What FoundationDB would buy later.** Snowflake has run its metadata on FoundationDB since 2014: catalog, users, locks with watches, transaction state, per-file statistics, billing, every statement. It scales to a global, strictly serializable control plane. Postgres gives up cross-region active-active writes and write scaling past a single primary. That trade is fine for v1. A FoundationDB move becomes a real question when the control plane needs multiple regions or write volume Postgres cannot take. The research calls FDB the Snowflake-faithful path and a multi-year expertise bet. We take that bet only when the numbers force it.
- **The gap list.** An Iceberg catalog handles atomic table commits and part of the statistics story. Everything else in the control plane is opensnow product work: sessions, multi-table transactions for clone and branch merges, the grants graph, lineage, metering, tenant lifecycle (provision, suspend, delete, erase across snapshots, clones, and shares).

#### Compute

- **One engine instance per tenant.** Each tenant gets its own DuckDB process. Isolation comes from construction: no shared workload manager, no noisy neighbor inside the engine. MotherDuck claims about 100 ms spin-up for its per-user instances; treat that figure as a vendor claim and as the bar to measure against.
- **Spin-up, suspend, scale to zero.** Instances start on first query and suspend when idle. Idle costs $0. Instance size scales up for a big query and back down after. This mirrors the virtual warehouse feel Snowflake users expect: independent sizing, auto-suspend, auto-resume.
- **Cost caps are platform-enforced.** Every tenant, token, and agent gets a hard spend cap and query limits. The engine enforces them. A runaway query dies at the cap. Section 6 leans on this for agents; it protects humans too.
- **The local mode is the same engine.** A user or an agent can run the identical DuckDB engine locally against local files or a sandbox copy. Same SQL, same behavior, zero account needed. This is the DuckDB bridge pattern from docs/research/04.

#### Services

- **Metering on the familiar usage model.** The meter tracks storage and compute usage the way Snowflake's model does, and Section 7 sets the margin policy. Every query records its cost at the query level. Attribution covers the serverless-style background work too: compaction, clustering, and maintenance get line items, since opaque background charges are a top Snowflake complaint.
- **Governance in the base tier.** Roles, grants, row-access policies, column masking, and tags ship in every tier. Snowflake gates masking and row policies behind Enterprise pricing at a 50% higher per-credit price. opensnow treats governance as part of the product, and the free and cheap tiers include it.
- **Sharing as grants.** A provider grants a consumer account read access to tables or views. No data copy. The provider pays storage, the consumer pays compute, and revocation cuts access. On Iceberg this means sharing a catalog namespace with scoped, vended credentials, plus machinery Snowflake built into its metadata layer and Iceberg leaves to us:
  - Cross-table file reference counting. Expiring a source snapshot must never delete files a clone or a share still references. Iceberg catalogs skip cross-table ref-counting, so opensnow builds a protection and garbage-collection layer above snapshots. Flag this as real build work with data-loss consequences if it goes wrong.
  - Policy propagation. Row and column policies travel with shared data. The Iceberg spec leaves this to the catalog and engine layer, so opensnow implements it and tests it.
  - Per-consumer metering. The provider sees who read what and the consumer sees their compute bill.
- **Result cache and metadata-only queries.** Repeated queries hit a result cache. Count and metadata questions answer from the catalog without starting compute, the way Snowflake answers them from its services layer.

### 3.3 What this architecture deliberately skips

The services layer is Snowflake's moat, per the research, and the layer where projects stall. This design builds the smallest services layer that passes Gate 1: catalog, control plane, metering, governance, sharing, compaction. Workload management across shared pools, a global cost-based optimizer with a decade of tuning, and multi-cluster scaling stay out until a later archetype earns them.

---

## 4. SQL dialect

Status: **open decision, D-02.** This section states the recommendation and its tradeoffs. Tom rules before the design is final.

**Recommendation:** Postgres-compatible core syntax, plus a Snowflake-compatibility layer covering the functions and idioms migrants use most. The compatibility list gets written down and tested. A novel dialect is off the table.

The research pulls in two directions, and both sides are real:

- **The agent side favors Postgres.** Postgres and standard SQL dominate the training data models learn from. Agents generate Postgres-flavored SQL accurately on the first try. On Spider 2.0, a benchmark of real enterprise text-to-SQL work, frontier models scored 10 to 21 percent. Dialect familiarity alone collapses on big schemas, and an unfamiliar dialect makes a hard problem worse.
- **The migrant side favors Snowflake.** Principle 3 promises familiar syntax. Snowflake users bring years of SQL with Snowflake habits: uppercase-folded identifiers, VARIANT colon paths, DATEADD conventions, ILIKE, LATERAL FLATTEN, QUALIFY. Every quirk an agent or a migrant trips on is a support ticket and a reason to leave.

The recommendation tries to hold both. Postgres core keeps agent-generated SQL working and keeps DuckDB close to its own flavor, since DuckDB SQL grew from Postgres roots. The compatibility layer absorbs the Snowflake surface migrants touch daily. The cost of this choice: a translation and testing burden that never ends, plus a published dialect page listing every deviation. The benefit: opensnow speaks the dialect agents write and forgives the dialect its first customers bring.

One guardrail from the research applies either way. Dialect is the smaller half of text-to-SQL accuracy. Schema and context tools in the MCP server (catalog search, sample rows, semantic descriptions) move accuracy more. Section 6 includes them at launch regardless of the D-02 ruling.

---

## 5. Experience layer

Status: **sequencing is an open decision, D-03.** Principle 4 makes this layer part of the product. This section follows the D-03 recommendation. Tom rules before the design is final.

Snowflake users live in Snowsight. They browse data, open worksheets, chart results, and share dashboards inside the product. The wedge ships the parts users feel daily, in this order:

1. **Navigate.** Catalog browsing: databases, schemas, tables, columns, sample rows, sizes, freshness. Search across the catalog. This is the front door and the demo.
2. **Query.** SQL worksheets: an editor with history, saved worksheets, results in a grid, cost shown per query as it runs. The worksheet shows the meter working, which is the Principle 2 pitch made visible.
3. **Analyse.** Simple built-in analytics: charts from a query result, dashboards assembled from saved queries, scheduled refresh, sharing a dashboard with a link inside the org. Depth stops at simple. Dedicated BI tools keep the deep end.
4. **Transform.** A built-in dbt engine: run dbt projects against opensnow without a separate orchestrator, with runs, logs, and lineage visible in the product. Snowflake ships dbt projects natively; analysts plan their week around it.
5. **Serve.** Hybrid tables: one table that serves OLAP scans and OLTP point reads. Flag this clearly. Hybrid tables are engine surgery. They change the storage and write path of the engine itself, and they compete with scale-out work for the same engineering time. The D-03 recommendation schedules them last, after the wedge proves out. Gate 1 and Gate 2 say nothing about hybrid tables, and v1 promises nothing about them beyond a place in the sequence.
6. **Share and govern.** The sharing grants and governance from Section 3, surfaced in the UI: who has access, what a share includes, what a policy hides, what each consumer costs.

Later phases can add notebooks, an in-product AI assistant, and app building. Each addition must clear the same bar: users feel it weekly, and it serves the priced-out segment first.

---

## 6. Agent layer

Status: **launch requirements, from docs/research/04.** Principle 1 makes agents a primary user. DuckDB and Supabase won this channel through architecture: an agent can start alone, speak a language it knows, and fail cheaply. The checklist below turns that into launch work. Items here are requirements for launch, and the phasing in Section 8 protects them from slipping.

### 6.1 Architecture requirements

- Postgres-compatible SQL surface, with a published dialect page listing every deviation. Aim: zero deviations in the common 95 percent of queries (pending D-02).
- A local and sandbox mode on the same engine. A single binary or a one-line install. In-memory databases and local files work with zero account, zero credentials, zero cost.
- Query files in place. Parquet, CSV, JSON, and Iceberg by path or URL. The first query needs no CREATE TABLE, no load step, no warehouse sizing.
- Programmatic provisioning for everything. An API call creates a project or database and returns a connection string plus a scoped token in under 5 seconds, with under 1 second as the target. Every dashboard action has an API or CLI equivalent. Neon is the benchmark: its create call returns a working connection string in seconds, and over 80 percent of its databases were agent-created by 2025.
- Agent sandboxes. Instant branches or zero-copy clones per agent or per task, deletable through the API, on isolated compute.
- Hard cost caps per agent and per token, enforced by the platform. Cost estimate before execution, actual cost in every query response, so an agent can throttle itself.
- Scoped, short-lived, revocable tokens. Agents mint and rotate their own tokens. MCP access runs on OAuth. The default role is read-only.

### 6.2 Signup without a human

MotherDuck's pattern, copied: an API endpoint creates a free organization and returns a token plus a claim URL. A human claims ownership later. The agent starts work in minutes; the paperwork happens when a person shows up. This endpoint ships at launch, and it gets the same abuse controls as any signup path.

### 6.3 MCP

- A hosted remote MCP server (Streamable HTTP, OAuth) and a local stdio server, both registered in the official MCP Registry and the Claude, Cursor, and VS Code directories on launch day.
- Tools: catalog and table discovery, fuzzy schema search, sample rows, a read-only query tool, a separate write and migration tool, query logs, cost lookup, and docs search. Docs search matters: the agent grounds itself in current docs at runtime instead of guessing.
- Safety defaults from the Supabase playbook: read-only mode as the default, project scoping, tool-group allow-lists, and confirmation prompts for destructive SQL and for actions with a price tag.

### 6.4 Docs and skills

- `/llms.txt` written to a recommending agent: when to use opensnow, when to reach for something else, the free tier, the MCP endpoint, the signup endpoint, pricing in plain text. Plus `/llms-full.txt`. Every docs page also serves as clean Markdown.
- An OpenAPI spec for the provisioning and management APIs.
- An agent skills repo under a permissive license: install with one command, covering opensnow SQL practice and product specifics, with discovery files where agents look for them. A plugin that bundles MCP plus skills in one install for the major coding agents.
- Agent instruction pages, following the Supabase pattern: a declarative schema workflow, a migration workflow, a debugging workflow. Pages an agent can be pointed at, written as procedures.
- Public example repos in idiomatic opensnow SQL. Today's examples become tomorrow's training data.

### 6.5 What waits

Platform partnerships (bundled default inside AI builders, marketplace provisioning) come after launch, and the provisioning API gets designed for them from day one. Semantic-layer tooling for large schemas, fine-tuned text-to-SQL, and a public counter of agent-created share all come later too. Section 5 of docs/research/04 holds the full later list.

---

## 7. Pricing and metering

Principle 2 governs this section. The meter stays familiar. The margin shrinks.

- **Usage-based pricing, like Snowflake.** Customers pay for storage and compute usage, measured and reported in terms a Snowflake customer already understands. A radical new meter is rejected: familiarity is the feature, since a migrant can predict the bill on day one.
- **Lower gross margin, on purpose.** Snowflake's product gross margin runs around 75 percent, and its filings show sales and marketing at 44 percent of revenue. opensnow skips the sales force and accepts thinner margin. The business case offer: the median workload at $10k to $30k a year all in, against about $100k on Snowflake.
- **No per-user pricing, ever.** Principle 3. Seats are free. Usage is the bill.
- **Per-query cost attribution in the product.** Every query shows its cost. Every background service (compaction, maintenance, serverless-style tasks) appears as its own line item. Hard budgets per tenant, per token, per agent, enforced by the platform with alerts before the cap and a stop at the cap. The FinOps industry that exists to explain Snowflake bills is the market proof that this transparency matters.
- **Granularity fixes ride along.** Per-second metering, scale to zero, and no charge while idle. Snowflake's 60-second minimum per warehouse start stays a useful comparison point, and Principle 2 ranks it below the margin move.
- **Free tier at MotherDuck parity or better.** Permanent, no credit card. MotherDuck's free tier offers 10 GB of storage and 10 compute hours a month. opensnow matches or beats that scale and keeps the tier useful for a real prototype. A 30-day trial is a different thing from a tier, and opensnow ships the tier. Gate 2 assumes free-to-paid conversion at 1 percent or better, the range the business case research found for open-source infrastructure (roughly 1 to 3 percent).

---

## 8. Phasing

Phases follow the gates from the business case. Work starts only after Tom rules on the decision queue and approves this design.

### Phase 0: Design and prototype prep

- Rule on D-01 to D-06.
- Build the reference workload set: analytical workloads sized to the priced-out segment (Section 2), modeled on documented Snowflake usage patterns, each with its Snowflake cost worked out on published Snowflake pricing.
- Write the Snowflake-compatibility list for D-02 and the cost model that Gate 1 will be judged against: our real AWS costs against the same workloads priced on Snowflake's arithmetic. No design partner feeds this (Tom, Nov 5, 2026); the cost model gets rebuilt from real user bills after launch.

### Phase 1: Gate 1

Build the thinnest end-to-end slice: DuckDB per tenant, Iceberg storage with compaction, Postgres control plane, worksheets and navigation in the UI, metering with per-query costs, the MCP server and local mode from Section 6.

**Gate 1 passes when** the prototype serves the reference workload set at about a tenth of its Snowflake cost, on Snowflake's own arithmetic, without needing distributed execution.

A Gate 1 failure is useful data. It means the wedge shape is wrong, and the kill criteria below decide what happens next.

### Phase 2: Gate 2

Open the free tier. Ship the agent signup endpoint, the skills repo, and the registry listings. Add built-in dbt (Section 5 sequence). Harden sharing and governance for paying teams.

**Gate 2 passes when** 10 orgs pay full price with no discount doing the work, converted from the free tier at 1 percent or better.

### Phase 3: After the gates

Scale work earns its place here: bigger nodes, more concurrency, the Archetype B evaluation with real workload data in hand, hybrid tables per the D-03 sequence, platform partnerships for builder distribution. Each item enters through the decision queue with its own ruling.

### Kill / park criteria

Copied from docs/BUSINESS-CASE.md. Park if any two hold:

- Target workloads need multi-node queries the wedge cannot serve.
- Free-tier cost breaks the $10k to $30k target at 70% gross margin.
- MotherDuck closes the open-source and self-host gap before opensnow differentiates.
- Services-layer scope creeps toward Snowflake parity before revenue exists.

---

## 9. Decision queue

Restated from docs/DECISIONS.md. Tom approved the recommendations for D-01 to D-06 on Nov 5, 2026. Each item below records the ruling.

### D-01: Starting engine archetype

Four archetypes from the research: A (DuckDB-centric, one engine per tenant), B (DataFusion-native, built for multi-node), C (adopt ClickHouse), D (Iceberg plus Trino plus Spark assembly).

Recommendation: A to start, because Gate 1 tests exactly it, it is the fastest path to a working product, and the Iceberg storage layer keeps the exit to B open. Choosing A is choosing a single-query ceiling for v1 and saying so in public.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

Sub-study queued under this ruling: evaluate DuckLake as an A-variant storage layer against the Iceberg default (see docs/DECISIONS.md). The evaluation informs the Phase 0 storage spike; it does not reopen the archetype.

### D-02: SQL dialect

Recommendation: Postgres-compatible core syntax, with a Snowflake-compatibility layer for the functions and idioms migrants use most (a documented list, tested). Never a novel dialect.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

### D-03: Experience layer sequencing

Recommendation: navigation, worksheets, and simple analytics in the wedge. Built-in dbt next. Hybrid tables last: they are engine surgery and compete with scale-out for the same effort.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

### D-04: License

Recommendation: Apache-2.0 for opensnow itself, with a license check on every dependency before it enters the core.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

### D-05: Hosting model for v1

Recommendation: both, in order. The same artifact self-hosts free, and the hosted version is that artifact run by us. Hosted starts on one cloud.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended. First cloud decided the same day: AWS.

### D-06: The name

Recommendation: keep the codename for the build phase; run the naming decision before any public launch. opensnow sits close to Snowflake's marks and probably cannot ship as the product name.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

---

## 10. Risks and unknowns

### Risks the research surfaced

- **The single-query ceiling (Archetype A).** One node caps one query. If the reference workloads hit that cap early, Gate 1 fails and the first kill criterion fires. The mitigation is publishing the ceiling plainly plus the Iceberg exit ramp, and both cost time if the move to Archetype B comes early.
- **Distributed execution is where projects stall.** Every path to multi-node runs through the hardest distributed systems problems in databases: shuffle, skew handling, fault tolerance, a global optimizer. Archetype B estimates run 2 to 3 years and 10+ engineers for a credible multi-node warehouse. The wedge survives by deferring this, and deferral has an expiry date.
- **Services-layer scope creep.** The services layer is the moat and the tar pit. The research found DIY lakehouse assembly alone running 3 to 6 FTE and 30 to 40 percent of build cost per year in ongoing maintenance, in vendor-adjacent estimates. The fourth kill criterion watches this directly.
- **MotherDuck closes the gap.** MotherDuck has the hosted DuckDB layer, the agent story, and $100M raised. Its weak spots today are the self-host and open-source gaps the business case targets. If it closes them first, the third kill criterion fires.
- **Compaction and garbage collection bugs lose data.** The cross-table ref-counting layer in Section 3 guards files that clones and shares still reference. A bug there deletes live data. This service needs the strongest tests in the system.
- **Forking an engine to distribute it fails.** Bauplan forked DuckDB, hit its I/O and Iceberg limits, and moved to DataFusion. This design avoids engine forks. If a fork ever looks necessary, that is a decision-queue item, not an engineering shortcut.

### Licensing traps

Per D-04, the core stays permissive. The research flags license classes to keep out of the core, each needing a legal read at the exact version before any use:

- AGPLv3 components (ParadeDB is the named example in the research).
- Elastic License 2.0 and other source-available terms (Airbyte ELv2 is the named example).
- BSL-class and changed-license components (Redpanda's BSL parts, CockroachDB's move away from open core terms).

DuckDB (MIT) and Polaris, Iceberg, and DataFusion (Apache-2.0) pass the check today. Every future dependency gets the same check before it enters the core.

### Unknowns the research left open

- Snowflake bill percentiles and edition mix. The cost model for the median workload rests on reported contract data (a roughly $100k median against a roughly $336k mean per customer). The cost model gets rebuilt from real user bills after launch; before launch it rests on published Snowflake pricing applied to our reference workloads.
- MotherDuck customers and revenue. The closest competitor's traction is unmeasured, so the differentiation window is a judgment call.
- Independent price-performance benchmarks at small scale. Vendor benchmark wars (Databricks against Snowflake, ClickHouse against everyone) are disputed by construction. opensnow needs its own reproducible benchmark on the reference workload set before it claims speed in public.
- Iceberg v3 adoption timing. The v2 default holds until engine support evens out. Nobody in the research could date that.
- Whether agent-created projects convert to paid workloads at scale. Neon and Supabase show the creation numbers. Public conversion numbers do not exist yet. Gate 2 is our own measurement of exactly this.

### Operating model (decided Nov 5, 2026)

- Team: Tom plus the Muse and Codex building loop, the same way OpenSight and freeDMS are built. No hires, no outside funding, no design partner. Gate 1 scope is sized to that loop.
- First cloud: AWS (under D-05).
- Gate 1 validation runs on reference workloads we build ourselves, priced against published Snowflake rates, with our real AWS bills as the cost side.
