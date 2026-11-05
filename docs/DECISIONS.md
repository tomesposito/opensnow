# Decision Queue

Decisions for the solution design. Tom approved the recommendations for D-01 to D-06 on Nov 5, 2026; each is marked DECIDED below.

## D-01 — Starting engine archetype

Context: four archetypes from the technical research (docs/research/02). A: DuckDB-centric, one engine instance per tenant. B: DataFusion-native, built for multi-node. C: adopt ClickHouse. D: Iceberg + Trino + Spark assembly.

Recommendation: A to start, because Gate 1 tests exactly it, it is the fastest path to a design partner, and the Iceberg storage layer keeps the exit to B open. Choosing A is choosing a single-query ceiling for v1 and saying so in public.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

## D-02 — SQL dialect

Context: the real tension in the research. Agents generate Postgres-flavored SQL best, because it is dense in training data. Snowflake migrants expect Snowflake-flavored SQL. Principle 3 says familiar syntax; the agent research says Postgres density is distribution.

Recommendation: Postgres-compatible core syntax, with a Snowflake-compatibility layer for the functions and idioms migrants use most (a documented list, tested). Never a novel dialect.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

## D-03 — Experience layer sequencing

Context: Principle 4 makes the experience part of the product: data navigation, SQL worksheets, simple built-in analytics, a built-in dbt engine, hybrid tables.

Recommendation: navigation, worksheets, and simple analytics in the wedge (they are what users feel daily, and they are the demo). Built-in dbt next. Hybrid tables last: they are engine surgery and compete with scale-out for the same effort.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

## D-04 — License

Context: freeness is part of the pitch. DuckDB is MIT; several components in the stack are Apache-2.0; some adjacent projects are AGPL or source-available and must be kept out of the core.

Recommendation: Apache-2.0 for opensnow itself, with a license check on every dependency before it enters the core.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

## D-05 — Hosting model for v1

Context: the wedge can be hosted-only (one cloud, one region), self-host first, or both. MotherDuck's lesson is that hosted convenience is what people pay for; the OSS lesson is that self-host is what makes it spread.

Recommendation: both, but in order: the same artifact self-hosts free, and the hosted version is that artifact run by us. Hosted starts on one cloud.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

## D-06 — The name

Context: opensnow is a codename. It is close to Snowflake's marks and probably cannot ship as the product name.

Recommendation: keep the codename for the build phase; run the naming decision before any public launch.

Status: DECIDED (Tom, Nov 5, 2026): approved as recommended.

## D-01 sub-study — DuckLake vs the Iceberg default

Queued Nov 5, 2026 under the D-01 ruling. The technical research (docs/research/02) calls DuckLake a core candidate for Archetype A: metadata in Postgres, Parquet in object storage, commit as one SQL transaction, the closest live experiment to Snowflake's metadata-in-a-database design. The design defaults to Iceberg (portable, engine-neutral, the exit ramp to Archetype B). Evaluate DuckLake against that default during the Phase 0 storage spike on: multi-engine access, time travel and clone semantics, compaction story, and exit cost if opensnow later moves to Archetype B. Finding goes to Tom as an amendment to D-01, not a silent swap.
