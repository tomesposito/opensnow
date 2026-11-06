-- GROUP: standard
-- TEST: ST01_JOIN_GROUP
SELECT e.dept, COUNT(*) AS n, AVG(e.salary) AS avg_salary FROM employees e GROUP BY e.dept ORDER BY e.dept;
-- TEST: ST02_CTE
WITH top_paid AS (SELECT * FROM employees WHERE salary > 75) SELECT COUNT(*) AS n FROM top_paid;
-- TEST: ST03_WINDOW_ROW_NUMBER
SELECT name, ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn FROM employees;
-- TEST: ST04_WINDOW_SUM_OVER
SELECT name, dept, SUM(salary) OVER (PARTITION BY dept) AS dept_total FROM employees;
-- TEST: ST05_CASE
SELECT name, CASE WHEN salary >= 90 THEN 'high' WHEN salary >= 70 THEN 'mid' ELSE 'low' END AS band FROM employees;
-- TEST: ST06_UNION
SELECT dept FROM employees UNION SELECT dept FROM users;
-- TEST: ST07_CORRELATED_SUBQUERY
SELECT name FROM employees e WHERE salary > (SELECT AVG(salary) FROM employees WHERE dept = e.dept);
-- TEST: ST08_HAVING_LIMIT
SELECT dept, COUNT(*) AS n FROM employees GROUP BY dept HAVING COUNT(*) > 1 ORDER BY n DESC LIMIT 2;
-- TEST: ST09_EXISTS_SUBQUERY
SELECT name FROM employees e WHERE EXISTS (SELECT 1 FROM events ev WHERE ev.user_id = e.id);
