//! Issue #11 UI surface tests: the served page, password sign-in, the
//! catalog describe and search routes, and server-side worksheets.
//! Hosted tests follow the control plane pattern and skip cleanly when
//! Postgres on localhost:5433 is unreachable.

use std::sync::Arc;

use opensnow_control::{ControlPlane, ControlStore};
use opensnow_mcp::http::{serve, HttpServer};
use opensnow_storage::CatalogConfig;
use serde_json::{json, Value};
use sqlx::ConnectOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

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
                println!("SKIP ui {label}: Postgres unreachable: {error}");
                return Ok(None);
            }
            return Err(error.into());
        }
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let db = format!("opensnow_ui_{label}_{}", &suffix[..8]);
    sqlx::query(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)")).execute(&mut admin).await?;
    sqlx::query(&format!("CREATE DATABASE {db}")).execute(&mut admin).await?;
    let uri = format!("postgres://postgres@localhost:5433/{db}");
    let pool = sqlx::PgPool::connect(&uri).await?;
    let store = ControlStore::new(pool);
    store.ensure_schema().await?;
    let temp = tempfile::tempdir()?;
    let storage = CatalogConfig::postgres("ui-test", uri.clone(), temp.path().join("warehouse"))
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

    async fn serve_http(&self) -> (std::net::SocketAddr, tokio::task::JoinHandle<std::io::Result<()>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(listener, HttpServer::new(self.plane.clone())));
        (addr, server)
    }
}

/// Returns (status, head, body). The head is kept so tests can check the
/// content type of the UI page.
async fn http_raw(addr: std::net::SocketAddr, request: &str) -> (u16, String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut parts = text.splitn(2, "\r\n\r\n");
    let head = parts.next().unwrap_or_default().to_string();
    let body = parts.next().unwrap_or_default().to_string();
    (status, head, body)
}

fn req(method: &str, path: &str, token: Option<&str>, body: &Value) -> String {
    let auth = token.map(|t| format!("authorization: Bearer {t}\r\n")).unwrap_or_default();
    format!("{method} {path} HTTP/1.1\r\nhost: x\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        body.to_string().len(), body)
}

async fn http_json(addr: std::net::SocketAddr, request: &str) -> (u16, Value) {
    let (status, _, body) = http_raw(addr, request).await;
    (status, serde_json::from_str(&body).unwrap_or(json!({})))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_page_sign_in_and_opaque_failures() -> TestResult {
    let Some(fx) = setup("signin").await? else { return Ok(()); };
    let (addr, server) = fx.serve_http().await;

    // The UI page itself: one static document with the sign-in state.
    let (status, head, body) = http_raw(addr, &req("GET", "/", None, &json!({}))).await;
    assert_eq!(status, 200);
    assert!(head.contains("text/html"), "UI content type said: {head}");
    assert!(body.contains("Sign in to your warehouse"), "sign-in state missing");
    assert!(body.contains("No results yet"), "worksheet empty state missing");
    assert!(body.contains("Query history"), "history view missing");
    assert!(body.contains("exact cost is calculated when it completes"), "Q5 running note missing");

    let (status, _) = http_json(addr, &req("GET", "/no-such-route", None, &json!({}))).await;
    assert_eq!(status, 404, "unknown routes stay 404");

    let provisioned = fx.plane.provision_tenant("uico", "admin@uico.test", "s3cret-admin-pass", None, None).await?;
    let write_token = provisioned.token.clone();
    let tenant_id = provisioned.tenant.id.clone();

    // Data through the write token, as an API caller would load it.
    for sql in [
        "CREATE TABLE items (id BIGINT NOT NULL, name VARCHAR, salary BIGINT)",
        "INSERT INTO items VALUES (1, 'Alice', 100), (2, 'Bob', 80)",
    ] {
        let (status, out) = http_json(addr, &req("POST", "/api/execute", Some(&write_token), &json!({"sql": sql}))).await;
        assert_eq!(status, 200, "execute {sql} said {out}");
    }

    // Sign-in failures are one identical opaque body: wrong password,
    // unknown tenant, unknown login.
    let mut failure_bodies = Vec::new();
    for (tenant, login, password) in [
        ("uico", "admin@uico.test", "wrong-pass"),
        ("nosuchtenant", "admin@uico.test", "s3cret-admin-pass"),
        ("uico", "nobody@uico.test", "s3cret-admin-pass"),
    ] {
        let (status, _, body) = http_raw(addr, &req("POST", "/api/sign-in", None,
            &json!({"tenant": tenant, "login": login, "password": password}))).await;
        assert_eq!(status, 401, "sign-in {tenant}/{login} must fail");
        failure_bodies.push(body);
    }
    assert!(failure_bodies.windows(2).all(|w| w[0] == w[1]),
        "sign-in failure bodies differ: {failure_bodies:?}");
    assert_eq!(failure_bodies[0], "{\"error\":\"authentication failed\"}");

    // Good sign-in returns a working read-scope token.
    let (status, signed) = http_json(addr, &req("POST", "/api/sign-in", None,
        &json!({"tenant": "uico", "login": "admin@uico.test", "password": "s3cret-admin-pass"}))).await;
    assert_eq!(status, 200, "sign-in said {signed}");
    let read_token = signed["token"].as_str().unwrap().to_string();
    assert_eq!(signed["tenant_name"], "uico");

    let (status, session) = http_json(addr, &req("GET", "/api/session", Some(&read_token), &json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(session["login"], "admin@uico.test");
    assert_eq!(session["scopes"], json!(["read"]));

    let (status, out) = http_json(addr, &req("POST", "/api/execute", Some(&read_token),
        &json!({"sql": "SELECT name FROM items ORDER BY id"}))).await;
    assert_eq!(status, 200, "read execute said {out}");
    assert_eq!(out["rows"], json!([["Alice"], ["Bob"]]));

    // A read token cannot write, at the control plane, not just in the UI.
    let (status, _) = http_json(addr, &req("POST", "/api/execute", Some(&read_token),
        &json!({"sql": "INSERT INTO items VALUES (3, 'Eve', 10)"}))).await;
    assert_eq!(status, 403, "read token write must be denied");

    // Suspending the tenant ends every session the same opaque way.
    fx.plane.suspend_tenant(&tenant_id).await?;
    let (status, _, body) = http_raw(addr, &req("POST", "/api/sign-in", None,
        &json!({"tenant": "uico", "login": "admin@uico.test", "password": "s3cret-admin-pass"}))).await;
    assert_eq!(status, 401);
    assert_eq!(body, failure_bodies[0], "suspended sign-in must be the same opaque failure");
    let (status, _) = http_json(addr, &req("GET", "/api/tables", Some(&read_token), &json!({}))).await;
    assert_eq!(status, 401, "a suspended tenant's token must stop working at once");

    server.abort();
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worksheets_and_catalog_search_are_tenant_scoped() -> TestResult {
    let Some(fx) = setup("worksheets").await? else { return Ok(()); };
    let (addr, server) = fx.serve_http().await;

    let a = fx.plane.provision_tenant("aco", "admin@aco.test", "s3cret-admin-pass", None, None).await?;
    let _b = fx.plane.provision_tenant("bco", "admin@bco.test", "s3cret-admin-pass", None, None).await?;
    let a_signin = fx.plane.sign_in("aco", "admin@aco.test", "s3cret-admin-pass").await?;
    let b_signin = fx.plane.sign_in("bco", "admin@bco.test", "s3cret-admin-pass").await?;
    let (a_token, b_token) = (a_signin.token.clone(), b_signin.token.clone());

    // Catalog: A has a table, B does not. Describe and search see only
    // the caller's namespace.
    for sql in [
        "CREATE TABLE items (id BIGINT NOT NULL, name VARCHAR, salary BIGINT)",
        "INSERT INTO items VALUES (1, 'Alice', 100)",
    ] {
        let (status, out) = http_json(addr, &req("POST", "/api/execute", Some(&a.token), &json!({"sql": sql}))).await;
        assert_eq!(status, 200, "execute {sql} said {out}");
    }
    let (status, out) = http_json(addr, &req("POST", "/api/describe", Some(&a_token), &json!({"table": "items"}))).await;
    assert_eq!(status, 200);
    let col_names: Vec<&str> = out["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(col_names, ["id", "name", "salary"]);

    let (status, out) = http_json(addr, &req("POST", "/api/search", Some(&a_token), &json!({"query": "SAL"}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["matches"], json!([{"table": "items", "columns": ["salary"]}]),
        "search is case-insensitive over names and columns: {out}");

    let (status, out) = http_json(addr, &req("POST", "/api/search", Some(&b_token), &json!({"query": "sal"}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["matches"], json!([]), "B must not see A's catalog: {out}");
    let (status, _) = http_json(addr, &req("POST", "/api/describe", Some(&b_token), &json!({"table": "items"}))).await;
    assert_eq!(status, 404, "B describing A's table must be not found");

    // Worksheets: create, list, update, delete, all scoped to the owner.
    let (status, saved) = http_json(addr, &req("POST", "/api/worksheets", Some(&a_token),
        &json!({"name": "Revenue check", "sql": "SELECT * FROM items"}))).await;
    assert_eq!(status, 200, "save said {saved}");
    let ws_id = saved["id"].as_str().unwrap().to_string();
    assert_eq!(saved["name"], "Revenue check");

    let (status, out) = http_json(addr, &req("GET", "/api/worksheets", Some(&a_token), &json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["worksheets"].as_array().unwrap().len(), 1);
    let (status, out) = http_json(addr, &req("GET", "/api/worksheets", Some(&b_token), &json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["worksheets"], json!([]), "B must not list A's worksheets");

    let (status, updated) = http_json(addr, &req("POST", "/api/worksheets", Some(&a_token),
        &json!({"id": ws_id, "name": "Revenue check v2", "sql": "SELECT id FROM items"}))).await;
    assert_eq!(status, 200);
    assert_eq!(updated["name"], "Revenue check v2");
    assert_eq!(updated["sql"], "SELECT id FROM items");

    // B cannot update or delete A's worksheet; both read as not found.
    let (status, _) = http_json(addr, &req("POST", "/api/worksheets", Some(&b_token),
        &json!({"id": ws_id, "name": "Hijack", "sql": "SELECT 1"}))).await;
    assert_eq!(status, 404, "B updating A's worksheet must be not found");
    let (status, _) = http_json(addr, &req("POST", "/api/worksheets/delete", Some(&b_token),
        &json!({"id": ws_id}))).await;
    assert_eq!(status, 404, "B deleting A's worksheet must be not found");

    let (status, _) = http_json(addr, &req("POST", "/api/worksheets/delete", Some(&a_token),
        &json!({"id": ws_id}))).await;
    assert_eq!(status, 200);
    let (status, out) = http_json(addr, &req("GET", "/api/worksheets", Some(&a_token), &json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(out["worksheets"], json!([]));

    // An empty name is a bad request, and no token is a 401.
    let (status, _) = http_json(addr, &req("POST", "/api/worksheets", Some(&a_token),
        &json!({"name": "  ", "sql": "SELECT 1"}))).await;
    assert_eq!(status, 400);
    let (status, _) = http_json(addr, &req("GET", "/api/worksheets", None, &json!({}))).await;
    assert_eq!(status, 401);

    server.abort();
    fx.teardown().await
}
