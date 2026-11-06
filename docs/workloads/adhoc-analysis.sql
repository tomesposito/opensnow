-- W3: Ad-hoc analyst analysis
-- Weekdays on a Medium warehouse. Written by hand, run once or twice,
-- then edited and run again. Shapes map to spike B1: Q3 join, Q5 window, Q1 scan.

-- 1. Revenue by category and customer segment. (B1 Q3)
-- An analyst starts here when a dashboard number looks wrong.
SELECT p.category,
       c.segment,
       SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue,
       COUNT(*) AS order_count
FROM sales_fact AS f
JOIN dim_product AS p
  ON p.product_id = f.product_id
JOIN dim_customer AS c
  ON c.customer_id = f.customer_id
WHERE f.order_date >= DATEADD(month, -6, CURRENT_DATE())
GROUP BY p.category, c.segment
ORDER BY net_revenue DESC;

-- 2. Top 3 customers by revenue in each region, this year. (B1 Q5)
SELECT customer_id, region, net_revenue, rnk
FROM (
    SELECT customer_id,
           region,
           SUM(quantity * unit_price * (1 - discount)) AS net_revenue,
           RANK() OVER (PARTITION BY region ORDER BY net_revenue DESC) AS rnk
    FROM sales_fact
    WHERE order_date >= DATE_TRUNC('year', CURRENT_DATE())
    GROUP BY customer_id, region
) AS ranked
WHERE rnk <= 3
ORDER BY region, rnk;

-- 3. Discount investigation: orders where the discount looks out of line. (B1 Q1)
SELECT region,
       COUNT(*) AS orders_checked,
       SUM(CASE WHEN discount > 0.4 THEN 1 ELSE 0 END) AS deep_discount_orders,
       AVG(discount) AS avg_discount
FROM sales_fact
WHERE order_date >= DATEADD(month, -3, CURRENT_DATE())
  AND quantity * unit_price > 100
GROUP BY region
ORDER BY deep_discount_orders DESC;

-- 4. Month over month growth for one category, written after seeing query 1.
WITH monthly AS (
    SELECT DATE_TRUNC('month', f.order_date) AS month,
           SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue
    FROM sales_fact AS f
    JOIN dim_product AS p
      ON p.product_id = f.product_id
    WHERE p.category = 'Software'
    GROUP BY month
)
SELECT month,
       net_revenue,
       LAG(net_revenue) OVER (ORDER BY month) AS prior_month,
       net_revenue - LAG(net_revenue) OVER (ORDER BY month) AS change
FROM monthly
ORDER BY month;
