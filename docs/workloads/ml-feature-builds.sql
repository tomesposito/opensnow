-- W5: ML feature builds
-- Four big builds a month on an X-Large warehouse, plus a daily refresh
-- on a Small warehouse (marked below). Writes features_customer.
-- Shapes map to spike B1: Q5 window at full scale, Q6 CTE, Q3 large join.

-- 1. Recency, frequency, and monetary features per customer. (Big build)
CREATE OR REPLACE TEMP TABLE features_base AS
SELECT customer_id,
       DATEDIFF(day, MAX(order_date), CURRENT_DATE()) AS recency_days,
       COUNT(*) AS orders_12m,
       SUM(quantity * unit_price * (1 - discount)) AS spend_12m,
       AVG(quantity * unit_price * (1 - discount)) AS avg_order_value
FROM sales_fact
WHERE order_date >= DATEADD(month, -12, CURRENT_DATE())
GROUP BY customer_id;

-- 2. Spend percentile and rank inside each segment. (B1 Q5, full table)
CREATE OR REPLACE TEMP TABLE features_ranked AS
SELECT b.customer_id,
       b.recency_days,
       b.orders_12m,
       b.spend_12m,
       b.avg_order_value,
       PERCENT_RANK() OVER (
           PARTITION BY c.segment ORDER BY b.spend_12m
       ) AS spend_percentile_in_segment,
       NTILE(10) OVER (ORDER BY b.spend_12m DESC) AS spend_decile
FROM features_base AS b
JOIN dim_customer AS c
  ON c.customer_id = b.customer_id;

-- 3. Activity trend: this month against the trailing average. (B1 Q6)
CREATE OR REPLACE TEMP TABLE features_trend AS
WITH monthly AS (
    SELECT customer_id,
           DATE_TRUNC('month', event_ts) AS month,
           COUNT(*) AS events
    FROM clickstream_events
    WHERE event_ts >= DATEADD(month, -13, CURRENT_TIMESTAMP())
    GROUP BY customer_id, month
),
trailing AS (
    SELECT customer_id,
           AVG(events) AS avg_monthly_events
    FROM monthly
    WHERE month < DATE_TRUNC('month', CURRENT_TIMESTAMP())
    GROUP BY customer_id
)
SELECT m.customer_id,
       m.events AS events_this_month,
       t.avg_monthly_events,
       m.events / NULLIF(t.avg_monthly_events, 0) AS activity_ratio
FROM monthly AS m
JOIN trailing AS t
  ON t.customer_id = m.customer_id
WHERE m.month = DATE_TRUNC('month', CURRENT_TIMESTAMP());

-- 4. Write the feature table the model trains on.
MERGE INTO features_customer AS f
USING (
    SELECT r.customer_id, r.recency_days, r.orders_12m, r.spend_12m,
           r.avg_order_value, r.spend_percentile_in_segment, r.spend_decile,
           t.events_this_month, t.activity_ratio
    FROM features_ranked AS r
    LEFT JOIN features_trend AS t
      ON t.customer_id = r.customer_id
) AS s
    ON f.customer_id = s.customer_id
WHEN MATCHED THEN UPDATE SET
    f.recency_days = s.recency_days,
    f.orders_12m = s.orders_12m,
    f.spend_12m = s.spend_12m,
    f.avg_order_value = s.avg_order_value,
    f.spend_percentile_in_segment = s.spend_percentile_in_segment,
    f.spend_decile = s.spend_decile,
    f.events_this_month = s.events_this_month,
    f.activity_ratio = s.activity_ratio
WHEN NOT MATCHED THEN INSERT
    (customer_id, recency_days, orders_12m, spend_12m, avg_order_value,
     spend_percentile_in_segment, spend_decile, events_this_month, activity_ratio)
    VALUES
    (s.customer_id, s.recency_days, s.orders_12m, s.spend_12m, s.avg_order_value,
     s.spend_percentile_in_segment, s.spend_decile, s.events_this_month, s.activity_ratio);

-- 5. Daily refresh (Small warehouse): recompute recency only, for the last two days of buyers.
UPDATE features_customer AS f
SET recency_days = DATEDIFF(day, last_order.last_date, CURRENT_DATE())
FROM (
    SELECT customer_id, MAX(order_date) AS last_date
    FROM sales_fact
    WHERE order_date >= DATEADD(day, -2, CURRENT_DATE())
    GROUP BY customer_id
) AS last_order
WHERE f.customer_id = last_order.customer_id;
