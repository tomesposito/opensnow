# opensnow: Agent-First From Day One — What DuckDB and Supabase Actually Did

*Research for opensnow, a Snowflake alternative. Read 2026-11-05. Every load-bearing fact is cited with its source URL and flagged `verified live` (opened and read on the primary page) or `index` (search-result text from that page). Vendor claims are flagged **[vendor claim]**. Revenue/valuation figures for private companies are estimates unless noted.*

## Summary

DuckDB and Supabase did not win the agent era by adding AI features on top. They won because, **before agents existed as a channel, their core architecture already matched how an agent works**:

1. **An agent can start without a human.** DuckDB: `pip install duckdb` / `import duckdb`, no server, no account, no credentials, no cost. Supabase/Neon: a free project provisioned by API/MCP in seconds, no credit card, no sales call.
2. **An agent already knows the language.** DuckDB speaks Postgres-flavoured standard SQL and reads Parquet/CSV/JSON in place; Supabase *is* Postgres. LLMs have seen orders of magnitude more Postgres/standard-SQL than any proprietary dialect, so generated code works first time — and when it doesn't, execution is cheap and local/fast enough for a self-correction loop.
3. **Failure is cheap and safe.** A wrong DuckDB query costs milliseconds on a laptop/sandbox. A wrong Supabase/Neon query costs a branch or a free project. A wrong Snowflake query can cost a 60-second warehouse minimum, every resume, in credits a human has to explain.
4. **Distribution moved inside the agent's tools.** MCP servers (hosted, OAuth, one URL), agent skills (`npx skills add …`), `llms.txt` / `llms-full.txt`, markdown docs, and — decisively for Supabase — being the *bundled default backend* inside Lovable, Bolt, v0 and Figma Make.

The payoff is now measured, not anecdotal:

- **Neon (the control case):** “over 80 percent of the databases provisioned on Neon were created automatically by AI agents rather than by humans” — Databricks, announcing its ~$1B Neon acquisition, May 2025 (`index`) — https://www.aireporteramerica.com/news/7/692/databricks-to-acquire-neon-supercharging-ai-agent-workflows-with-serverless-postgres.html — up from 0.1% in Oct 2023 (`index`) — https://www.gocrazy.dev/learn/feng-shui-for-robots-the-fly-mcp-server/
- **Supabase:** database launches +600% year-on-year and “more than 60% of new databases are now started by an AI coding tool rather than a person, with Anthropic's Claude Code the largest single source” — CEO Paul Copplestone, quoted at the $500M Series F / $10.5B valuation **[vendor claim]** (`index`) — https://www.implicator.ai/supabase-doubles-to-10-5-billion-as-agents-deploy-most-of-its-databases/
- **DuckDB:** 50M+ PyPI downloads/month in Aug 2026, more than double the 20M+ reported in summer 2025, 40,000 GitHub stars (`verified live`) — https://duckdb.org/2026/08/05/github-40k-stars

**For opensnow the transferable core is not “be embedded” (a cloud warehouse can't) — it is: local/sandbox mode with the identical engine and dialect, sub-second programmatic provisioning, per-query (not per-minute) pricing with hard per-agent cost caps, standard/Postgres-compatible SQL, and MCP + skills + llms.txt shipped at launch, not a year later.** Snowflake shipped its managed MCP server GA only on 4 Nov 2025, and it exposes Cortex Analyst/Search — not provisioning or general SQL-first agent operation (`verified live`) — https://docs.snowflake.com/en/release-notes/2025/other/2025-11-04-cortex-agents-mcp

Classification key used throughout: **(a)** product architecture · **(b)** distribution/docs · **(c)** pricing/packaging.

---

## 1. DuckDB

### 1.1 Classic factors

| Factor | Class | Evidence |
|---|---|---|
| In-process / embedded — “SQLite for analytics”. No DBMS server to install, update or maintain; embedded in the host process, high-speed (sometimes zero-copy) data transfer | (a) | “For DuckDB, there is no DBMS server software to install, update, and maintain… completely embedded within a host process… the DuckDB Python package can run queries directly on Pandas data without ever importing or copying any data” (`verified live`) — https://duckdb.org/why_duckdb |
| Zero setup, single file, no dependencies | (a)/(c) | “DuckDB has no external dependencies… Data can be stored in DuckDB's native format, a single-file database” (`verified live`) — https://duckdb.org/why_duckdb |
| Runs everywhere | (a) | Compiled for Linux/macOS/Windows, x86/ARM, “from small, resource-constrained edge devices to large multi-terabyte memory servers… Using DuckDB-Wasm, DuckDB can also run in web browsers and even on mobile phones”; APIs for C, C++, Go, Python, R, Rust, Java, Node.js (`verified live`) — https://duckdb.org/why_duckdb . Lambda/serverless use follows from single-binary + in-process design; it is widely cited in secondary coverage but was not separately verified live here. |
| Free, MIT, foundation-held | (c) | “released under the very permissive MIT License and the project's intellectual property is held by the DuckDB Foundation” (`verified live`) — https://duckdb.org/why_duckdb |
| Fast vectorized engine | (a) | “DuckDB uses a columnar-vectorized query execution engine, where… a large batch of values (a ‘vector’) are processed in one operation” vs row-at-a-time PostgreSQL/MySQL/SQLite (`verified live`) — https://duckdb.org/why_duckdb |
| Reads Parquet/CSV/JSON directly, no ingestion | (a) | Parquet, JSON, time zones, HTTP(S)/S3 are extensions; extensions also work in Wasm (`verified live`) — https://duckdb.org/why_duckdb . Canonical agent pattern: `SELECT * FROM 'sales.parquet'` — no CREATE TABLE / load step for an agent to get wrong. |
| Notebook / data-science fit | (a)/(b) | “deeply integrated into Python and R”; Python API supports Pandas, Arrow and Polars (`verified live` / `index`) — https://duckdb.org/why_duckdb , https://en.wikipedia.org/wiki/DuckDB |
| Familiar SQL | (a)/(b) | SQL parser was historically derived from PostgreSQL's parser (repackaged as pg_query); v2.0 moved to its own PEG parser (`verified live`) — https://duckdb.org/why_duckdb . Net effect for LLMs: Postgres-flavoured SQL, the dialect with the densest training-data coverage. |

### 1.2 Growth mechanics and business model — with a correction

- **Growth:** 25k stars (Dec 2024) → 30k (6 June 2025, with “20M+ monthly downloads in PyPI” and 3M+ monthly website visitors) → **40k stars (5 Aug 2026, “50M+ monthly downloads in PyPI”, 8M+ monthly unique visitors)** (`verified live` for 40k post; `index` for 30k post) — https://duckdb.org/2026/08/05/github-40k-stars , https://duckdb.org/2025/06/06/github-30k-stars . Stack Overflow survey usage cited in secondary coverage: 1.4% → 3.3% of respondents 2024→2025 (`index`) — https://medium.com/data-arkaive/duckdb-gpu-acceleration-after-2-0-where-it-wins-where-it-doesnt-and-why-the-mac-is-different-a4f37d463150
- **DuckDB Labs / DuckLabs model — do not copy the VC story circulating in dossiers.** DuckDB's creators' company deliberately did **not** take venture capital (“We feel investment would force the project direction towards monetization”), funding development via support/consultancy; IP sits in the non-profit DuckDB Foundation. In Aug 2026 AWS agreed to acquire DuckLabs (the people), explicitly **not** the open-source project, which stays MIT under the Foundation (`index`) — https://en.wikipedia.org/wiki/DuckDB , https://aibacon.net/posts/aws-acquires-ducklabs-duckdb-foundation-mit-zero-vc.html . A third-party “DuckDB Intelligence Dossier” claiming a DuckDB Labs Series B of $87M at ~$750M is contradicted by these sources and should be treated as erroneous (`index`) — https://www.crows-eye.com/duckdb
- **MotherDuck is the monetisation layer, and a separate company:** serverless cloud warehouse on DuckDB, founded 2022 by ex-BigQuery founding engineer Jordan Tigani; $52.5M Series B (Sep 2023, Felicis lead, a16z among existing investors) brought total raised to **$100M** **[vendor announcement]** (`index`) — https://motherduck.com/blog/motherduck-open-for-all-with-series-b/
- **Product expansion that matters for agents:** Local UI — Mar 2025, DuckLabs + MotherDuck, browser UI shipped as the `ui` extension with an explicit path to MotherDuck cloud; **Quack** — technical preview May 2025, client-server protocol (`CALL quack_serve('quack:localhost', token=…)` / `ATTACH 'quack:…'`) enabling multi-writer remote DuckDB, polished in DuckDB 2.0 (21 Oct 2026) (`index`) — https://duckdb.org/history/ , https://duckdb.org/2026/05/20/announcing-duckdb-153 . Quack is the bridge from “embedded only” to “cloud warehouse with the same engine/dialect” — exactly opensnow's local→cloud problem.

### 1.3 Why LLM agents recommend and *succeed* with DuckDB

1. **No provisioning step to fail at (a).** An agent's tool loop is: install → import → query. There is no account, connection string, firewall, warehouse sizing, or credential for the agent (or its human) to paste. Every one of those steps is a drop-off point where agents abandon or hallucinate.
2. **Local execution = safe sandbox (a).** The agent can run `:memory:` or a throwaway file, execute generated SQL, read the real error, and retry — with zero cost, zero blast radius, and no production data. The canonical LangChain/MotherDuck agent pattern is explicitly that loop: inspect schema (`sql_db_list_tables`, `sql_db_schema`) → draft DuckDB SQL → check (`sql_db_query_checker`) → execute → read error/result → self-correct (`index`) — https://motherduck.com/blog/langchain-sql-agent-duckdb-motherduck/
3. **Training-data density (b).** A decade of notebooks, dbt projects, blog posts and Stack Overflow answers in DuckDB/Postgres-flavoured SQL means models recommend DuckDB unprompted and generate its dialect accurately. This is self-reinforcing: 50M downloads/month generates more public code, which generates more training data.
4. **Text-to-SQL reality check (a)/(b).** Familiar dialect helps, but does not solve enterprise text-to-SQL. Spider 2.0 (real enterprise workflows across BigQuery/Snowflake/DuckDB/SQLite, 1,000+-column schemas) scored GPT-4o at 10.1% and o1-preview at 17.1–21.3%, versus ~86–91% on Spider 1.0 (`index`) — https://github.com/phoebefu6/learn-text-to-sql-with-phoebe/blob/HEAD/materials/market-landscape.md . Lesson for opensnow: ship schema/context tools (catalog search, sample rows, semantic descriptions) with the MCP server — dialect familiarity alone collapses on large schemas.
5. **A dedicated text-to-SQL model existed:** `duckdb-nsql` (a DuckDB-dialect-fine-tuned model) is cited in secondary research as “dialect-perfect” but Llama-2-based and stale (`index`) — https://github.com/professorgrumblemore/basanti-app/blob/HEAD/STRATEGY.md . Treat as directional evidence that dialect-specific fine-tuning was worth doing, not as a current asset.
6. **DuckDB/MotherDuck's own AI moves (b):**
   - **Local MCP server (`mcp-server-motherduck`, MotherDuck):** `uvx mcp-server-motherduck --db-path :memory: --read-write`, connects AI assistants to local DuckDB files, in-memory DBs, S3 and MotherDuck; read-only by default; originally a single `query` tool — deliberately minimal (`index`) — https://pypi.org/project/mcp-server-motherduck/0.6.3/
   - **Remote hosted MCP (MotherDuck):** `https://api.motherduck.com/mcp` (Streamable HTTP), with `query` (read-only) and `query_rw`, fuzzy catalog search, `ask_docs_question` (answers from official docs before web search), plus a manifest at `/.well-known/mcp.json` (`verified live` via llms.txt) — https://motherduck.com/llms.txt , (`index`) — https://motherduck.com/learn/motherduck-ai-agent-data-layer/
   - **The llms.txt as a product spec — study this file.** MotherDuck's `llms.txt` is written *to agents making a recommendation*: “When to use MotherDuck / Do not reach for MotherDuck for OLTP…”, per-agent isolated “Duckling” spinning up in 100ms, cost isolation (“an agent querying a Standard Duckling cannot run up Giga prices”), free tier, and — most radically — **“Sign up without a human: `POST https://new.motherduck.com` creates a Free Plan organization and returns `motherduck_token`, `claim_org_url`, and `how_to_use_motherduck`”**, with a human claiming ownership later (`verified live`) — https://motherduck.com/llms.txt . It also offers every page as Markdown (Accept: `text/markdown` or append `.md`), agent skills at `/.well-known/agent-skills/index.json`, an OpenAPI spec, and `llms-full.txt` for full docs. **[All MotherDuck performance/pricing framing in that file is vendor claim.]**
   - **Per-agent economics (a)/(c) [vendor claim]:** hypertenancy — every user/agent gets a dedicated DuckDB instance; Pulse instances “meter per query rather than by wall-clock time, so exploratory or bursty agent queries cost you per query, not per hour of idle compute” (`index`) — https://motherduck.com/learn/motherduck-ai-agent-data-layer/

**Transferability to a cloud warehouse:** embedded itself does not transfer; *everything it causes* does — identical local engine for dev/sandbox, one-line install, query-files-in-place, zero-cost retry loops, Postgres-compatible SQL. MotherDuck is the proof that the bridge (local DuckDB → `md:` → cloud, same SQL) is buildable.

---

## 2. Supabase

### 2.1 Classic factors

| Factor | Class | Evidence |
|---|---|---|
| “Open-source Firebase alternative” positioning | (b) | Founded 2020 by Paul Copplestone and Ant Wilson as an open-source alternative to Google's Firebase (`index`) — https://dailyfinancial.in/supabase-grew-111-in-a-single-year-the-shocking-numbers-behind-its-rise-to-4-million-developers-and-a-5-billion-valuation/ . One sentence an LLM (and a human) can repeat verbatim. |
| Postgres, not a novel store | (a) | Relational/SQL, familiar and trusted; agents already generate correct Postgres. Firebase's document model + proprietary query language is the counter-example: less training data, more generated-code errors. |
| Batteries included, instant API | (a) | Postgres + Auth + Storage + Realtime + Edge Functions, auto-generated REST/GraphQL APIs, dashboard. Platform post lists the full stack as what every provisioned project gets (`verified live`) — https://supabase.com/blog/introducing-supabase-for-platforms |
| “Build in a weekend” DX | (b) | Tagline and Launch Week cadence; sharp focus on the *initialization* event (first project/first query) is credited by investor Craft Ventures for growth from 1M to 4.5M developers in <1 year (`index`) — https://www.craftventures.com/articles/inside-supabase-breakout-growth |
| Free tier, no card | (c) | Free: $0, 2 active projects, 500 MB database/project, 50,000 MAU, 1 GB file storage, 5 GB egress, unlimited API requests, no credit card; projects pause after 1 week idle; Pro $25/mo (secondary sources consistent with official pricing page, Sep–Oct 2026) (`index`) — https://makerkit.dev/blog/saas/supabase-pricing , https://freetier.wiki/services/supabase/ |
| YC + community | (b) | YC alumnus; >50% of the latest YC batch (at Series E) used Supabase **[vendor/PR claim]** (`index`) — https://pr.wvcjournal.com/article/Supabase-Raises-dollar100M-at-dollar5B-Valuation-Co-Led-by-Accel-and-Peak-XV/68dfad893ce2ea39bbf99d0d |

### 2.2 Growth / scale numbers — flag the soft ones

- **Oct 2025 (Series E, hardest numbers):** $100M raised at **$5B valuation** (Accel + Peak XV co-lead), total funding **>$500M**, **>4 million developers**, **>100,000 customers** including PwC, McDonald's, GitHub Next (`index`, PR Newswire release) — https://pr.wvcjournal.com/article/Supabase-Raises-dollar100M-at-dollar5B-Valuation-Co-Led-by-Accel-and-Peak-XV/68dfad893ce2ea39bbf99d0d . Four months earlier: $200M Series D at $2B (Apr 2025) (`index`) — https://www.Bbntimes.com/technology/supabase-soars-to-5b-the-open-source-giant-is-redefining-ai-development
- **2026 (Series F) [vendor claims via press]:** $500M at **$10.5B**, total capital >$1B, ~9–10M developers, ~250,000 customers, ~350 employees, databases +600% YoY, **>60% of new databases started by an AI tool, Claude Code the largest single source** (`index`) — https://www.implicator.ai/supabase-doubles-to-10-5-billion-as-agents-deploy-most-of-its-databases/ , https://theaiworld.org/news/supabase-raises-500m-at-105b-ai-agents-lead . Note the implication in the first source: valuation “runs on usage, not disclosed revenue.”
- **Revenue: do not quote a single number.** Supabase has repeatedly declined to disclose revenue. Third-party estimates conflict wildly — e.g. ~$70M ARR for 2025 (Sacra via secondary), $101M ARR Dec 2025 / $170M May 2026 (Sacra via another secondary), vs $27–30M in other trackers (`index`) — https://dailyfinancial.in/supabase-grew-111-in-a-single-year-the-shocking-numbers-behind-its-rise-to-4-million-developers-and-a-5-billion-valuation/ , https://tech-insider.org/supabase-vs-firebase-2026/ . All are estimates, not company financials.
- **Caveat on agent-created volume:** as with Neon, a large share of agent-created projects/databases are ephemeral experiments. Investors are explicitly betting they convert to paying workloads; that conversion is not yet proven in public numbers (noted in Implicator coverage, `index`, URL above).

### 2.3 AI/agent factors — the deep part

1. **Being the bundled default inside AI builders is the #1 distribution channel (b).** “Supabase has become the default backend for this shift, with Lovable, Bolt.new, Vercel's v0, Figma Make, and many others building on top of Supabase projects owned by their users” (`verified live`, published 2025-12-05) — https://supabase.com/blog/introducing-supabase-for-platforms . Lovable (~8M users, >100,000 new apps/day, $200M ARR) and Bolt (~5M users) figures in that post are **[vendor-reported partner figures]**. The user never *chooses* a database — the builder chooses for them, at scale.
2. **Supabase for Platforms: provisioning as a product (a)/(b).** White-label backends provisioned/managed entirely via **Management API or remote MCP server**: create projects programmatically, configure Auth/PostgREST/Edge Functions/Storage/Realtime, deploy functions with no CLI, manage API keys, branch, restore points, run migrations via API, Nano instances that scale to zero (`verified live`) — https://supabase.com/blog/introducing-supabase-for-platforms . “Leading AI builders… have already started using Supabase for Platforms to create millions of Supabase projects in the last few months alone” **[vendor claim]**.
3. **Official MCP server — launched Mar 2025, Launch Week 14, alongside declarative schemas (`index`)** — https://supabase.com/changelog?tags=database . Current hosted server, `https://mcp.supabase.com/mcp`, verified live (`verified live`) — https://supabase.com/docs/guides/ai-tools/mcp :
   - *Database:* `list_tables`, `list_extensions`, `list_migrations`, `apply_migration`, `execute_sql`
   - *Debugging:* `query_logs`, `get_advisors` (security/performance)
   - *Development:* `get_project_url`, `get_publishable_keys`, `generate_typescript_types`
   - *Edge Functions:* `list/get/deploy_edge_function`
   - *Account:* `list_projects/get_project/create_project/pause_project/restore_project`, organisations, `get_cost/confirm_cost`
   - *Docs:* `search_docs` (the agent can ground itself in current docs at runtime)
   - *Branching (experimental):* create/list/delete/merge/reset/rebase branch; *Storage* (off by default)
   - *Agent-safety design opensnow should copy:* `read_only=true` (runs as read-only Postgres user), `project_ref=` scoping (drops account tools), `features=` tool-group allow-listing, OAuth with dynamic client registration (no PAT to create/paste), and MCP *elicitations* — cost confirmation before `create_project/create_branch` and confirmation for destructive SQL in `execute_sql/apply_migration`.
4. **Declarative schemas agents can write and version (a).** Schema as `.sql` files in `supabase/schemas/` as the source of truth; CLI (pg-delta diff engine) generates migrations: `supabase db schema declarative sync -f <name> --apply`. Supabase even publishes it as a ready-made **AI Prompt** page instructing agents to use declarative files exclusively and never edit the DB directly (`index`) — https://Supabase.com/docs/guides/ai-tools/ai-prompts/declarative-database-schema . This is idempotent, diffable, git-friendly config — exactly what an agent can generate, a human can review, and CI can apply.
5. **CLI agents can run (a)/(b).** Local stack via CLI (including a local MCP endpoint at `http://localhost:54321/mcp`), migrations, type generation. CLI v3 explicitly considered agent/CI use: unattended hosted changes require `--yes`, remote DB commands require `--remote` (`index`) — https://Supabase.com/changelog?next=Y3Vyc29yOnYyOpK0MjAyMy0xMi0xM1QxOTo0Njo0MlrOAFrpeQ%3D%3D&restPage=3
6. **Docs structured for LLMs (b).** AI Prompts collection in docs; Mar 2026: “docs export as Markdown for AI tools”; docs available over SSH (Apr 2026) (`index`, changelog above). Plus the MCP `search_docs` tool (above) — docs *as a tool*, not just a website.
7. **Agent Skills + Plugin (b).** `supabase/agent-skills` (MIT): `npx skills add supabase/agent-skills`, compatible with 18+ agents (Claude Code, Copilot, Cursor, Cline…), also served at `https://supabase.com/.well-known/agent-skills/`, installable as a Claude Code plugin; skills cover Supabase products + `supabase-postgres-best-practices` (`index`) — https://www.everydev.ai/tools/supabase-agent-skills . June 2026: a combined **Supabase Plugin for AI coding agents** bundling MCP + skills in one install (`index`) — https://supabase.com/changelog?tags=database
8. **pgvector riding the AI wave (a).** pgvector shipped in Supabase projects in Jan 2023 (“pgvector ships for storing OpenAI embeddings”, docs search itself pgvector-powered) — every RAG tutorial written 2023–24 defaulted to “your Postgres, with vectors”, and most named Supabase (`index`, changelog above).
9. **Agent-friendly auth/keys (a).** Publishable + secret API keys designed to be handed to generated apps; MCP OAuth removes token-pasting; Platform APIs let builders hold keys on users' behalf and let users “claim” projects later (`verified live`) — https://supabase.com/blog/introducing-supabase-for-platforms
10. **vs competitors:** Firebase — proprietary/document model, less SQL training data, Google-account/GCP provisioning friction (it has since shipped its own MCP/CLI; Zapier's 2026 comparison still calls Supabase the long-standing vibe-coding default) (`index`) — https://zapier.com/blog/supabase-vs-firebase/ . Neon — purer/faster Postgres provisioning (see §3) but no auth/storage/realtime bundle, so builders needing a *backend*, not just a DB, default to Supabase. PlanetScale — MySQL/Vitess lineage (less Postgres training-data gravity) and branch-centric workflow; no comparable builder-bundle footprint was found in this research (absence noted, not proven).

---

## 3. The general playbook — attractive to agents in 2025–2026

### 3.1 AEO for dev tools: being the answer, honestly assessed
- **What actually drives LLM recommendations today is training-data density + retrieval grounding:** years of public code, tutorials, comparisons and Q&A in your syntax. There is no shortcut file that substitutes for it — which is why familiar syntax (§3.4) and free-tier volume (§3.3) *are* the AEO strategy.
- **llms.txt: ship it, don't believe the hype.** Proposed Sep 2024 by Jeremy Howard; a Markdown index at `/llms.txt` (+ `llms-full.txt` full dump). Google states no AI system uses it for Search/AI Overviews, and an OtterlyAI 90-day study found only 84 of 62,100 AI bot requests (~0.1%) hit llms.txt (`index`) — https://consultusdigital.com/blog/what-is-llms-txt/ . **But** the confirmed consumers are exactly opensnow's audience: coding agents/IDEs (Cursor, Claude Code) fetch it on demand when indexing a docs site (`index`) — https://github.com/howells/agentsurface/blob/HEAD/src/content/docs/discovery/llms-txt.mdx . Stripe, Anthropic, Cloudflare, Vercel, Supabase, MotherDuck all publish one. Cost ≈ zero; do it at launch, write it like MotherDuck's (use-cases, anti-use-cases, agent instructions, pricing, signup endpoint), and *also* serve every docs page as clean Markdown.
- **Structured docs:** one concept per page, executable code block first, explicit error/troubleshooting sections — docs are chunked for RAG (250–500 tokens) whether you design for it or not (`index`) — https://brightjasper.com/blog/structuring-dev-docs-humans-ai

### 3.2 MCP servers as distribution
- **Be where agents discover tools:** Official MCP Registry — https://registry.modelcontextprotocol.io (`index`) — plus Claude/Cursor/VS Code directories, ChatGPT app directory, and skills.sh. Registration is a launch task, not marketing afterthought.
- **What a good data-infra MCP server exposes** (synthesis of Supabase + MotherDuck, both cited above):
  1. *Discover:* list databases/tables/columns, fuzzy catalog search, sample rows, `search_docs`/`ask_docs_question`
  2. *Query:* read-only query tool by default; separate write tool (`query_rw` / `apply_migration`)
  3. *Change safely:* migrations (tracked DDL) distinct from ad-hoc SQL; branching/cloning for agent sandboxes
  4. *Operate:* logs (queryable), advisors/health, cost estimation *before* execution
  5. *Provision:* create/pause/delete project/database (with cost elicitation)
  6. *Guardrails:* read-only mode, project scoping, tool-group allow-lists, OAuth (no static PAT pasting), destructive-action confirmations
- **Hosted remote (Streamable HTTP + OAuth) beats local-only:** zero install for the agent; local stdio variant (`uvx …`) as the developer/sandbox complement. Supabase and MotherDuck both ship both.

### 3.3 Programmatic provisioning & pricing that agents can survive
- **Benchmark: Neon.** “Neon's API creates a new Postgres project in seconds. Every resource (project, branch, role, database, compute) has a REST endpoint, so an agent can create an isolated database, run SQL against it, and delete it in one workflow, with no human approval step. The create-project call returns a working connection string.” Branch = copy-on-write metadata operation; scale-to-zero after 5 min idle **[vendor FAQ]** (`index`) — https://neon.com/faqs/postgres-database-services-ai-provisioning . Neon guides put provisioning at ~500ms–1s (`index`) — https://neon.com/guides/autogen-neon
- **MotherDuck's agent-signup pattern** (§1.3) goes one step further: the *account itself* is API-created, human claims it later. That inverts the Snowflake funnel.
- **Pricing rules for agent workloads:** agents are bursty, exploratory, repetitive and abandon most of what they create. Therefore: permanent free tier (not a 30-day trial), no credit card to start, scale-to-zero / per-query or per-byte pricing (not per-minute minimums), hard spend caps per token/project/agent, and cost returned *in the tool response* so the agent can self-throttle.

### 3.4 Familiar syntax as agent strategy
- Postgres is the most-used database in the Stack Overflow 2025 survey (55.6%, +7pts YoY, cited `index`) — https://www.alibabacloud.com/blog/what-kind-of-postgresql-do-we-need-when-80%25-of-databases-are-automatically-created-by-ai_603524 — and SQLite/Postgres/standard SQL dominate LLM training corpora. A novel dialect taxes every generated query: date functions, identifier casing, JSON operators and string aggregation all diverge (see §4 for Snowflake specifics). **Rule: if opensnow must innovate, innovate behind standard SQL — Postgres-compatible surface, extensions namespaced, never breaking the 95% of queries agents write from memory.**
- Complement with runtime grounding: semantic layer / schema comments / example-query corpus in the MCP server. Secondary text-to-SQL research consistently finds context engineering (examples, metadata) moves accuracy far more than model size (`index`) — https://github.com/phoebefu6/learn-text-to-sql-with-phoebe/blob/HEAD/materials/market-landscape.md

### 3.5 Sandboxes, declarative config, one-line everything
- Local mode with the *identical* engine (DuckDB's whole trick); or instant branch/clone sandboxes (Neon branches, Supabase branching, MotherDuck zero-copy clones) where an agent can DROP/rewite freely.
- One-line install everywhere agents live: `pip install`, `npx`, `uvx`, `brew`, a single binary; a copy-paste MCP config snippet for Claude Code / Cursor / Codex on the docs landing page.
- Declarative, idempotent config: SQL schema files + migrations (Supabase model), infrastructure describable in files an agent writes once and re-applies safely.

### 3.6 Beyond DuckDB/Supabase — documented agent-driven adoption
| Product | Agent mechanic | Evidence |
|---|---|---|
| **Neon** | API-first serverless Postgres, <1s provisioning, branching, scale-to-zero | >80% agent-created DBs; ~$1B Databricks acquisition (`index`) — https://www.aireporteramerica.com/news/7/692/databricks-to-acquire-neon-supercharging-ai-agent-workflows-with-serverless-postgres.html |
| **Convex** | TypeScript-first reactive backend; bundled default in its own builder (Chef); official Cursor plugin + MCP + agent skills | Landscape analysis: builders bundle a default backend (Chef→Convex) as the primary acquisition vector (`index`) — https://chierhu.medium.com/the-backend-as-a-service-and-managed-serverless-database-landscape-eaa16bccd44e ; skills listing (`index`) — https://www.skills.sh/topic/databases |
| **Turso** | libSQL/SQLite — agents already know SQLite; edge replicas; hosted MCP server in Cursor Marketplace, agent skills (`turso-db`) | (`index`) — https://cursor.com/marketplace , https://www.skills.sh/topic/databases |
| **Vercel** | Not a database, but the template: `vercel integration add …` provisions Neon/Supabase/Turso/Convex from CLI/marketplace inside the agent's deploy flow; v0 defaults funnel to Supabase | (`index`) — https://vercel.com/integrations?_rsc=17thc |
| **Databricks (buyer signal)** | Paid ~$1B for Neon *because* of agent telemetry; Feb 2026 State of AI Agents: agents create 80% of databases and 97% of test/dev environments on the platform (as summarised `index`) | https://chierhu.medium.com/the-backend-as-a-service-and-managed-serverless-database-landscape-eaa16bccd44e |

---

## 4. Contrast with Snowflake — why it is agent-hostile today

| Friction | Class | Evidence |
|---|---|---|
| No permanent free tier; trial expires | (c) | “Snowflake does not offer a permanent free tier. It offers a 30-day free trial with $400 in usage credits. No credit card is required… The trial ends when you exhaust the credits or after 30 days” (`index`) — https://tomorrowdesk.com/info/cloud-tiers ; trial terms (`index`) — https://www.snowflake.com/en/snowflake-trial/ . An agent cannot build a durable default on a 30-day clock. *(Correction to the mission premise: self-serve trial signup does exist and needs no card — the hostility is expiry + no free tier + enterprise sales motion for real accounts, not literal sales-only signup.)* |
| No local mode | (a) | No embedded/on-laptop Snowflake engine exists in any source found; the only way to run a query is a provisioned cloud account + warehouse. An agent cannot sandbox, unit-test SQL, or work offline. (Absence established across Snowflake docs searched; flag as “none found”, not a Snowflake admission.) |
| 60-second minimum punishes agent query patterns | (c) | “The minimum billing charge for provisioning compute resources is 1 minute (i.e. 60 seconds)… If a warehouse runs for 61 seconds, shuts down, and then restarts and runs for less than 60 seconds, it is billed for 121 seconds” (`verified live`) — https://docs.snowflake.com/en/user-guide/warehouses-considerations?ref=chaosgenius.io . Agents issue many small, bursty, exploratory queries with idle gaps — the exact pattern that repeatedly triggers resume minimums. Credits are $2–$4 each by edition (on-demand, AWS US) (`index`) — https://www.owox.com/blog/articles/snowflake-pricing-cost-control |
| Immutable signup choices + warehouse management burden | (a) | Cloud provider, region and edition are chosen at signup and “cannot be changed later” (`index`) — https://www.snowflake.com/en/snowflake-trial/ . Warehouse size/auto-suspend/auto-resume tuning is a human ops discipline agents perform badly and expensively. |
| Dialect quirks agents get wrong | (a)/(b) | Uppercase-folded unquoted identifiers (opposite of Postgres), `DATEADD`/`DATEDIFF` argument conventions, reversed `DATE_TRUNC` vs BigQuery, `ILIKE`, `VARIANT` colon-path (`col:field::STRING`), `LATERAL FLATTEN`, `QUALIFY`, `LISTAGG … WITHIN GROUP` — a text-to-SQL engine “needs to know” each of these or it returns errors (`index`) — https://www.basedash.com/blog/text-to-sql-for-data-warehouses-querying-snowflake-bigquery-and-postgresql-in-plain-english . Snowflake-specific agent skills now exist in the community precisely to patch this (`index`) — https://github.com/signalpilot-labs/signalpilot/blob/HEAD/signalpilot-plugin/skills/snowflake-sql/SKILL.md |
| MCP arrived late and narrow | (b) | Snowflake-managed MCP server: preview Oct 2025, **GA 4 Nov 2025** — over a year after MCP's introduction and ~8 months after Supabase's — and the GA release configures it to “serve Cortex Analyst and Cortex Search as tools”: natural-language retrieval over *pre-built semantic models/search indexes inside an existing enterprise account*, with OAuth/RBAC (`verified live`) — https://docs.snowflake.com/en/release-notes/2025/other/2025-11-04-cortex-agents-mcp . It does not, in the GA note, expose project/account provisioning, general schema discovery for a new user, branching/sandboxing, or a free entry point. The earlier open-source Snowflake-Labs MCP server has since been deprecated in favour of the managed endpoint (`index`) — https://chatforest.com/reviews/snowflake-mcp-server/ |
| Cortex is model-gated, rivals are model-agnostic | (a)/(b) | MotherDuck's (self-interested, but structurally accurate) contrast: agents could use a newly released model via standard MCP “the day it ships”, while the same model “wasn't yet available… in Snowflake Cortex” **[vendor claim by competitor]** (`index`) — https://motherduck.com/learn/motherduck-ai-agent-data-layer/ |

Fairness note: Snowflake's MCP/Cortex direction is real — GA managed MCP, Cortex Agents, broad agent-platform integrations (Claude, Cursor, Devin, CrewAI…) were announced Oct–Nov 2025 (`index`) — https://fintechnews.sg/119655/ai/snowflake-cortex-ai-mcp-server-financial-services/ . The gap is *audience*: Cortex serves existing enterprise accounts querying governed data; it does nothing for an agent choosing a warehouse for a new project — the decision opensnow needs to win.

---

## 5. Factor classification & transferability to a cloud data warehouse

| Factor | Class | Transfers to opensnow (cloud DW)? |
|---|---|---|
| No-server local/sandbox engine, same dialect | (a) | **Partially — highest priority.** Ship a local/embedded or instant-ephemeral mode on the same engine. This is the MotherDuck/DuckDB bridge pattern. |
| Query files in place (Parquet/CSV/JSON/Iceberg), no load step | (a) | Yes, fully. Zero-ETL first query is an agent's first success. |
| Postgres/standard-SQL surface | (a) | Yes, fully. Non-negotiable. |
| Vectorized/fast small-query latency (<1s cold) | (a) | Yes. Agents abandon slow tool loops; 100ms-class spin-up is now the marketed bar (MotherDuck **[vendor claim]**). |
| Per-agent isolated compute + hard cost cap | (a)/(c) | Yes — hypertenancy or equivalent; an agent must be unable to create a bill spike. |
| Permanent free tier, no card | (c) | Yes, fully. A trial is not a tier. |
| Per-query/per-byte pricing, scale-to-zero, no minimums | (c) | Yes, fully. Explicitly anti-60s-minimum. |
| MIT/open-source core | (c)/(b) | Optional but powerful for trust + training-data generation; at minimum open client, docs, skills and MCP server. |
| Hosted MCP + local MCP, OAuth, read-only default | (b)/(a) | Yes, fully, at launch. |
| Agent skills + plugin for Claude Code/Cursor/Codex | (b) | Yes, fully, at launch. Cheap to produce, compounding. |
| llms.txt / llms-full.txt / markdown docs / docs-as-tool | (b) | Yes, fully, at launch. |
| Management API for every dashboard action | (a)/(b) | Yes, fully. If a human must click it, an agent can't do it. |
| Bundled default in AI builders/platforms | (b) | Yes — partnerships (Lovable/Bolt/v0/Replit/…) + platform-provisioning API (Supabase for Platforms pattern). Later than launch, but design the API for it from day one. |
| Batteries-included bundle (auth/storage/etc.) | (a) | Partially — opensnow is a warehouse, not a BaaS; bundle the warehouse equivalents: ingestion, semantic layer, sharing, BI-ready outputs. |
| Community/Launch Week/YC network effects | (b) | Slowly; cannot be faked at launch, but free tier + open artefacts seed it. |

---

## 6. Checklist — “Agent-first from day one” for opensnow

### Must-have at launch
**Architecture (a)**
- [ ] Postgres-compatible / standard-SQL dialect; publish a dialect page listing every deviation (aim: none in the common 95%).
- [ ] Local/sandbox mode: same engine, `pip install opensnow` / single binary / Wasm, `:memory:` + local files, zero account needed.
- [ ] Query Parquet/CSV/JSON (and Iceberg) in place by path/URL — first query requires no CREATE TABLE, no load, no warehouse sizing.
- [ ] Programmatic provisioning API for *everything*: POST creates project/database and returns a connection string + scoped token in <5s (target <1s); every dashboard action has an API/CLI equivalent.
- [ ] Agent sandboxes: instant branch / zero-copy clone per agent or per task, deletable via API, isolated compute.
- [ ] Per-agent/per-token hard cost caps and query limits enforced by the platform (not by prompting); cost estimate before execution and actual cost in every query response.
- [ ] Scoped, short-lived, revocable tokens an agent can mint/rotate itself; OAuth flow for MCP (no static key pasting); read-only role as the default.

**Pricing/packaging (c)**
- [ ] Permanent free tier, no credit card: enough storage + compute for a real prototype (benchmarks: MotherDuck 10 GB / 10 compute-hrs/mo; Supabase 500 MB / 2 projects — for a warehouse, follow MotherDuck/BigQuery scale, not Supabase's).
- [ ] No billing minimums. Per-query or per-byte-scanned pricing, scale-to-zero, $0 when idle. State this in one sentence on the pricing page — agents (and LLMs summarising you) will quote it against Snowflake's 60s minimum.
- [ ] Agent signup endpoint: API creates a free org and returns token + claim URL for a human to take ownership later (MotherDuck pattern).

**Distribution/docs (b)**
- [ ] Hosted remote MCP server (Streamable HTTP, OAuth) **and** local stdio MCP (`uvx …`), exposing: catalog/table discovery, schema+sample rows, read-only query, separate write/migration tools, logs, cost, docs search. Read-only default, project scoping, tool-group allow-lists, destructive-action confirmation.
- [ ] Registered in the Official MCP Registry + Claude/Cursor/VS Code directories on launch day.
- [ ] Agent skills repo (MIT): `npx skills add opensnow/agent-skills`, SKILL.md for SQL best practices + opensnow specifics, `.well-known/agent-skills/` discovery, Claude Code plugin bundling MCP + skills in one install.
- [ ] `/llms.txt` written *to a recommending agent* (when to use / when NOT to use, free tier, MCP endpoint, signup endpoint, pricing in plain text) + `/llms-full.txt`; every docs page available as Markdown (`.md` suffix or Accept header); OpenAPI spec published.
- [ ] Docs landing page leads with copy-paste MCP configs for Claude Code, Cursor and Codex, and a 3-line “first query in 60 seconds” path. Public example repos/templates in idiomatic opensnow SQL — this is tomorrow's training data.
- [ ] AI Prompts / agent instruction pages (Supabase pattern): declarative schema workflow, migration workflow, debugging workflow — as pages an agent can be pointed at.

### Later (post-launch, in order)
- [ ] Platform/white-label provisioning (Supabase for Platforms pattern) + partnerships to be the bundled default in Lovable/Bolt/v0/Replit-class builders and Vercel/Netlify marketplaces (`integration add` flows).
- [ ] Semantic-layer / business-context tooling in MCP (metric definitions, example queries, schema comments) — the proven text-to-SQL accuracy lever on large schemas.
- [ ] Branching GA with merge/reset, restore points per agent change, ephemeral per-PR/per-task warehouses in CI.
- [ ] Fine-tuned / assisted text-to-SQL for opensnow dialect (duckdb-nsql pattern) and an in-product AI assistant — only after MCP + context tooling, which help every external model too.
- [ ] Usage telemetry + public reporting on agent-created share (the Neon/Supabase stat is itself marketing); startup/builder credit programmes.
- [ ] Enterprise layer agents still respect: RBAC-scoped MCP, audit logs of every agent tool call, SOC 2 — sold as “safe to give an agent write access”, not as a gate on getting started.

---

## Could not verify / open questions

- **Supabase revenue:** company does not disclose; third-party ARR estimates range ~$27M–$170M depending on source/date. No single figure is reliable.
- **DuckDB Labs funding:** a third-party dossier's $87M Series B claim is contradicted by Wikipedia/DuckLabs' stated no-VC model; treated as erroneous. DuckLabs' AWS acquisition (Aug 2026) is reported in Wikipedia/secondary sources; terms undisclosed.
- **DuckDB core MCP:** the widely used DuckDB MCP server is MotherDuck's (`mcp-server-motherduck`, local + remote). Claims in secondary rankings that “DuckDB v1.5 added a built-in MCP server” were not confirmed on a DuckDB primary page in this research — do not repeat without checking duckdb.org.
- **Snowflake “sales-only provisioning”:** the mission premise is overstated for trials — self-serve $400/30-day trial signup exists with no card. The agent-hostility case rests on trial expiry, no free tier, no local mode, billing minimums and MCP scope/timing, all sourced above.
- **PlanetScale agent positioning:** no comparable agent-adoption stat or builder-bundle evidence was found; the comparison in §2.3 is an absence-of-evidence note, not proof of absence.
- **MotherDuck/Supabase performance and partner figures** (100ms spin-up, Lovable/Bolt ARR and user counts, per-query metering advantages) are vendor-reported and flagged as such where used.
- No live hands-on test was performed (no installs, signups or MCP connections — research was read-only web research, per boundaries).

## Sources

Primary / verified live (opened 2026-11-05):
- DuckDB, Why DuckDB — https://duckdb.org/why_duckdb
- DuckDB, Thank You for 40,000 Stars — https://duckdb.org/2026/08/05/github-40k-stars
- Supabase, MCP Server docs — https://supabase.com/docs/guides/ai-tools/mcp
- Supabase, Introducing Supabase for Platforms — https://supabase.com/blog/introducing-supabase-for-platforms
- MotherDuck, llms.txt — https://motherduck.com/llms.txt
- Snowflake, Managed MCP Server GA release note (2025-11-04) — https://docs.snowflake.com/en/release-notes/2025/other/2025-11-04-cortex-agents-mcp
- Snowflake, Warehouse considerations (60s minimum) — https://docs.snowflake.com/en/user-guide/warehouses-considerations?ref=chaosgenius.io

Index (search-result text, 2026-11-05) — key items:
- DuckDB 30k stars — https://duckdb.org/2025/06/06/github-30k-stars ; History (UI/Quack) — https://duckdb.org/history/ ; Quack in 1.5.3 — https://duckdb.org/2026/05/20/announcing-duckdb-153
- DuckDB (Wikipedia; no-VC, MotherDuck $100M, AWS/DuckLabs) — https://en.wikipedia.org/wiki/DuckDB
- MotherDuck Series B / $100M total — https://motherduck.com/blog/motherduck-open-for-all-with-series-b/ ; MCP PyPI — https://pypi.org/project/mcp-server-motherduck/0.6.3/ ; AI agent data layer — https://motherduck.com/learn/motherduck-ai-agent-data-layer/ ; LangChain agent — https://motherduck.com/blog/langchain-sql-agent-duckdb-motherduck/
- Supabase Series E PR — https://pr.wvcjournal.com/article/Supabase-Raises-dollar100M-at-dollar5B-Valuation-Co-Led-by-Accel-and-Peak-XV/68dfad893ce2ea39bbf99d0d ; Series F / agent stats — https://www.implicator.ai/supabase-doubles-to-10-5-billion-as-agents-deploy-most-of-its-databases/ ; changelog (MCP Mar 2025, declarative schemas, markdown docs, plugin) — https://supabase.com/changelog?tags=database ; declarative schema AI prompt — https://Supabase.com/docs/guides/ai-tools/ai-prompts/declarative-database-schema ; agent skills — https://www.everydev.ai/tools/supabase-agent-skills ; pricing/free tier — https://makerkit.dev/blog/saas/supabase-pricing , https://freetier.wiki/services/supabase/
- Neon/Databricks 80% — https://www.aireporteramerica.com/news/7/692/databricks-to-acquire-neon-supercharging-ai-agent-workflows-with-serverless-postgres.html ; Neon provisioning FAQ — https://neon.com/faqs/postgres-database-services-ai-provisioning ; Neon agent guides — https://neon.com/guides/autogen-neon
- Snowflake trial/no free tier — https://www.snowflake.com/en/snowflake-trial/ , https://tomorrowdesk.com/info/cloud-tiers ; credit pricing — https://www.owox.com/blog/articles/snowflake-pricing-cost-control ; dialect/text-to-SQL — https://www.basedash.com/blog/text-to-sql-for-data-warehouses-querying-snowflake-bigquery-and-postgresql-in-plain-english ; Spider 2.0 scores — https://github.com/phoebefu6/learn-text-to-sql-with-phoebe/blob/HEAD/materials/market-landscape.md
- llms.txt evidence — https://consultusdigital.com/blog/what-is-llms-txt/ , https://github.com/howells/agentsurface/blob/HEAD/src/content/docs/discovery/llms-txt.mdx ; MCP Registry — https://registry.modelcontextprotocol.io ; BaaS landscape (Convex/Turso/Vercel defaults) — https://chierhu.medium.com/the-backend-as-a-service-and-managed-serverless-database-landscape-eaa16bccd44e
