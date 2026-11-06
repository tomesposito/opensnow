-- GROUP: postgres
-- TEST: PG01_ILIKE
SELECT name FROM employees WHERE name ILIKE 'a%';
-- TEST: PG02_CAST_OPERATOR
SELECT id, salary::varchar AS salary_text FROM employees;
-- TEST: PG03_DATE_TRUNC
SELECT date_trunc('month', hire_date) AS month_start FROM employees;
-- TEST: PG04_GENERATE_SERIES
SELECT value FROM generate_series(1, 5) AS t(value);
-- TEST: PG05_RETURNING
INSERT INTO users (id, name, dept, salary, hire_date, active) VALUES (99, 'Zed', 'eng', 50, DATE '2024-01-01', true) RETURNING id;
-- TEST: PG06_ON_CONFLICT
INSERT INTO users (id, name, dept, salary, hire_date, active) VALUES (1, 'Alice', 'eng', 100, DATE '2020-01-02', true) ON CONFLICT (id) DO NOTHING;
-- TEST: PG07_ARRAY_CONSTRUCT
SELECT ARRAY[1, 2, 3] AS a FROM employees LIMIT 1;
-- TEST: PG08_UNNEST
SELECT unnest(ARRAY[1, 2, 3]) AS v;
-- TEST: PG09_LATERAL
SELECT e.name, sub.c FROM employees e, LATERAL (SELECT COUNT(*) AS c FROM events ev WHERE ev.user_id = e.id) sub;
-- TEST: PG10_DISTINCT_ON
SELECT DISTINCT ON (dept) dept, name, salary FROM employees ORDER BY dept, salary DESC;
-- TEST: PG11_FILTER_WHERE
SELECT COUNT(*) FILTER (WHERE active) AS active_count FROM employees;
-- TEST: PG12_CONCAT_OPERATOR
SELECT name || '-' || dept AS label FROM employees;
-- TEST: PG13_EXTRACT_EPOCH
SELECT EXTRACT(EPOCH FROM created_at) AS epoch_s FROM events;
