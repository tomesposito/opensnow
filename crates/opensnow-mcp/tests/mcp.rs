use std::sync::Arc;
use std::time::Instant;

use opensnow_control::{ControlPlane, ControlStore};
use opensnow_mcp::http::{serve, HttpServer};
use opensnow_mcp::{LocalDb, McpServer};
use opensnow_storage::CatalogConfig;
use serde_json::{json, Value};
use sqlx::ConnectOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn rpc(server: &McpServer, body: Value, auth: Option<&str>) -> Value {
    let reply = server.handle_jsonrpc(&body.to_string(), auth).await.expect("reply expected");
    serde_json::from_str(&reply).unwrap()
}

async fn call(server: &McpServer, tool: &str, args: Value, auth: Option<&str>) -> Value {
    let reply = rpc(server, json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": tool, "arguments": args}}), auth).await;
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    if reply["result"]["isError"] == json!(true) {
        return json!({"__error": text});
    }
    serde_json::from_str(&text).unwrap_or_else(|_| json!({"__raw": text}))
}

async fn call_err(server: &McpServer, tool: &str, args: Value, auth: Option<&str>) -> String {
    let out = call(server, tool, args, auth).await;
    out["__error"].as_str().unwrap_or_default().to_string()
}

const CREATE: &str = "CREATE TABLE items (id BIGINT NOT NULL, name VARCHAR, dept VARCHAR, salary BIGINT)";
const INSERT: &str = "INSERT INTO items VALUES (1, 'Alice', 'eng', 100), (2, 'Bob', 'sales', 80), (3, 'Carol', 'eng', 90)";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_mcp_end_to_end() -> TestResult {
    let temp = tempfile::tempdir()?;
    let start = Instant::now();
    let db = LocalDb::open(temp.path().join("local"), None).await?;
    assert!(start.elapsed().as_secs() < 5, "local provisioning took {:?}", start.elapsed());
    let server = McpServer::local(db);

    let init = rpc(&server, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-03-26"}}), None).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "opensnow");

    let list = rpc(&server, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), None).await;
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter()
        .map(|t| t["name"].as_str().unwrap()).collect();
    for expected in ["provision_database", "list_tables", "describe_table", "search_schema",
        "sample_rows", "query", "execute_write", "cost_records", "cost_summary", "docs_search"] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }

    let provisioned = call(&server, "provision_database", json!({}), None).await;
    assert_eq!(provisioned["mode"], "local");

    call(&server, "execute_write", json!({"sql": CREATE}), None).await;
    call(&server, "execute_write", json!({"sql": INSERT}), None).await;

    let tables = call(&server, "list_tables", json!({}), None).await;
    assert_eq!(tables["tables"], json!(["items"]));

    let rows = call(&server, "query", json!({"sql": "SELECT name FROM items WHERE name ILIKE 'a%'"}), None).await;
    assert_eq!(rows["rows"], json!([["Alice"]]));
    assert!(rows["outcome"]["cost_dollars"].as_f64().is_some());

    let sample = call(&server, "sample_rows", json!({"table": "items", "limit": 2}), None).await;
    assert_eq!(sample["row_count"], 2);

    let desc = call(&server, "describe_table", json!({"table": "items"}), None).await;
    let col_names: Vec<&str> = desc["columns"].as_array().unwrap().iter()
        .map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(col_names, ["id", "name", "dept", "salary"]);

    let search = call(&server, "search_schema", json!({"query": "sal"}), None).await;
    assert!(search["matches"].as_array().unwrap().iter()
        .any(|m| m["column"] == "salary"), "schema search misses salary: {search}");

    let records = call(&server, "cost_records", json!({}), None).await;
    let records = records["records"].as_array().unwrap().clone();
    assert!(records.len() >= 4, "expected cost records, got {}", records.len());
    assert!(records.iter().all(|r| r["cost_dollars"].as_f64().is_some()));

    let summary = call(&server, "cost_summary", json!({}), None).await;
    assert_eq!(summary["record_count"].as_u64().unwrap(), records.len() as u64);
    let total: f64 = records.iter().map(|r| r["cost_dollars"].as_f64().unwrap()).sum();
    assert!((summary["total_cost_dollars"].as_f64().unwrap() - total).abs() < 1e-12);

    let docs = call(&server, "docs_search", json!({"query": "DATEADD"}), None).await;
    assert!(docs["matches"].as_array().unwrap().iter()
        .any(|m| m["doc"] == "COMPATIBILITY.md"), "docs search misses DATEADD: {docs}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_split_is_read_only_by_default() -> TestResult {
    let temp = tempfile::tempdir()?;
    let server = McpServer::local(LocalDb::open(temp.path().join("local"), None).await?);
    let err = call_err(&server, "query", json!({"sql": INSERT}), None).await;
    assert!(err.contains("execute_write"), "query refused write with: {err}");
    let err = call_err(&server, "execute_write", json!({"sql": "SELECT id FROM items"}), None).await;
    assert!(err.contains("query"), "execute_write refused read with: {err}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_cost_cap_aborts_through_query_tool() -> TestResult {
    let temp = tempfile::tempdir()?;
    let server = McpServer::local(LocalDb::open(temp.path().join("local"), Some(2)).await?);
    call(&server, "execute_write", json!({"sql": CREATE}), None).await;
    call(&server, "execute_write", json!({"sql": INSERT}), None).await;
    let err = call_err(&server, "query", json!({"sql": "SELECT id FROM items"}), None).await;
    assert!(!err.is_empty(), "cap query should fail");
    let records = call(&server, "cost_records", json!({}), None).await;
    assert!(records["records"].as_array().unwrap().iter()
        .any(|r| r["status"] == "cost_cap_exceeded"), "no cap record in {records}");
    Ok(())
}

// Hosted fixture, same pattern as crates/opensnow-control/tests.
struct Fixture {
    plane: Arc<ControlPlane>,
    db: String,
    _temp: tempfile::TempDir,
}

async fn setup(label: &str) -> Result<Option<Fixture>, Box<dyn std::error::Error + Send + Sync>> {
    let mut admin = match sqlx::postgres::PgConnectOptions::new().host("localhost").port(5433)
        .username("postgres").database("postgres").connect().await {
        Ok(c) => c,
        Err(error) => {
            let unreachable = matches!(&error, sqlx::Error::Io(io) if matches!(io.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::NetworkUnreachable | std::io::ErrorKind::HostUnreachable));
            if unreachable {
                println!("SKIP hosted {label}: Postgres unreachable: {error}");
                return Ok(None);
            }
            return Err(error.into());
        }
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let db = format!("opensnow_mcp_{label}_{}", &suffix[..8]);
    sqlx::query(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)")).execute(&mut admin).await?;
    sqlx::query(&format!("CREATE DATABASE {db}")).execute(&mut admin).await?;
    let uri = format!("postgres://postgres@localhost:5433/{db}");
    let pool = sqlx::PgPool::connect(&uri).await?;
    let store = ControlStore::new(pool);
    store.ensure_schema().await?;
    let temp = tempfile::tempdir()?;
    let storage = CatalogConfig::postgres("mcp-test", uri.clone(), temp.path().join("warehouse"))
        .connect().await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e })?;
    Ok(Some(Fixture { plane: Arc::new(ControlPlane::new(store, Arc::new(storage))), db, _temp: temp }))
}

impl Fixture {
    async fn teardown(self) -> TestResult {
        let mut admin = sqlx::postgres::PgConnectOptions::new().host("localhost").port(5433)
            .username("postgres").database("postgres").connect().await?;
        sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.db)).execute(&mut admin).await?;
        Ok(())
    }
}

const COMPAT_QUERIES: [&str; 4] = [
    "SELECT name FROM items WHERE name ILIKE 'a%' ORDER BY name",
    "SELECT dept, COUNT(*) AS n FROM items GROUP BY dept ORDER BY dept",
    "SELECT id, salary::varchar AS s FROM items ORDER BY id",
    "SELECT name FROM items WHERE salary > 85 ORDER BY name",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_and_hosted_run_identically() -> TestResult {
    let Some(fx) = setup("identical").await? else { return Ok(()); };
    let hosted = McpServer::hosted(fx.plane.clone());
    let start = Instant::now();
    let provisioned = call(&hosted, "provision_database", json!({
        "name": "agentco", "admin_login": "admin@agentco.test", "admin_password": "s3cret-admin-pass",
    }), None).await;
    assert!(start.elapsed().as_secs() < 5, "hosted provisioning took {:?}", start.elapsed());
    let token = provisioned["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("osn_"));

    // No token, no data.
    let err = call_err(&hosted, "list_tables", json!({}), None).await;
    assert!(err.contains("authentication failed"), "unauthenticated call said: {err}");

    let temp = tempfile::tempdir()?;
    let local = McpServer::local(LocalDb::open(temp.path().join("local"), None).await?);

    for server_and_token in [(&hosted, Some(token.as_str())), (&local, None)] {
        let (server, auth) = server_and_token;
        call(server, "execute_write", json!({"sql": CREATE}), auth).await;
        call(server, "execute_write", json!({"sql": INSERT}), auth).await;
    }
    for sql in COMPAT_QUERIES {
        let hosted_rows = call(&hosted, "query", json!({"sql": sql}), Some(&token)).await;
        let local_rows = call(&local, "query", json!({"sql": sql}), None).await;
        assert_eq!(hosted_rows["columns"], local_rows["columns"], "columns differ for {sql}");
        assert_eq!(hosted_rows["rows"], local_rows["rows"], "rows differ for {sql}");
    }
    fx.teardown().await
}

async fn http_request(stream_addr: std::net::SocketAddr, request: &str) -> (u16, Value) {
    let mut stream = tokio::net::TcpStream::connect(stream_addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
    (status, serde_json::from_str(&body).unwrap_or(json!({})))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_transport_end_to_end() -> TestResult {
    let Some(fx) = setup("http").await? else { return Ok(()); };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(serve(listener, HttpServer::new(fx.plane.clone())));
    let req = |method: &str, path: &str, token: Option<&str>, body: &Value| {
        let auth = token.map(|t| format!("authorization: Bearer {t}\r\n")).unwrap_or_default();
        format!("{method} {path} HTTP/1.1\r\nhost: x\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.to_string().len(), body)
    };

    let (status, _) = http_request(addr, &req("POST", "/api/execute", None, &json!({"sql": "SELECT 1"}))).await;
    assert_eq!(status, 401, "unauthenticated execute must be 401");

    let (status, provisioned) = http_request(addr, &req("POST", "/api/provision", None,
        &json!({"name": "httpco", "admin_login": "a@httpco.test", "admin_password": "s3cret-admin-pass"}))).await;
    assert_eq!(status, 200, "provision said {provisioned}");
    let token = provisioned["token"].as_str().unwrap().to_string();

    for sql in [CREATE, INSERT] {
        let (status, out) = http_request(addr, &req("POST", "/api/execute", Some(&token), &json!({"sql": sql}))).await;
        assert_eq!(status, 200, "execute {sql} said {out}");
    }
    let (status, out) = http_request(addr, &req("POST", "/api/execute", Some(&token),
        &json!({"sql": "SELECT name FROM items WHERE salary > 85 ORDER BY name"}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["rows"], json!([["Alice"], ["Carol"]]));

    let (status, out) = http_request(addr, &req("GET", "/api/tables", Some(&token), &json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["tables"], json!(["items"]));

    let (status, out) = http_request(addr, &req("GET", "/api/cost-records", Some(&token), &json!({}))).await;
    assert_eq!(status, 200);
    assert!(out["records"].as_array().unwrap().len() >= 3);

    let mcp_body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "query", "arguments": {"sql": "SELECT COUNT(*) AS n FROM items"}}});
    let (status, out) = http_request(addr, &req("POST", "/mcp", Some(&token), &mcp_body)).await;
    assert_eq!(status, 200, "mcp over http said {out}");
    assert_eq!(out["result"]["isError"], json!(false));

    server.abort();
    fx.teardown().await
}
