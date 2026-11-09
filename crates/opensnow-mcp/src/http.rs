//! HTTP transport for the hosted control plane. This is the transport
//! the worksheets UI (issue #11) builds on, plus the hosted MCP
//! endpoint. Minimal HTTP/1.1: one request per connection, JSON in and
//! out, Bearer token auth from the control plane. The one exception is
//! the UI itself: `GET /` serves a single static HTML page with no
//! external assets. No TLS here; a hosted deploy terminates TLS in
//! front of this listener.

use std::sync::Arc;

use opensnow_control::{ControlError, ControlPlane, WorksheetRecord};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::mcp::McpServer;
use crate::{query_result_to_json, record_to_json, summary_to_json};

/// The worksheets and navigation UI (issue #11). One embedded page; the
/// design package it implements is the approved mockups for issue #11.
const UI_HTML: &str = include_str!("ui.html");

pub struct HttpServer {
    plane: Arc<ControlPlane>,
    mcp: McpServer,
}

impl HttpServer {
    pub fn new(plane: Arc<ControlPlane>) -> Self {
        let mcp = McpServer::hosted(plane.clone());
        Self { plane, mcp }
    }
}

fn status_for(error: &ControlError) -> u16 {
    match error {
        ControlError::AuthFailed => 401,
        ControlError::AccessDenied(_) | ControlError::CostCapExceeded => 403,
        ControlError::DuplicateTenant | ControlError::DuplicateUser => 409,
        ControlError::InvalidInput(_) => 400,
        ControlError::NotFound => 404,
        _ => 500,
    }
}

fn bearer(headers: &[(String, String)]) -> Option<&str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, v)| v.strip_prefix("Bearer "))
}

fn worksheet_to_json(w: &WorksheetRecord) -> Value {
    json!({
        "id": w.id, "name": w.name, "sql": w.sql_text,
        "created_at": w.created_at, "updated_at": w.updated_at,
    })
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn json_response(status: u16, value: Value) -> Response {
    Response { status, content_type: "application/json", body: value.to_string() }
}

fn error_response(error: &ControlError) -> Response {
    json_response(status_for(error), json!({"error": error.to_string()}))
}

fn auth_failed() -> Response {
    json_response(401, json!({"error": "authentication failed"}))
}

async fn route(server: &HttpServer, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> Response {
    if matches!((method, path), ("GET", "/") | ("GET", "/ui")) {
        return Response { status: 200, content_type: "text/html; charset=utf-8", body: UI_HTML.to_string() };
    }
    let json_body: Value = if body.is_empty() { json!({}) } else {
        match serde_json::from_slice(body) { Ok(v) => v, Err(e) => return json_response(400, json!({"error": format!("bad json: {e}")})) }
    };
    match (method, path) {
        ("GET", "/health") => json_response(200, json!({"status": "ok"})),
        ("POST", "/api/provision") => {
            let (name, login, password) = (json_body["name"].as_str(), json_body["admin_login"].as_str(), json_body["admin_password"].as_str());
            let (Some(name), Some(login), Some(password)) = (name, login, password) else {
                return json_response(400, json!({"error": "name, admin_login, and admin_password are required"}));
            };
            let start = std::time::Instant::now();
            match server.plane.provision_tenant(name, login, password, json_body["cost_cap_units"].as_u64(), None).await {
                Ok(p) => json_response(200, json!({"tenant_id": p.tenant.id, "tenant_name": p.tenant.name,
                    "namespace": p.tenant.namespace, "token": p.token,
                    "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0})),
                Err(e) => error_response(&e),
            }
        }
        // Password sign-in for the UI. On success the caller gets a
        // read-scope token (the password principal's scope, nothing
        // more). Every failure is the same opaque 401.
        ("POST", "/api/sign-in") => {
            let (tenant, login, password) = (json_body["tenant"].as_str(), json_body["login"].as_str(), json_body["password"].as_str());
            let (Some(tenant), Some(login), Some(password)) = (tenant, login, password) else {
                return json_response(400, json!({"error": "tenant, login, and password are required"}));
            };
            match server.plane.authenticate_password(tenant, login, password).await {
                Ok(principal) => match server.plane.issue_token(&principal.tenant_id, &principal.user_id, vec![opensnow_control::TokenScope::Read], None).await {
                    Ok(issued) => json_response(200, json!({"token": issued.token,
                        "tenant_name": principal.tenant_name, "namespace": principal.namespace,
                        "login": principal.login})),
                    Err(e) => error_response(&e),
                },
                Err(_) => auth_failed(),
            }
        }
        ("POST", "/mcp") => {
            let text = String::from_utf8_lossy(body).into_owned();
            match server.mcp.handle_jsonrpc(&text, bearer(headers)).await {
                Some(reply) => json_response(200, serde_json::from_str(&reply).unwrap_or(json!({}))),
                None => json_response(202, json!({})),
            }
        }
        ("POST", "/api/execute") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(sql) = json_body["sql"].as_str() else { return json_response(400, json!({"error": "sql is required"})); };
            match server.plane.execute_authenticated(token, sql).await {
                Ok(result) => json_response(200, query_result_to_json(&result)),
                Err(e) => error_response(&e),
            }
        }
        ("GET", "/api/tables") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            match server.plane.list_tables(token).await {
                Ok(tables) => json_response(200, json!({"tables": tables})),
                Err(e) => error_response(&e),
            }
        }
        // Column detail for one of the caller's tables, read from the
        // catalog in the caller's own namespace.
        ("POST", "/api/describe") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(table) = json_body["table"].as_str() else { return json_response(400, json!({"error": "table is required"})); };
            match server.plane.describe_table(token, table).await {
                Ok(columns) => json_response(200, json!({"columns": columns.iter()
                    .map(|c| json!({"name": c.name, "data_type": c.data_type})).collect::<Vec<_>>() })),
                Err(e) => error_response(&e),
            }
        }
        // Catalog search across the caller's table names and column
        // names, case-insensitive substring, in the caller's namespace
        // only. A table matches when its name or any column matches.
        ("POST", "/api/search") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(query) = json_body["query"].as_str() else { return json_response(400, json!({"error": "query is required"})); };
            let needle = query.trim().to_lowercase();
            if needle.is_empty() { return json_response(200, json!({"matches": []})); }
            let tables = match server.plane.list_tables(token).await {
                Ok(t) => t,
                Err(e) => return error_response(&e),
            };
            let mut matches = Vec::new();
            for table in tables {
                let columns = server.plane.describe_table(token, &table).await.unwrap_or_default();
                let matched_columns: Vec<String> = columns.iter()
                    .filter(|c| c.name.to_lowercase().contains(&needle))
                    .map(|c| c.name.clone()).collect();
                if table.to_lowercase().contains(&needle) || !matched_columns.is_empty() {
                    matches.push(json!({"table": table, "columns": matched_columns}));
                }
            }
            json_response(200, json!({"matches": matches}))
        }
        ("GET", "/api/worksheets") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            match server.plane.list_worksheets(token).await {
                Ok(list) => json_response(200, json!({"worksheets": list.iter().map(worksheet_to_json).collect::<Vec<_>>() })),
                Err(e) => error_response(&e),
            }
        }
        ("POST", "/api/worksheets") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(name) = json_body["name"].as_str() else { return json_response(400, json!({"error": "name is required"})); };
            let sql = json_body["sql"].as_str().unwrap_or("");
            match server.plane.save_worksheet(token, json_body["id"].as_str(), name, sql).await {
                Ok(w) => json_response(200, worksheet_to_json(&w)),
                Err(e) => error_response(&e),
            }
        }
        ("POST", "/api/worksheets/delete") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(id) = json_body["id"].as_str() else { return json_response(400, json!({"error": "id is required"})); };
            match server.plane.delete_worksheet(token, id).await {
                Ok(()) => json_response(200, json!({"deleted": id})),
                Err(e) => error_response(&e),
            }
        }
        // Who the caller's token belongs to. The UI uses this to show
        // the tenant and login for a token session.
        ("GET", "/api/session") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            match server.plane.authenticate_token(token).await {
                Ok(p) => json_response(200, json!({"tenant_name": p.tenant_name,
                    "namespace": p.namespace, "login": p.login,
                    "scopes": p.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>() })),
                Err(e) => error_response(&e),
            }
        }
        ("GET", "/api/cost-records") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            match server.plane.cost_records(token).await {
                Ok(records) => json_response(200, json!({"records": records.iter().map(record_to_json).collect::<Vec<_>>() })),
                Err(e) => error_response(&e),
            }
        }
        ("GET", "/api/cost-summary") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            match server.plane.cost_summary(token).await {
                Ok(summary) => json_response(200, summary_to_json(&summary)),
                Err(e) => error_response(&e),
            }
        }
        _ => json_response(404, json!({"error": "not found"})),
    }
}

async fn handle(server: &Arc<HttpServer>, mut stream: TcpStream) {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        match stream.read(&mut chunk).await {
            Ok(0) => return,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") { break pos + 4; }
                if buffer.len() > 1_000_000 { return; }
            }
            Err(_) => return,
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().split('?').next().unwrap_or_default().to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => return,
        }
    }
    body.truncate(content_length);
    let response = route(server, &method, &path, &headers, &body).await;
    let reason = match response.status { 200 => "OK", 202 => "Accepted", 400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found", 409 => "Conflict", _ => "Internal Server Error" };
    let wire = format!("HTTP/1.1 {} {reason}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.status, response.content_type, response.body.len(), response.body);
    let _ = stream.write_all(wire.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Serves until the listener is dropped or the process stops.
pub async fn serve(listener: TcpListener, server: HttpServer) -> std::io::Result<()> {
    let server = Arc::new(server);
    loop {
        let (stream, _) = listener.accept().await?;
        let shared = server.clone();
        tokio::spawn(async move { handle(&shared, stream).await; });
    }
}
