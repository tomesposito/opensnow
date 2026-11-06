-- Q1 scan + filter + aggregate
SELECT count(*) AS n, sum(quantity * unit_price * (1 - discount)) AS revenue FROM sales WHERE region = 'West' AND order_date >= DATE '2024-01-01';
-- Q2 group by region and month
SELECT region, date_trunc('month', order_date) AS month, sum(quantity * unit_price * (1 - discount)) AS revenue, count(*) AS n FROM sales GROUP BY region, date_trunc('month', order_date) ORDER BY region, month;
-- Q3 join + aggregate by category
SELECT p.category, count(*) AS n, sum(s.quantity * s.unit_price * (1 - s.discount)) AS revenue FROM sales s JOIN products p ON s.product_id = p.product_id GROUP BY p.category ORDER BY revenue DESC;
-- Q4 sort + top-N
SELECT order_id, customer_id, quantity * unit_price * (1 - discount) AS line_revenue FROM sales ORDER BY line_revenue DESC LIMIT 10;
-- Q5 window: top 3 orders per customer by revenue
SELECT customer_id, order_id, line_revenue, rnk FROM (SELECT customer_id, order_id, quantity * unit_price * (1 - discount) AS line_revenue, rank() OVER (PARTITION BY customer_id ORDER BY quantity * unit_price * (1 - discount) DESC) AS rnk FROM sales) t WHERE rnk <= 3 ORDER BY customer_id, rnk;
-- Q6 CTE + subquery: months above average monthly revenue
WITH monthly AS (SELECT date_trunc('month', order_date) AS month, sum(quantity * unit_price * (1 - discount)) AS revenue FROM sales GROUP BY date_trunc('month', order_date)) SELECT month, revenue FROM monthly WHERE revenue > (SELECT avg(revenue) FROM monthly) ORDER BY month;
