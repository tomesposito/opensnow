# B3 Dialect Spike: DataFusion vs Postgres and Snowflake SQL

## Question

D-02 says the opensnow core dialect is Postgres-compatible, with a Snowflake compatibility layer on top. The engine is our own build on DataFusion. This spike measures how much of each dialect DataFusion already parses and executes, so the size of the compat layer is a measurement and not a guess.

## Setup

- DataFusion 50.3.0, sqlparser 0.58.0, Arrow 56.2.1, Tokio 1.54.1.
- Rust 1.99.1, installed with rustup inside this spike dir.
- Runner: `runner/` (binary `df-dialect-runner`). It takes a dialect name and one or more SQL files, and prints one JSON record per statement.
- Demo data: two in-memory tables registered fresh for every statement, so DML tests cannot pollute each other. `employees` (id, name, dept, salary, hire_date, active), `events` (id, user_id, payload as JSON text, amount, created_at, tags), and `users` as a second copy of `employees`.
- Two layers measured per statement. Layer 1 parses with sqlparser alone (`Parser::parse_sql`). Layer 2 plans and executes with a DataFusion `SessionContext` whose `sql_parser.dialect` config matches layer 1.
- Test set: 37 statements in `sql/01-postgres.sql`, `sql/02-snowflake.sql`, `sql/03-standard.sql`. Raw output in `results/generic.jsonl` and `results/postgres.jsonl`.
- Rewrite checks: `sql/04-workarounds.sql` and `sql/05-workarounds2.sql`, output in `results/workarounds.jsonl` plus the console run for file 05. A workaround below is only called verified if its rewrite target ran green there.
- Caveat: the demo tables are MemTables. The DML failures below are planner failures. DataFusion rejects the statement shape before table writability is even a question. A real opensnow table provider does not fix them for free; it only changes where the fix would live.

## Headline numbers

| Group | Statements | Parse (Generic) | Parse (PostgreSql) | Execute (either dialect) |
|---|---|---|---|---|
| Postgres idioms | 13 | 13/13 (100%) | 13/13 (100%) | 10/13 (77%) |
| Snowflake idioms | 15 | 13/15 (87%) | 12/15 (80%) | 4/15 (27%) |
| Standard SQL | 9 | 9/9 (100%) | 9/9 (100%) | 9/9 (100%) |
| Total | 37 | 35/37 (95%) | 34/37 (92%) | 23/37 (62%) |

Three facts carry the story.

1. Standard SQL is done. All 9 pass end to end, including window functions, a CTE, a correlated subquery in WHERE, and EXISTS.
2. Postgres idioms mostly work. All 13 parse in both dialects and 10 execute as written. The 3 failures are all write-path or lateral-join planner limits, and none of them is a parse problem.
3. Snowflake idioms parse far better than they run. 13 of 15 parse under the Generic dialect, and several parse into real AST nodes (QUALIFY, PIVOT, CLONE). Only 4 execute. The gap is mostly missing functions and missing planner features, and 7 of the 11 failures have a rewrite that this spike verified executes.

## Dialect setting: Generic vs PostgreSql

The dialect flag changes almost nothing, and what it changes is slightly negative on this set.

- Execution results are identical. Same 23 passes, same failures.
- The PostgreSql dialect parses one fewer statement. `CREATE TABLE ... CLUSTER BY` parses under Generic (and then fails in the planner) and fails at parse time under PostgreSql.
- Two error messages differ (FLATTEN named arguments, ARRAY_CONSTRUCT suggestion), with no change in outcome.

Practical read: setting DataFusion to its PostgreSql dialect does not deliver the Postgres core that D-02 wants. The Postgres behaviour that works here (ILIKE, `::`, date_trunc, DISTINCT ON, FILTER, generate_series) works because DataFusion and sqlparser built it in, and it works under Generic too. The compat layer should parse with Generic, or with its own Snowflake-aware pre-pass, and treat dialect as an opensnow layer. The dialect flag is not a strategy.

## Group 1: Postgres idioms (13)

| ID | Statement shape | Parse | Execute | Error class | Workaround for opensnow |
|---|---|---|---|---|---|
| PG01 | `WHERE name ILIKE 'a%'` | yes | yes | ok | none |
| PG02 | `salary::varchar` | yes | yes | ok | none |
| PG03 | `date_trunc('month', hire_date)` | yes | yes | ok | none |
| PG04 | `generate_series(1, 5)` as table function | yes | yes | ok | none |
| PG05 | `INSERT ... RETURNING id` | yes | no | unsupported (planner: "Insert-returning clause not supported") | No SQL rewrite. Compat layer can split the statement at the service layer (run the insert, then select the keys back) for writable tables, or accept the gap in v1. |
| PG06 | `INSERT ... ON CONFLICT (id) DO NOTHING` | yes | no | unsupported (planner: "Insert-on clause not supported") | No SQL rewrite. Service layer upsert semantics, or a two-statement rewrite (insert with an anti-join against existing keys) once opensnow owns writes. |
| PG07 | `ARRAY[1, 2, 3]` literal | yes | yes | ok | none |
| PG08 | `unnest(ARRAY[1, 2, 3])` | yes | yes | ok | none |
| PG09 | Correlated `LATERAL` join (count per outer row) | yes | no | unsupported (physical plan: OuterReferenceColumn not supported) | Rewrite rule for the common pattern: pre-aggregate the inner side by the join key and join that. Correlated subqueries in WHERE do work (see ST07), so the planner handles some correlation; lateral joins are the specific hole. |
| PG10 | `DISTINCT ON (dept) ... ORDER BY dept, salary DESC` | yes | yes | ok | none |
| PG11 | `COUNT(*) FILTER (WHERE active)` | yes | yes | ok | none |
| PG12 | `name \|\| '-' \|\| dept` | yes | yes | ok | none |
| PG13 | `EXTRACT(EPOCH FROM created_at)` | yes | yes | ok | none |

Notes:

- DISTINCT ON and FILTER working out of the box is better than expected. Both are idioms agents reach for often.
- The two write failures matter for agent-generated code, since agents generate upserts and RETURNING a lot. Both belong on the opensnow roadmap at the service layer. Neither blocks read workloads, which are the warehouse core.

## Group 2: Snowflake idioms (15)

| ID | Statement shape | Parse (Generic / PG) | Execute | Error class | Workaround for opensnow |
|---|---|---|---|---|---|
| SF01 | `QUALIFY rn = 1` over ROW_NUMBER | yes / yes | yes | ok | none. sqlparser parses QUALIFY and the DataFusion planner executes it. |
| SF02 | `LATERAL FLATTEN(input => payload)` | yes / yes | no | unsupported (no flatten table function; named-argument form rejected in the PG run) | Partial rewrite plus a UDF. Array inputs rewrite to `unnest` (verified, W06). JSON and variant inputs first need a get_path or parse step that turns the value into an array. |
| SF03 | `payload:a` colon path | no / no | no | parse ("Expected: end of statement, found: :") | Parser work. Either extend the parser, or pre-parse rewrite `col:path:field` into a `get_path(col, 'path.field')` call. The rewrite must not touch `::` casts or time literals. This is the signature semi-structured idiom, so the layer has to own it. |
| SF04 | `DATEADD(day, 7, hire_date)` | yes / yes | no | unsupported (invalid function dateadd) | Rewrite rule, verified: `DATEADD(unit, n, d)` becomes `d + INTERVAL 'n' unit` (W01). Argument order flips and units need a mapping table. |
| SF05 | `DATEDIFF(day, hire_date, d)` | yes / yes | no | unsupported (invalid function datediff; date_diff is also absent in 50.3) | Rewrite rules, verified for day: date subtraction (W09) or epoch math (W10). Snowflake counts unit boundaries crossed, so other units need per-unit rules built on date_trunc, not one formula. |
| SF06 | `CREATE TABLE t CLONE src` | yes / yes | no | unsupported (planner: "Clone not supported") | Not a SQL problem. opensnow implements CLONE in the catalog over Iceberg snapshots (spike B2 territory). Accept the gap inside DataFusion. |
| SF07 | `SELECT ... FROM t AT (TIMESTAMP => ts)` | no / no | no | parse ("Expected: ), found: =>") | Parser plus provider work. Pre-parse rewrite to a snapshot selection the Iceberg table provider understands, or an opensnow syntax extension. Hard at the parser level, medium at the rewrite plus provider level. |
| SF08 | `SELECT ... SAMPLE (50)` | yes / yes | yes | ok | none. Semantics are close enough at this size; Snowflake percent sampling and DataFusion sampling are not bit-identical. |
| SF09 | `WHERE dept ILIKE 'ENG'` | yes / yes | yes | ok | none |
| SF10 | `CREATE TABLE ... CLUSTER BY (d)` | yes / no | no | unsupported under Generic (planner: "Cluster by not supported"); parse failure under PostgreSql | Accept and strip. CLUSTER BY is a physical layout hint. The compat layer can drop it with a log line, and the storage layer owns layout through Iceberg partitioning and sort order. |
| SF11 | `OBJECT_CONSTRUCT('a', 1, 'b', 2)` | yes / yes | no | unsupported (invalid function object_construct) | Rewrite rule, verified: `named_struct` produces the value (W03). Struct is not JSON, so a variant column boundary still needs a wrap or cast at ingest. |
| SF12 | `ARRAY_CONSTRUCT(1, 2, 3)` | yes / yes | no | unsupported (invalid function array_construct) | Rewrite rule, verified: `make_array` (W04), or the `ARRAY[...]` literal. |
| SF13 | `TRY_CAST('not-a-number' AS INT)` | yes / yes | yes | ok | none. Returns NULL, which matches Snowflake. |
| SF14 | `DIV0(10, 0)` | yes / yes | no | unsupported (invalid function div0) | Two options, one verified: rewrite to `a / NULLIF(b, 0)` (W05), or register a small `div0` UDF. The UDF is less code. |
| SF15 | `PIVOT (SUM(salary) FOR dept IN (...))` | yes / yes | no | unsupported (planner: "Unsupported ast node Pivot") | Rewrite rule, verified for the common static form: conditional aggregation, `SUM(CASE WHEN ...)` per IN value (W07). Dynamic pivot lists stay a gap. |

Notes:

- QUALIFY passing is the single best surprise in this spike. It is a top-five Snowflake idiom in real workloads and it costs the compat layer nothing.
- TRY_CAST and SAMPLE also pass as written. With ILIKE, 4 of 15 Snowflake statements need no work at all.
- Only 2 Snowflake statements fail at parse time under Generic (colon path, time travel). The rest of the work sits in the planner and function layers, which are cheaper places to fix things than the parser.

## Group 3: Standard SQL (9)

| ID | Statement shape | Parse | Execute | Error class | Workaround |
|---|---|---|---|---|---|
| ST01 | Join with GROUP BY and AVG | yes | yes | ok | none |
| ST02 | CTE | yes | yes | ok | none |
| ST03 | ROW_NUMBER window | yes | yes | ok | none |
| ST04 | SUM OVER (PARTITION BY) | yes | yes | ok | none |
| ST05 | CASE bands | yes | yes | ok | none |
| ST06 | UNION across two tables | yes | yes | ok | none |
| ST07 | Correlated subquery in WHERE | yes | yes | ok | none |
| ST08 | GROUP BY with HAVING, ORDER BY, LIMIT | yes | yes | ok | none |
| ST09 | EXISTS subquery | yes | yes | ok | none |

No notes needed. This is the floor the product stands on, and it is solid.

## Verified rewrites

Each row ran green in the runner under the Generic dialect. These are the load-bearing claims behind the workaround column above.

| ID | Rewrite | Proves |
|---|---|---|
| W01 | `hire_date + INTERVAL '7' DAY` | DATEADD target form executes |
| W03 | `named_struct('a', 1, 'b', 2)` | OBJECT_CONSTRUCT target form executes |
| W04 | `make_array(1, 2, 3)` | ARRAY_CONSTRUCT target form executes |
| W05 | `10 / NULLIF(0, 0)` | DIV0 target form executes |
| W06 | `unnest(make_array(10, 20))` | FLATTEN target form executes for array inputs |
| W07 | `SUM(CASE WHEN dept = 'eng' THEN salary END)` per pivot value | PIVOT target form executes for static IN lists |
| W08 | Subquery plus `WHERE rn = 1` | The manual QUALIFY expansion works (moot, since QUALIFY is native) |
| W09 | `DATE '2025-01-01' - hire_date` | DATEDIFF day target form executes (interval result) |
| W10 | `(EPOCH(b) - EPOCH(a)) / 86400` | DATEDIFF day target form executes (numeric result) |
| W02 | `date_diff('day', a, b)` | Failed. date_diff does not exist in DataFusion 50.3 either. Do not plan on it. |

## The compat layer, sized

Four buckets, in build order.

### 1. AST rewrite rules (sqlparser AST in, AST out)

Each rule is small, and each target form is verified above unless marked otherwise.

- DATEADD to interval addition, with a unit mapping table and the argument flip.
- DATEDIFF to per-unit rules. Day is verified two ways. Month, quarter, and year need boundary-count rules on date_trunc, which follow Snowflake semantics and need their own tests.
- ARRAY_CONSTRUCT to make_array.
- OBJECT_CONSTRUCT to named_struct, with a variant wrap at the column boundary.
- PIVOT with a static IN list to conditional aggregation.
- FLATTEN over an array expression to unnest.
- LATERAL aggregate-per-row pattern to a pre-aggregated join. Pattern is identified from PG09 and ST07; the rule itself is not yet written or verified.
- CLUSTER BY clause strip, with a log line.

### 2. UDFs and JSON helpers to register

- `div0` (or keep the NULLIF rewrite; pick one).
- `get_path(value, 'a.b.c')` for semi-structured access. This one UDF is the target for both the colon path rewrite and the JSON side of FLATTEN, so it is the highest-value single build in this layer.
- A parse_json style constructor that turns JSON text into the value get_path walks, if the storage layer does not already hand over a native semi-structured type.

### 3. Pre-parse and parser work

- Colon path access (`col:a:b`). Text-level rewrite is risky near casts and literals; an AST-level fix needs parser support first, since the token stream never becomes an AST today. Budget this as real parser work.
- Time travel `AT (TIMESTAMP => ...)` and `BEFORE`. Same shape of problem, plus it needs the table provider to accept a snapshot choice.
- Snowflake named-argument syntax (`input => value`) in general. FLATTEN uses it; other functions will too.

### 4. Service and catalog layer, outside the SQL engine

- CLONE, as a catalog operation over Iceberg snapshots.
- Time travel snapshot resolution, once the parser hands over the timestamp.
- RETURNING and ON CONFLICT for writable tables, as statement splitting or provider-level write semantics.

### Accepted gaps for v1

- Dynamic PIVOT (value list read from data).
- FLATTEN over arbitrary variant paths, until get_path lands.
- CLUSTER BY as a user-facing tuning knob.
- Snowflake DDL surface beyond the tested shapes (warehouses, stages, file formats, tasks). This spike tested query idioms and two DDL shapes only; the DDL surface is a separate, larger inventory.

## Hard gaps

Ranked by pain, where pain means common in real Snowflake code and hard to fix.

1. Colon path access. Fails at parse, sits in a large share of semi-structured Snowflake queries, and needs parser work plus the get_path UDF.
2. Time travel AT and BEFORE. Fails at parse, and the fix spans parser, rewrite, and table provider. Time travel is also a headline Snowflake feature, so users will ask early.
3. CLONE. Parses and dies in the planner. The fix is a catalog feature, not a compat fix, and it is core Snowflake vocabulary.
4. RETURNING and ON CONFLICT. Postgres side, planner level. Agent-generated write code uses both constantly. Reads are unaffected, which is the only reason this ranks below the top three.
5. Correlated LATERAL joins. Planner and physical plan limit. The pre-aggregation rewrite covers the common aggregate pattern and nothing else yet.
6. PIVOT. Planner limit with a verified rewrite for the static case. Dynamic pivot is the remaining gap.

## What this means for D-02

The Postgres core is real and close to free: 100% parse, 77% execute, and the failures cluster on the write path. The Snowflake layer is a medium build, not a research project: most failures are one verified rewrite or one UDF away, and only two idioms need parser work. The two parser items, colon paths and time travel, are also the two most distinctive Snowflake behaviours in the set, so they set the floor on compat layer effort. Plan the layer as rewrites first, UDFs second, parser work as its own scheduled piece, and CLONE plus write semantics in the catalog and service layers where they belong.

## Files

- `runner/` Rust runner (Cargo.toml, src/main.rs, Cargo.lock). Build with the rustup and cargo homes inside this dir.
- `sql/01-postgres.sql`, `sql/02-snowflake.sql`, `sql/03-standard.sql`, `sql/04-workarounds.sql`, `sql/05-workarounds2.sql`.
- `results/generic.jsonl`, `results/postgres.jsonl`, `results/workarounds.jsonl`: one JSON record per statement with parse result, execute result, row count, error class, and error text.
- Toolchain dirs `rustup/` and `cargo/` are local to this spike and safe to delete. The build is large; a cold release build of DataFusion 50.3 on this shared VM took roughly an hour wall clock while two sibling spikes compiled at the same time.
