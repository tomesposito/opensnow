-- W1: Nightly ELT
-- Runs once a night on a Large warehouse. See README.md for the run mix and cost.
-- Tables: orders_raw (landing), sales_fact, daily_revenue_agg.
-- Query shapes map to spike B1: Q5 window (dedupe), Q6 CTE rollup (aggregate rebuild).

-- 1. Deduplicate the day's landed orders.
-- The same order can land twice from a retried extract. Keep the latest load.
CREATE OR REPLACE TEMP TABLE orders_deduped AS
SELECT order_id, order_date, customer_id, product_id, region,
       quantity, unit_price, discount
FROM orders_raw
WHERE loaded_at >= DATEADD(day, -1, CURRENT_TIMESTAMP())
QUALIFY ROW_NUMBER() OVER (
    PARTITION BY order_id
    ORDER BY loaded_at DESC
) = 1;

-- 2. Merge the new orders into the fact table.
MERGE INTO sales_fact AS f
USING orders_deduped AS o
    ON f.order_id = o.order_id
WHEN MATCHED THEN UPDATE SET
    f.order_date  = o.order_date,
    f.customer_id = o.customer_id,
    f.product_id  = o.product_id,
    f.region      = o.region,
    f.quantity    = o.quantity,
    f.unit_price  = o.unit_price,
    f.discount    = o.discount
WHEN NOT MATCHED THEN INSERT
    (order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount)
    VALUES
    (o.order_id, o.order_date, o.customer_id, o.product_id, o.region, o.quantity, o.unit_price, o.discount);

-- 3. Rebuild the daily revenue aggregate for the last 35 days.
-- Rebuilding a window instead of a single day keeps late-arriving corrections simple.
CREATE OR REPLACE TEMP TABLE daily_revenue_new AS
WITH daily AS (
    SELECT order_date,
           region,
           SUM(quantity * unit_price * (1 - discount)) AS net_revenue,
           COUNT(*) AS order_count
    FROM sales_fact
    WHERE order_date >= DATEADD(day, -35, CURRENT_DATE())
    GROUP BY order_date, region
),
monthly AS (
    SELECT DATE_TRUNC('month', order_date) AS month,
           region,
           AVG(net_revenue) AS avg_daily_revenue
    FROM daily
    GROUP BY month, region
)
SELECT d.order_date, d.region, d.net_revenue, d.order_count, m.avg_daily_revenue
FROM daily AS d
JOIN monthly AS m
  ON m.month = DATE_TRUNC('month', d.order_date)
 AND m.region = d.region;

MERGE INTO daily_revenue_agg AS a
USING daily_revenue_new AS n
    ON a.order_date = n.order_date AND a.region = n.region
WHEN MATCHED THEN UPDATE SET
    a.net_revenue = n.net_revenue,
    a.order_count = n.order_count,
    a.avg_daily_revenue = n.avg_daily_revenue
WHEN NOT MATCHED THEN INSERT
    (order_date, region, net_revenue, order_count, avg_daily_revenue)
    VALUES
    (n.order_date, n.region, n.net_revenue, n.order_count, n.avg_daily_revenue);

-- 4. Data quality checks. Each should return 0.
SELECT COUNT(*) AS null_order_ids
FROM sales_fact
WHERE order_id IS NULL
  AND order_date >= DATEADD(day, -1, CURRENT_DATE());

SELECT COUNT(*) AS negative_revenue_rows
FROM orders_deduped
WHERE quantity * unit_price * (1 - discount) < 0;

SELECT COUNT(*) AS orphan_customers
FROM orders_deduped AS o
LEFT JOIN dim_customer AS c
  ON c.customer_id = o.customer_id
WHERE c.customer_id IS NULL;
