#!/bin/bash
# REFERENCE ONLY: DuckDB yardstick timings on the same Parquet files. DuckDB is not a dependency.
D=./duckdb-ref/duckdb
for i in 1 2 3 4 5 6; do
  Q=$(sed -n "$((2*i))p" queries.sql)
  echo "--- Q$i"
  for run in 1 2 3; do
    $D -c ".timer on" -c "CREATE VIEW sales AS SELECT * FROM 'data/sales.parquet'; CREATE VIEW products AS SELECT * FROM 'data/products.parquet';" -c "$Q" 2>&1 | grep -E "real|Error" | head -2
  done
done
