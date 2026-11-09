//! HTTP transport for the hosted control plane. This is the transport
//! the worksheets UI (issue #11) builds on, plus the hosted MCP
//! endpoint. Minimal HTTP/1.1: one request per connection, JSON in and
//! out, Bearer token auth from the control plane. No TLS here; a hosted
//! deploy terminates TLS in front of this listener.

use std::sync::Arc;

use opensnow_control::{ControlError, ControlPlane};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::mcp::McpServer;
use crate::{query_result_to_json, record_to_json, summary_to_json};

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

async fn route(server: &HttpServer, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> (u16, Value) {
    let json_body: Value = if body.is_empty() { json!({}) } else {
        match serde_json::from_slice(body) { Ok(v) => v, Err(e) => return (400, json!({"error": format!("bad json: {e}")})) }
    };
    match (method, path) {
        ("GET", "/health") => (200, json!({"status": "ok"})),
        ("POST", "/api/provision") => {
            let (name, login, password) = (json_body["name"].as_str(), json_body["admin_login"].as_str(), json_body["admin_password"].as_str());
            let (Some(name), Some(login), Some(password)) = (name, login, password) else {
                return (400, json!({"error": "name, admin_login, and admin_password are required"}));
            };
            let start = std::time::Instant::now();
            match server.plane.provision_tenant(name, login, password, json_body["cost_cap_units"].as_u64(), None).await {
                Ok(p) => (200, json!({"tenant_id": p.tenant.id, "tenant_name": p.tenant.name,
                    "namespace": p.tenant.namespace, "token": p.token,
                    "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0})),
                Err(e) => (status_for(&e), json!({"error": e.to_string()})),
            }
        }
        ("POST", "/mcp") => {
            let text = String::from_utf8_lossy(body).into_owned();
            match server.mcp.handle_jsonrpc(&text, bearer(headers)).await {
                Some(reply) => (200, serde_json::from_str(&reply).unwrap_or(json!({}))),
                None => (202, json!({})),
            }
        }
        ("POST", "/api/execute") => {
            let Some(token) = bearer(headers) else { return (401, json!({"error": "authentication failed"})); };
            let Some(sql) = json_body["sql"].as_str() else { return (400, json!({"error": "sql is required"})); };
            match server.plane.execute_authenticated(token, sql).await {
                Ok(result) => (200, query_result_to_json(&result)),
                Err(e) => (status_for(&e), json!({"error": e.to_string()})),
            }
        }
        ("GET", "/api/tables") => {
            let Some(token) = bearer(headers) else { return (401, json!({"error": "authentication failed"})); };
            match server.plane.list_tables(token).await {
                Ok(tables) => (200, json!({"tables": tables})),
                Err(e) => (status_for(&e), json!({"error": e.to_string()})),
            }
        }
        ("GET", "/api/cost-records") => {
            let Some(token) = bearer(headers) else { return (401, json!({"error": "authentication failed"})); };
            match server.plane.cost_records(token).await {
                Ok(records) => (200, json!({"records": records.iter().map(record_to_json).collect::<Vec<_>>() })),
                Err(e) => (status_for(&e), json!({"error": e.to_string()})),
            }
        }
        ("GET", "/api/cost-summary") => {
            let Some(token) = bearer(headers) else { return (401, json!({"error": "authentication failed"})); };
            match server.plane.cost_summary(token).await {
                Ok(summary) => (200, summary_to_json(&summary)),
                Err(e) => (status_for(&e), json!({"error": e.to_string()})),
            }
        }
        _ => (404, json!({"error": "not found"})),
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
    let (status, value) = route(server, &method, &path, &headers, &body).await;
    let payload = value.to_string();
    let reason = match status { 200 => "OK", 202 => "Accepted", 400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found", 409 => "Conflict", _ => "Internal Server Error" };
    let response = format!("HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", payload.len(), payload);
    let _ = stream.write_all(response.as_bytes()).await;
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
