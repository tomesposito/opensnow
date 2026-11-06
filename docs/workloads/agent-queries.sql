-- W6: Agent-driven queries
-- Weekdays on an X-Small warehouse. An AI agent explores schema, probes data,
-- and checks numbers in many small statements, most under two seconds.
-- The workload is the pattern, not any single query. Shapes: B1 Q1 at tiny scale.

-- 1. Schema exploration: what tables exist and what they hold.
SELECT table_name, row_count, bytes
FROM INFORMATION_SCHEMA.TABLES
WHERE table_schema = 'ANALYTICS'
ORDER BY bytes DESC;

SELECT column_name, data_type
FROM INFORMATION_SCHEMA.COLUMNS
WHERE table_name = 'SALES_FACT'
ORDER BY ordinal_position;

-- 2. First look at a table the agent has not seen before.
SELECT *
FROM clickstream_events
SAMPLE (100 ROWS);

-- 3. Probe: is this column clean enough to group by?
SELECT region,
       COUNT(*) AS rows_seen,
       SUM(CASE WHEN TRY_CAST(quantity AS NUMBER) IS NULL THEN 1 ELSE 0 END) AS bad_quantities
FROM sales_fact
WHERE order_date >= DATEADD(day, -7, CURRENT_DATE())
GROUP BY region;

-- 4. Checking a number a human quoted in a doc.
SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue
FROM sales_fact
WHERE order_date >= DATE_TRUNC('month', CURRENT_DATE())
  AND region = 'East';

-- 5. Following up: which products drive that number?
SELECT product_id,
       SUM(quantity * unit_price * (1 - discount)) AS net_revenue
FROM sales_fact
WHERE order_date >= DATE_TRUNC('month', CURRENT_DATE())
  AND region = 'East'
GROUP BY product_id
ORDER BY net_revenue DESC
LIMIT 20;

-- 6. Sanity check on a join the agent is about to trust.
SELECT COUNT(*) AS fact_rows_without_customer
FROM sales_fact AS f
LEFT JOIN dim_customer AS c
  ON c.customer_id = f.customer_id
WHERE c.customer_id IS NULL
  AND f.order_date >= DATEADD(day, -30, CURRENT_DATE());
