//! Local mode, MCP server, and HTTP transport for opensnow.
//! See docs/MCP.md and issue #12 (slice 1.9).

pub mod http;
pub mod local;
pub mod mcp;
pub mod results;

pub use local::LocalDb;
pub use mcp::McpServer;
pub use opensnow_engine::{CostRecord, TenantCostSummary};

use arrow::record_batch::RecordBatch;
use opensnow_engine::QueryResult;

/// Runs a local-mode MCP smoke path in a temporary directory: open,
/// create, insert, and query through the JSON-RPC handler. Used by
/// stack-check so the combined harness covers this slice too.
async fn check_call(server: &McpServer, tool: &str, args: serde_json::Value) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": args}}).to_string();
    let reply = server.handle_jsonrpc(&body, None).await.expect("reply");
    let value: serde_json::Value = serde_json::from_str(&reply)?;
    if value["result"]["isError"] == serde_json::Value::Bool(true) {
        return Err(format!("tool {tool} failed: {}", value["result"]["content"][0]["text"]).into());
    }
    Ok(serde_json::from_str(value["result"]["content"][0]["text"].as_str().unwrap())?)
}

pub async fn check() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let temp = tempfile::tempdir()?;
    let db = LocalDb::open(temp.path().join("local"), None).await?;
    let server = McpServer::local(db);
    check_call(&server, "execute_write", serde_json::json!({"sql": "CREATE TABLE events (id BIGINT NOT NULL)"})).await?;
    check_call(&server, "execute_write", serde_json::json!({"sql": "INSERT INTO events VALUES (1), (2)"})).await?;
    let out = check_call(&server, "query", serde_json::json!({"sql": "SELECT id FROM events ORDER BY id"})).await?;
    assert_eq!(out["row_count"], 2);
    println!("PASS MCP local mode: provision, load, and query through tools alone");
    Ok(())
}

pub fn record_to_json(record: &CostRecord) -> serde_json::Value {
    serde_json::json!({
        "seq": record.seq,
        "kind": record.kind.as_str(),
        "status": record.status.as_str(),
        "sql": record.sql,
        "service": record.service,
        "started_at_unix_ms": record.started_at_unix_ms,
        "elapsed_ms": record.elapsed.as_secs_f64() * 1000.0,
        "rows_returned": record.rows_returned,
        "bytes_returned": record.bytes_returned,
        "vcpu_seconds": record.vcpu_seconds,
        "cost_dollars": record.cost_dollars,
    })
}

pub fn summary_to_json(summary: &TenantCostSummary) -> serde_json::Value {
    serde_json::json!({
        "record_count": summary.record_count,
        "total_cost_dollars": summary.total_cost_dollars,
        "total_vcpu_seconds": summary.total_vcpu_seconds,
        "total_elapsed_ms": summary.total_elapsed.as_secs_f64() * 1000.0,
        "total_rows_returned": summary.total_rows_returned,
        "total_bytes_returned": summary.total_bytes_returned,
    })
}

pub fn query_result_to_json(result: &QueryResult) -> serde_json::Value {
    let mut value = results::batches_to_json(&result.batches);
    value["outcome"] = serde_json::json!({
        "rows_returned": result.outcome.rows_returned,
        "cost_units_charged": result.outcome.cost_units_charged,
        "elapsed_ms": result.outcome.elapsed.as_secs_f64() * 1000.0,
        "cost_record_seq": result.outcome.cost_record_seq,
        "bytes_returned": result.outcome.bytes_returned,
        "vcpu_seconds": result.outcome.vcpu_seconds,
        "cost_dollars": result.outcome.cost_dollars,
    });
    value
}

pub fn batches_to_json(batches: &[RecordBatch]) -> serde_json::Value {
    results::batches_to_json(batches)
}
