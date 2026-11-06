# SQL Compatibility List v1

This is the D-02 list. The core dialect is Postgres-compatible. A Snowflake compatibility layer sits on top. This document says which statements run, which run after a rewrite in that layer, and which do not run yet.

The source is spike B3 (spikes/B3-dialect/FINDINGS.md). The spike ran 37 statements against DataFusion 50.3.0 with sqlparser 0.58.0. Each statement ran against fresh in-memory tables. Every statement from the spike appears below with the status the spike measured. Nothing here claims a result the spike did not verify.

Two facts from the spike shape this list:

- The dialect setting barely matters. Results under the Generic and PostgreSql parser dialects were the same at execution time. The compat layer parses with Generic and owns dialect behaviour itself.
- Standard SQL is solid, the Postgres core is close, and the Snowflake layer is a set of small fixes. Of 37 statements, 23 run as written, 7 run after a verified rewrite, and 7 sit in the named gaps.

## 1. Runs as written

These statements parse and execute on DataFusion 50.3.0 with no rewrite.

### Postgres core (10 of 13)

```sql
-- PG01  ILIKE
SELECT name FROM employees WHERE name ILIKE 'a%';

-- PG02  cast operator
SELECT id, salary::varchar AS salary_text FROM employees;

-- PG03  date_trunc
SELECT date_trunc('month', hire_date) AS month_start FROM employees;

-- PG04  generate_series as a table function
SELECT value FROM generate_series(1, 5) AS t(value);

-- PG07  array literal
SELECT ARRAY[1, 2, 3] AS a FROM employees LIMIT 1;

-- PG08  unnest
SELECT unnest(ARRAY[1, 2, 3]) AS v;

-- PG10  DISTINCT ON
SELECT DISTINCT ON (dept) dept, name, salary FROM employees ORDER BY dept, salary DESC;

-- PG11  FILTER (WHERE ...)
SELECT COUNT(*) FILTER (WHERE active) AS active_count FROM employees;

-- PG12  string concatenation operator
SELECT name || '-' || dept AS label FROM employees;

-- PG13  EXTRACT EPOCH
SELECT EXTRACT(EPOCH FROM created_at) AS epoch_s FROM events;
```

DISTINCT ON and FILTER passing as written is better than expected. Agents reach for both often.

### Standard SQL (9 of 9)

```sql
-- ST01  join with GROUP BY and AVG
SELECT e.dept, COUNT(*) AS n, AVG(e.salary) AS avg_salary FROM employees e GROUP BY e.dept ORDER BY e.dept;

-- ST02  CTE
WITH top_paid AS (SELECT * FROM employees WHERE salary > 75) SELECT COUNT(*) AS n FROM top_paid;

-- ST03  ROW_NUMBER window function
SELECT name, ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn FROM employees;

-- ST04  SUM OVER (PARTITION BY ...)
SELECT name, dept, SUM(salary) OVER (PARTITION BY dept) AS dept_total FROM employees;

-- ST05  CASE bands
SELECT name, CASE WHEN salary >= 90 THEN 'high' WHEN salary >= 70 THEN 'mid' ELSE 'low' END AS band FROM employees;

-- ST06  UNION
SELECT dept FROM employees UNION SELECT dept FROM users;

-- ST07  correlated subquery in WHERE
SELECT name FROM employees e WHERE salary > (SELECT AVG(salary) FROM employees WHERE dept = e.dept);

-- ST08  GROUP BY with HAVING, ORDER BY, LIMIT
SELECT dept, COUNT(*) AS n FROM employees GROUP BY dept HAVING COUNT(*) > 1 ORDER BY n DESC LIMIT 2;

-- ST09  EXISTS subquery
SELECT name FROM employees e WHERE EXISTS (SELECT 1 FROM events ev WHERE ev.user_id = e.id);
```

This is the floor the product stands on. It holds.

### Snowflake idioms that pass (4 of 15)

```sql
-- SF01  QUALIFY
SELECT id, name, ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn FROM employees QUALIFY rn = 1;

-- SF08  SAMPLE
SELECT * FROM employees SAMPLE (50);

-- SF09  ILIKE
SELECT name FROM employees WHERE dept ILIKE 'ENG';

-- SF13  TRY_CAST
SELECT TRY_CAST('not-a-number' AS INT) AS n;
```

QUALIFY is the best surprise in the spike. It is a top Snowflake idiom and it costs the compat layer nothing. TRY_CAST returns NULL on failure, which matches Snowflake. SAMPLE runs, with one caveat: Snowflake percent sampling and DataFusion sampling are close in meaning, not bit-identical.

## 2. Runs with a compat rewrite

Each pair below shows the statement as a user or agent writes it, then the form the compat layer turns it into. The spike ran each rewritten form and it executed. The rewrite target IDs (W01 and so on) are the spike's own test IDs in spikes/B3-dialect/sql/04-workarounds.sql and 05-workarounds2.sql.

### SF04  DATEADD

```sql
-- as written (fails: no dateadd function)
SELECT DATEADD(day, 7, hire_date) AS plus_week FROM employees;

-- rewrite (W01, verified)
SELECT hire_date + INTERVAL '7' DAY AS plus_week FROM employees;
```

The argument order flips and the unit needs a mapping table. The spike verified the day form above. Other units follow the same shape and still need their own tests.

### SF05  DATEDIFF

```sql
-- as written (fails: no datediff function)
SELECT DATEDIFF(day, hire_date, DATE '2025-01-01') AS days_diff FROM employees;

-- rewrite, interval form (W09, verified)
SELECT DATE '2025-01-01' - hire_date AS diff FROM employees;

-- rewrite, numeric form (W10, verified)
SELECT (EXTRACT(EPOCH FROM DATE '2025-01-01') - EXTRACT(EPOCH FROM hire_date)) / 86400 AS days_diff FROM employees;
```

Both rewrites are verified for the day unit only. Snowflake counts unit boundaries crossed, so month, quarter, and year need per-unit rules built on date_trunc. Those rules are planned, not verified. One warning from the spike: `date_diff` does not exist in DataFusion 50.3.0 either, so no rewrite may lean on it.

### SF12  ARRAY_CONSTRUCT

```sql
-- as written (fails: no array_construct function)
SELECT ARRAY_CONSTRUCT(1, 2, 3) AS arr;

-- rewrite (W04, verified)
SELECT make_array(1, 2, 3) AS arr;
```

The `ARRAY[...]` literal form also works; it is PG07 in section 1.

### SF11  OBJECT_CONSTRUCT

```sql
-- as written (fails: no object_construct function)
SELECT OBJECT_CONSTRUCT('a', 1, 'b', 2) AS obj;

-- rewrite (W03, verified)
SELECT named_struct('a', 1, 'b', 2) AS obj;
```

One caveat: named_struct produces a struct, not JSON. Where a value lands in a variant column, the layer still needs a wrap or cast at that boundary.

### SF14  DIV0

```sql
-- as written (fails: no div0 function)
SELECT DIV0(10, 0) AS safe_div;

-- rewrite (W05, verified)
SELECT 10 / NULLIF(0, 0) AS safe_div;
```

The spike also notes a small `div0` UDF as an alternative. The rewrite above is the verified path.

### SF02  FLATTEN, array inputs only

```sql
-- as written (fails: no flatten table function)
SELECT f.value FROM events e, LATERAL FLATTEN(input => e.payload) f;

-- rewrite pattern, for array inputs (W06, verified)
SELECT unnest(make_array(10, 20)) AS value;
```

The verified claim is narrow: FLATTEN over an array rewrites to unnest. In the spike data the payload column holds JSON text, and FLATTEN over JSON or variant values does not have a verified rewrite. That case first needs a get_path or parse step that turns the value into an array. Until that lands, it belongs with the gaps in section 3.

### SF15  PIVOT, static value lists only

```sql
-- as written (fails: planner rejects the Pivot node)
SELECT * FROM employees PIVOT (SUM(salary) FOR dept IN ('eng', 'sales'));

-- rewrite (W07, verified)
SELECT SUM(CASE WHEN dept = 'eng' THEN salary END) AS eng,
       SUM(CASE WHEN dept = 'sales' THEN salary END) AS sales
FROM employees;
```

The rewrite works when the IN list is written in the query. A pivot whose value list comes from the data is dynamic PIVOT, and it is a gap in section 3.

## 3. Named gaps

These do not run in v1. Each gap names where the fix lives. The counts: 6 of the 37 statements sit here in full (SF03, SF07, SF06, PG05, PG06, PG09), plus one DDL statement (SF10) and the dynamic half of SF15, which section 2 covers in its static form.

### SF03  Colon path access

```sql
SELECT payload:a AS a_val FROM events;
```

Fails at parse time. This is the signature semi-structured idiom in Snowflake, so the layer has to own it. The treatment is parser work plus a `get_path(value, 'a.b')` UDF: either extend the parser, or rewrite the path into a get_path call before parsing, taking care not to touch `::` casts or time literals. The spike judges the text rewrite risky and budgets this as real parser work.

### SF07  Time travel with AT

```sql
SELECT * FROM employees AT (TIMESTAMP => TIMESTAMP '2024-01-01 00:00:00');
```

Fails at parse time. The treatment sits in our layer: the parser hands the timestamp to the catalog, which resolves it to an Iceberg snapshot, and the table provider reads that snapshot. Spike B2 verified snapshot reads work in the Rust stack. The parser and provider wiring is ours to build.

### SF06  CLONE

```sql
CREATE TABLE employees_clone CLONE employees;
```

Parses, then the planner rejects it. CLONE is not a SQL engine problem. The treatment is a catalog operation over Iceberg snapshots: a clone is a new table whose first snapshot points at the same files. It is part of slice 1.3 in the Phase 1 plan, with the read path and the Postgres catalog.

### PG05  INSERT RETURNING

```sql
INSERT INTO users (id, name, dept, salary, hire_date, active)
VALUES (99, 'Zed', 'eng', 50, DATE '2024-01-01', true) RETURNING id;
```

Parses, then the planner rejects the returning clause. No SQL rewrite exists. The planned treatment is at the service layer: split the statement, run the insert, then select the keys back, once opensnow owns writable tables. Until then this is an accepted gap. It matters because agents generate RETURNING often. Read workloads are unaffected.

### PG06  ON CONFLICT

```sql
INSERT INTO users (id, name, dept, salary, hire_date, active)
VALUES (1, 'Alice', 'eng', 100, DATE '2020-01-02', true) ON CONFLICT (id) DO NOTHING;
```

Parses, then the planner rejects the on-conflict clause. No SQL rewrite exists today. The planned treatments are service-layer upsert semantics, or a two-statement form (insert with an anti-join against existing keys) once opensnow owns writes. Accepted gap for v1, on the roadmap next to PG05.

### PG09  Correlated LATERAL joins

```sql
SELECT e.name, sub.c FROM employees e,
  LATERAL (SELECT COUNT(*) AS c FROM events ev WHERE ev.user_id = e.id) sub;
```

Parses, then the physical plan rejects the outer reference. Correlated subqueries in WHERE work (ST07 in section 1); lateral joins are the specific hole. The planned treatment is a rewrite rule for the common pattern: pre-aggregate the inner side by the join key, then join that. The spike identified the pattern but did not write or verify the rule, so this stays a gap until the rule exists and passes a test.

### SF15  Dynamic PIVOT

A PIVOT whose IN list is read from the data, rather than written in the query, has no rewrite. The static form is covered in section 2. Accepted gap for v1.

### SF10  CLUSTER BY

```sql
CREATE TABLE t2 (id INT, d DATE) CLUSTER BY (d);
```

Parses under the Generic dialect, then the planner rejects it. It fails at parse time under the PostgreSql dialect. CLUSTER BY is a physical layout hint, not query logic. The treatment is to accept and strip the clause in the compat layer, with a log line, while the storage layer owns layout through Iceberg partitioning and sort order. Accepted gap as a user-facing tuning knob.

## What this means for the compat layer

The work falls in four buckets, in build order:

1. Rewrite rules on the parsed AST: DATEADD, DATEDIFF per unit, ARRAY_CONSTRUCT, OBJECT_CONSTRUCT, DIV0, FLATTEN over arrays, static PIVOT, the LATERAL pre-aggregation pattern, and the CLUSTER BY strip.
2. UDFs: `get_path` first, since both colon paths and the JSON side of FLATTEN depend on it, plus a JSON parse step if the storage layer does not hand over a native semi-structured value.
3. Parser work: colon paths and AT time travel, plus Snowflake named-argument syntax (`input => value`) in general.
4. Catalog and service layer: CLONE, snapshot resolution for time travel, and RETURNING and ON CONFLICT semantics for writable tables.

Scope note: this list covers the 37 statements the spike tested, which are query idioms plus two DDL shapes. The wider Snowflake DDL surface (warehouses, stages, file formats, tasks) is a separate inventory that no spike has measured yet.
