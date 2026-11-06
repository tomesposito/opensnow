# opensnow

A better, faster, freer version of Snowflake. Codename only, until a better name is chosen.

Snowflake proved the shape: separate storage from compute, delete the DBA job, and make time travel, cloning, and sharing metadata operations. What it did not do is stay cheap, stay simple, or open up. opensnow keeps the shape and fixes those three.

Status: research and business case complete. Verdict: GO SMALLER (see docs/BUSINESS-CASE.md). Solution design approved Nov 5, 2026, with D-01 to D-06 decided (see SOLUTION_DESIGN.md and docs/DECISIONS.md). Phase 1 has started. The Rust workspace has engine, storage, and combined stack-check crates. The pinned stack is recorded in [docs/PINNED-STACK.md](docs/PINNED-STACK.md).

## Principles

Four, set by Tom on Nov 5, 2026. They outrank anything else in this repo.

1. **AI-first, agent-attractive from day one.** Modeled on DuckDB and Supabase. Agents searching for and building with the product are a primary user: machine-readable docs, an MCP server, programmatic provisioning, an instant free start, and SQL an agent can generate reliably.
2. **Win on cost by cutting margin, not by changing the meter.** Snowflake customers are reeling from cost. Keep Snowflake's familiar usage-based pricing model (usage, not per-user) and charge less for the same usage by accepting lower gross margins. Granularity fixes such as the 60-second minimum are secondary.
3. **Keep what makes Snowflake successful.** Simple data sharing, no pricing by user, familiar syntax, trusted performance, zero-tuning elasticity.
4. **Engine plus experience.** A powerful, fast, trustworthy OLAP engine is table stakes. So is the layer on top of it: easy data navigation, SQL worksheets, simple built-in analytics, a built-in dbt engine, and hybrid tables that can serve OLAP or OLTP. A warehouse with a connection string loses even if its engine is faster.

## Docs

- SOLUTION_DESIGN.md - vision, architecture, and the decided design (living document)
- docs/BUSINESS-CASE.md - the scored case, verdict, gates, and kill criteria
- docs/DECISIONS.md - the decision queue for the solution design (D-01 to D-06 decided)
- docs/research/01-snowflake-research.md - what Snowflake is, why users love it, where it leaks
- docs/research/02-technical-architecture-research.md - engines, table formats, catalogs, and four build archetypes
- docs/research/03-business-case-research.md - market, pricing, competition, funding evidence
- docs/research/04-agent-first-duckdb-supabase.md - what made DuckDB and Supabase win with agents

## License

To be decided in the solution design. Open source is the intent.
