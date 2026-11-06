-- GROUP: workarounds
-- TEST: W01_DATEADD_AS_INTERVAL
SELECT hire_date + INTERVAL '7' DAY AS plus_week FROM employees;
-- TEST: W02_DATEDIFF_AS_DATE_DIFF
SELECT date_diff('day', hire_date, DATE '2025-01-01') AS days_diff FROM employees;
-- TEST: W03_OBJECT_AS_NAMED_STRUCT
SELECT named_struct('a', 1, 'b', 2) AS obj;
-- TEST: W04_ARRAY_AS_MAKE_ARRAY
SELECT make_array(1, 2, 3) AS arr;
-- TEST: W05_DIV0_AS_NULLIF
SELECT 10 / NULLIF(0, 0) AS safe_div;
-- TEST: W06_FLATTEN_AS_UNNEST
SELECT unnest(make_array(10, 20)) AS value;
-- TEST: W07_PIVOT_AS_CONDITIONAL_AGG
SELECT SUM(CASE WHEN dept = 'eng' THEN salary END) AS eng, SUM(CASE WHEN dept = 'sales' THEN salary END) AS sales FROM employees;
-- TEST: W08_QUALIFY_FREE_TOP_PER_GROUP
SELECT dept, name, salary FROM (SELECT dept, name, salary, ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn FROM employees) WHERE rn = 1;
