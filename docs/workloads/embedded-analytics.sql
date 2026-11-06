-- W4: Embedded customer-facing analytics
-- Every day on a Medium warehouse. The product fires these when a customer
-- opens a screen. :customer_id is the signed-in customer, bound by the app.
-- Many small runs, peak concurrency about 30. Shapes: B1 Q1, Q4, Q2.

-- 1. Customer home screen: their totals this month. (B1 Q1)
SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue_mtd,
       COUNT(*) AS orders_mtd,
       COUNT(DISTINCT product_id) AS products_mtd
FROM sales_fact
WHERE customer_id = :customer_id
  AND order_date >= DATE_TRUNC('month', CURRENT_DATE());

-- 2. Their top 10 products, trailing 90 days. (B1 Q4)
SELECT p.product_name,
       SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue
FROM sales_fact AS f
JOIN dim_product AS p
  ON p.product_id = f.product_id
WHERE f.customer_id = :customer_id
  AND f.order_date >= DATEADD(day, -90, CURRENT_DATE())
GROUP BY p.product_name
ORDER BY net_revenue DESC
LIMIT 10;

-- 3. Their monthly trend, trailing 12 months. (B1 Q2)
SELECT DATE_TRUNC('month', order_date) AS month,
       SUM(quantity * unit_price * (1 - discount)) AS net_revenue,
       COUNT(*) AS order_count
FROM sales_fact
WHERE customer_id = :customer_id
  AND order_date >= DATEADD(month, -12, CURRENT_DATE())
GROUP BY month
ORDER BY month;

-- 4. Their recent activity feed counts, from the clickstream.
SELECT event_type,
       COUNT(*) AS events
FROM clickstream_events
WHERE customer_id = :customer_id
  AND event_ts >= DATEADD(day, -30, CURRENT_TIMESTAMP())
GROUP BY event_type
ORDER BY events DESC;
