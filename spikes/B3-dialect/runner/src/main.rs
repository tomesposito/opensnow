use datafusion::arrow::array::{BooleanArray, Date32Array, Float64Array, Int32Array, Int64Array, StringArray, TimestampNanosecondArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::MemTable;
use datafusion::prelude::*;
use datafusion::sql::sqlparser::dialect::{GenericDialect, PostgreSqlDialect};
use datafusion::sql::sqlparser::parser::Parser;
use std::sync::Arc;

fn employees_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("dept", DataType::Utf8, false),
        Field::new("salary", DataType::Int64, false),
        Field::new("hire_date", DataType::Date32, false),
        Field::new("active", DataType::Boolean, false),
    ]));
    // Date32 is days since epoch. 2020-01-01 = 18262, etc. Values are approximate and fine for dialect tests.
    RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5])),
            Arc::new(StringArray::from(vec!["Alice", "bob", "Carol", "DAN", "Eve"])),
            Arc::new(StringArray::from(vec!["eng", "eng", "sales", "sales", "eng"])),
            Arc::new(Int64Array::from(vec![100, 90, 80, 70, 60])),
            Arc::new(Date32Array::from(vec![18262, 18628, 18993, 19358, 19723])),
            Arc::new(BooleanArray::from(vec![true, true, false, true, true])),
        ],
    )
    .unwrap()
}

fn events_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("user_id", DataType::Int32, false),
        Field::new("payload", DataType::Utf8, false),
        Field::new("amount", DataType::Float64, false),
        Field::new("created_at", DataType::Timestamp(TimeUnit::Nanosecond, None), false),
        Field::new("tags", DataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(Int32Array::from(vec![1, 1, 2])),
            Arc::new(StringArray::from(vec![
                r#"{"a": 1, "b": {"c": "x"}, "arr": [10, 20]}"#,
                r#"{"a": 2, "b": {"c": "y"}, "arr": [30]}"#,
                r#"{"a": 3, "b": {"c": "z"}, "arr": []}"#,
            ])),
            Arc::new(Float64Array::from(vec![10.5, 20.0, 0.0])),
            Arc::new(TimestampNanosecondArray::from(vec![
                1704067200000000000i64,
                1706745600000000000i64,
                1709251200000000000i64,
            ])),
            Arc::new(StringArray::from(vec!["a,b", "c", "a"])),
        ],
    )
    .unwrap()
}

async fn fresh_ctx(dialect: &str) -> SessionContext {
    let mut config = SessionConfig::new();
    // DataFusion exposes the parser dialect as a config string in recent versions.
    config.options_mut().sql_parser.dialect = dialect.to_string();
    let ctx = SessionContext::new_with_config(config);
    let emp_schema = employees_batch().schema();
    let emp_table = MemTable::try_new(emp_schema, vec![vec![employees_batch()]]).unwrap();
    ctx.register_table("employees", Arc::new(emp_table)).unwrap();
    let ev_schema = events_batch().schema();
    let ev_table = MemTable::try_new(ev_schema, vec![vec![events_batch()]]).unwrap();
    ctx.register_table("events", Arc::new(ev_table)).unwrap();
    // Alias used by several Snowflake and Postgres tests.
    let emp2_schema = employees_batch().schema();
    let emp2_table = MemTable::try_new(emp2_schema, vec![vec![employees_batch()]]).unwrap();
    ctx.register_table("users", Arc::new(emp2_table)).unwrap();
    ctx
}

fn classify(msg: &str) -> &'static str {
    let lower = msg.to_lowercase();
    if lower.contains("parsererror")
        || lower.contains("parse error")
        || lower.contains("sql parser error")
        || lower.contains("expected:")
        || lower.contains("expected ")
        || lower.contains("syntax")
    {
        "parse"
    } else if lower.contains("not implemented")
        || lower.contains("unsupported")
        || lower.contains("not supported")
        || lower.contains("notimplemented")
    {
        "unsupported"
    } else {
        "runtime"
    }
}

fn short(s: &str, n: usize) -> String {
    let one = s.replace('\n', " ");
    if one.len() <= n {
        one
    } else {
        format!("{}...", &one[..n])
    }
}

#[derive(Debug)]
struct TestCase {
    id: String,
    sql: String,
}

fn load_tests(path: &str) -> Vec<TestCase> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let mut out = Vec::new();
    let mut cur_id: Option<String> = None;
    let mut buf = String::new();
    let flush = |out: &mut Vec<TestCase>, cur_id: &Option<String>, buf: &String| {
        if let Some(id) = cur_id {
            let sql = buf.trim().trim_end_matches(';').trim().to_string();
            if !sql.is_empty() {
                out.push(TestCase { id: id.clone(), sql });
            }
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("-- TEST:") {
            flush(&mut out, &cur_id, &buf);
            cur_id = Some(rest.trim().to_string());
            buf.clear();
        } else if cur_id.is_some() && !line.trim_start().starts_with("-- GROUP") {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&mut out, &cur_id, &buf);
    out
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    // usage: runner <generic|PostgreSql> <file.sql> [file.sql...]
    let dialect_arg = args.get(1).cloned().unwrap_or_else(|| "generic".to_string());
    let (df_dialect, is_pg): (&str, bool) = if dialect_arg.to_lowercase().starts_with("post") {
        ("PostgreSql", true)
    } else {
        ("Generic", false)
    };
    println!("{{\"runner\":\"df-dialect-runner\",\"dialect\":\"{df_dialect}\"}}");
    for path in &args[2..] {
        for tc in load_tests(path) {
            // Layer 1: raw sqlparser parse check.
            let parse_res = if is_pg {
                Parser::parse_sql(&PostgreSqlDialect {}, &tc.sql)
            } else {
                Parser::parse_sql(&GenericDialect {}, &tc.sql)
            };
            let (parse_ok, parse_err) = match &parse_res {
                Ok(_) => (true, String::new()),
                Err(e) => (false, e.to_string()),
            };
            // Layer 2: DataFusion plan and execute, fresh context per test.
            let ctx = fresh_ctx(df_dialect).await;
            let mut exec_ok = false;
            let mut exec_err = String::new();
            let mut rows: Option<usize> = None;
            match ctx.sql(&tc.sql).await {
                Ok(df) => match df.collect().await {
                    Ok(batches) => {
                        exec_ok = true;
                        rows = Some(batches.iter().map(|b| b.num_rows()).sum());
                    }
                    Err(e) => exec_err = e.to_string(),
                },
                Err(e) => exec_err = e.to_string(),
            }
            let err_text = if !exec_err.is_empty() {
                exec_err.clone()
            } else {
                parse_err.clone()
            };
            let class = if parse_ok && exec_ok {
                "ok"
            } else if !parse_ok {
                "parse"
            } else {
                classify(&err_text)
            };
            let rec = serde_json::json!({
                "id": tc.id,
                "file": path,
                "dialect": df_dialect,
                "parse_ok": parse_ok,
                "exec_ok": exec_ok,
                "rows": rows,
                "error_class": class,
                "error": short(&err_text, 500),
                "sql": tc.sql,
            });
            println!("{}", rec.to_string());
        }
    }
}
