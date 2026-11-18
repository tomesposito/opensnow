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

    let (status, out) = http_request(addr, &req("POST", "/api/execute", Some(&token),
        &json!({"sql": MERGE_ITEMS}))).await;
    assert_eq!(status, 200, "HTTP MERGE said {out}");
    let (status, out) = http_request(addr, &req("POST", "/api/execute", Some(&token),
        &json!({"sql": "SELECT name FROM items WHERE id = 1"}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["rows"], json!([["Alicia"]]));

    server.abort();
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutations_use_writer_snapshots_and_metering() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("local");
    let db = LocalDb::open(&root, None).await?;
    // A second catalog handle observes snapshots without reopening LocalDb.
    let storage = CatalogConfig::sqlite("opensnow-local", root.join("catalog.db"), root.join("warehouse"))?.connect().await?;
    let namespace = "local";
    for sql in [
        "CREATE TABLE events (id BIGINT NOT NULL, value BIGINT)",
        "INSERT INTO events VALUES (1, 10), (2, 20), (3, NULL)",
        "INSERT INTO events VALUES (4, 40), (5, 50)",
        "CREATE TABLE incoming (id BIGINT NOT NULL, value BIGINT)",
        "INSERT INTO incoming VALUES (2, 200), (3, 300), (6, 60)",
    ] { db.execute(sql).await?; }
    let ident = iceberg::TableIdent::from_strs([namespace, "events"])?;
    let cases: &[(&str, &[(i64, i64)])] = &[
        ("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN UPDATE SET e.value = i.value WHEN NOT MATCHED THEN INSERT (id, value) VALUES (i.id, i.value)", &[(1, 10), (2, 200), (3, 300), (4, 40), (5, 50), (6, 60)]),
        ("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED AND e.value >= 300 THEN DELETE WHEN MATCHED THEN UPDATE SET e.value = i.value", &[(1, 10), (2, 200), (4, 40), (5, 50), (6, 60)]),
        ("UPDATE events AS e SET value = i.value + 1 FROM incoming AS i WHERE e.id = i.id", &[(1, 10), (2, 201), (4, 40), (5, 50), (6, 61)]),
        ("UPDATE events SET value = value + 1 WHERE id = 1", &[(1, 11), (2, 201), (4, 40), (5, 50), (6, 61)]),
        ("DELETE FROM events WHERE id = 5", &[(1, 11), (2, 201), (4, 40), (6, 61)]),
        ("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED AND e.id = 2 THEN UPDATE SET e.value = i.value WHEN MATCHED AND e.id = 6 THEN DELETE WHEN NOT MATCHED THEN INSERT (id, value) VALUES (i.id, i.value)", &[(1, 11), (2, 200), (3, 300), (4, 40)]),
    ];
    for (sql, expected) in cases {
        let before = storage.snapshots(&ident).await?;
        let result = db.execute(sql).await?;
        assert!(result.batches.is_empty());
        assert_eq!(result.outcome.cost_units_charged, 0);
        let records = db.cost_records().await?;
        let record = records.last().unwrap();
        assert_eq!(record.seq, result.outcome.cost_record_seq);
        assert_eq!(record.status, opensnow_engine::CostRecordStatus::Completed);
        assert_eq!(record.rows_returned, 0);
        assert_eq!(record.bytes_returned, 0);
        assert_eq!(record.elapsed, result.outcome.elapsed);
        assert_eq!(storage.snapshots(&ident).await?.len(), before.len() + 1, "{sql}");
        let rows = db.execute("SELECT id, value FROM events ORDER BY id").await?;
        let actual: Vec<(i64, i64)> = rows.batches.iter().flat_map(|batch| {
            let ids = batch.column(0).as_any().downcast_ref::<arrow::array::Int64Array>().unwrap();
            let values = batch.column(1).as_any().downcast_ref::<arrow::array::Int64Array>().unwrap();
            (0..batch.num_rows()).map(|i| (ids.value(i), values.value(i))).collect::<Vec<_>>()
        }).collect();
        assert_eq!(&actual, expected, "{sql}");
    }
    let before = storage.snapshots(&ident).await?;
    for sql in ["UPDATE events SET value = 0 WHERE id > 100", "DELETE FROM events WHERE id > 100",
        "MERGE INTO events AS e USING incoming AS i ON false WHEN MATCHED THEN DELETE"] {
        db.execute(sql).await?;
        assert_eq!(storage.snapshots(&ident).await?, before);
    }
    let original = db.execute("SELECT id, value FROM events ORDER BY id").await?.batches;
    db.execute("INSERT INTO incoming VALUES (2, 201)").await?;
    let error = db.execute("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN UPDATE SET e.value = i.value").await.unwrap_err();
    assert!(error.to_string().contains("more than once"), "{error}");
    assert_eq!(storage.snapshots(&ident).await?, before);
    let records = db.cost_records().await?;
    assert_eq!(records.last().unwrap().status, opensnow_engine::CostRecordStatus::Failed);
    assert_eq!(db.execute("SELECT id, value FROM events ORDER BY id").await?.batches, original);
    Ok(())
}

const MERGE_ITEMS: &str = "MERGE INTO items AS e USING (SELECT 1 AS id, 'Alicia' AS name) AS i ON e.id = i.id WHEN MATCHED THEN UPDATE SET e.name = i.name";

async fn check_mcp_merge(server: &McpServer, auth: Option<&str>) {
    let list = rpc(server, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}), auth).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    let write = tools.iter().find(|tool| tool["name"] == "execute_write").unwrap();
    assert_eq!(write["description"], "Run one write statement (create, insert, update, delete, merge, drop). Hosted: needs a write-scope token.");
    for sql in [CREATE, INSERT] {
        let out = call(server, "execute_write", json!({"sql": sql}), auth).await;
        assert!(out.get("__error").is_none(), "{out}");
    }
    let error = call_err(server, "query", json!({"sql": MERGE_ITEMS}), auth).await;
    assert!(error.contains("execute_write"), "{error}");
    let out = call(server, "execute_write", json!({"sql": MERGE_ITEMS}), auth).await;
    assert!(out.get("__error").is_none(), "{out}");
    let rows = call(server, "query", json!({"sql": "SELECT id, name FROM items ORDER BY id"}), auth).await;
    assert_eq!(rows["rows"], json!([["1", "Alicia"], ["2", "Bob"], ["3", "Carol"]]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_mcp_merge_and_query_split() -> TestResult {
    let temp = tempfile::tempdir()?;
    let server = McpServer::local(LocalDb::open(temp.path().join("local"), None).await?);
    check_mcp_merge(&server, None).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_mcp_merge_and_scopes() -> TestResult {
    let Some(fx) = setup("merge").await? else { return Ok(()); };
    let p = fx.plane.provision_tenant("merge", "admin@merge.test", "s3cret-admin-pass", None, None).await?;
    let server = McpServer::hosted(fx.plane.clone());
    check_mcp_merge(&server, Some(&p.token)).await;
    let read = fx.plane.issue_token(&p.tenant.id, &p.admin_user.id,
        vec![opensnow_control::TokenScope::Read], None).await?;
    let error = call_err(&server, "execute_write", json!({"sql": MERGE_ITEMS}), Some(&read.token)).await;
    assert!(error.contains("scope"), "{error}");
    let rows = call(&server, "query", json!({"sql": "SELECT name FROM items WHERE id = 1"}), Some(&read.token)).await;
    assert_eq!(rows["rows"], json!([["Alicia"]]));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_mutation_checks_cap_before_storage_runs() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = LocalDb::open(temp.path().join("local"), Some(0)).await?;
    for sql in ["DELETE FROM missing WHERE id = 1", "UPDATE missing SET id = 1",
        "MERGE INTO missing AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN DELETE"] {
        assert!(db.execute(sql).await.unwrap_err().to_string().contains("CostCapExceeded"));
    }
    assert!(db.cost_records().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn local_analytics_uses_catalog_metadata_after_reopen() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = LocalDb::open(temp.path(), None).await?;
    let worksheet = db.save_worksheet(None, "Events", "SELECT 1 AS x, 2 AS y").await?;
    let dashboard = db.save_dashboard(None, "Overview", std::slice::from_ref(&worksheet)).await?;
    let result = db.refresh_dashboard_tile(&dashboard.id, &worksheet).await?;
    let server = McpServer::local(db);
    let spec = json!({"mark": "point", "encoding": {
        "x": {"field": "x", "type": "quantitative"},
        "y": {"field": "y", "type": "quantitative"}
    }});
    let args = json!({"worksheet_id": worksheet, "chart_spec": spec});
    assert_eq!(call(&server, "set_chart_spec", args.clone(), None).await, args);
    drop(server);

    let server = McpServer::local(LocalDb::open(temp.path(), None).await?);
    assert_eq!(call(&server, "get_chart_spec", json!({"worksheet_id": worksheet}), None).await, args);
    let list = call(&server, "list_dashboards", json!({}), None).await;
    assert_eq!(list["dashboards"][0]["id"], dashboard.id);
    let detail = call(&server, "get_dashboard", json!({"dashboard_id": dashboard.id}), None).await;
    assert_eq!(detail["tiles"][0]["id"], worksheet);
    assert_eq!(detail["tiles"][0]["chart_spec"], spec);
    assert_eq!(detail["tiles"][0]["latest_result"], result);
    Ok(())
}
