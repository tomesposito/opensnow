-- W2: BI dashboards
-- Runs all business day on a Small warehouse, from a dashboard tool.
-- The tool sends the same shapes with different filters. Three representative
-- queries below; the README counts the sessions, not each filter variant.
-- Shapes map to spike B1: Q2 group by, Q4 top 10 sort, Q1 filtered scan.

-- 1. Revenue by region and month, trailing 12 months. (B1 Q2)
SELECT region,
       DATE_TRUNC('month', order_date) AS month,
       SUM(quantity * unit_price * (1 - discount)) AS net_revenue,
       COUNT(*) AS order_count
FROM sales_fact
WHERE order_date >= DATEADD(month, -12, CURRENT_DATE())
GROUP BY region, month
ORDER BY region, month;

-- 2. The same query as the tool sends it when a user picks one region.
SELECT region,
       DATE_TRUNC('month', order_date) AS month,
       SUM(quantity * unit_price * (1 - discount)) AS net_revenue,
       COUNT(*) AS order_count
FROM sales_fact
WHERE order_date >= DATEADD(month, -12, CURRENT_DATE())
  AND region = 'West'
GROUP BY region, month
ORDER BY region, month;

-- 3. Top 10 products by revenue this quarter. (B1 Q4)
SELECT p.product_name,
       p.category,
       SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue
FROM sales_fact AS f
JOIN dim_product AS p
  ON p.product_id = f.product_id
WHERE f.order_date >= DATE_TRUNC('quarter', CURRENT_DATE())
GROUP BY p.product_name, p.category
ORDER BY net_revenue DESC
LIMIT 10;

-- 4. Headline number for the home page: net revenue, current month. (B1 Q1)
SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue_mtd,
       COUNT(DISTINCT customer_id) AS active_customers_mtd
FROM sales_fact
WHERE order_date >= DATE_TRUNC('month', CURRENT_DATE());

-- 5. Funnel counts for the operations dashboard.
SELECT event_type,
       COUNT(*) AS events,
       COUNT(DISTINCT session_id) AS sessions
FROM clickstream_events
WHERE event_ts >= DATEADD(day, -7, CURRENT_TIMESTAMP())
GROUP BY event_type
ORDER BY events DESC;
