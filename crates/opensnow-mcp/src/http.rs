//! HTTP transport for the hosted control plane. This is the transport
//! the worksheets UI (issue #11) builds on, plus the hosted MCP
//! endpoint. Minimal HTTP/1.1: one request per connection, JSON in and
//! out, Bearer token auth from the control plane. The one exception is
//! the UI itself: `GET /` serves a single static HTML page with no
//! external assets. No TLS here; a hosted deploy terminates TLS in
//! front of this listener.

use std::sync::Arc;

use opensnow_control::{ControlError, ControlPlane};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::analytics::{dashboard_json, worksheet_json};
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

async fn refresh_tile(
    plane: &ControlPlane, token: &str, worksheet: &opensnow_control::WorksheetRecord,
) -> (u16, Value) {
    let mut record = None;
    let result = plane.execute_authenticated_with_record(token, &worksheet.sql_text, &mut record).await;
    let error = match result {
        Ok(result) => {
            let mut data = query_result_to_json(&result);
            data["refreshed_at"] = json!(record.as_ref().map(|r| r.started_at_unix_ms + r.elapsed.as_millis() as u64));
            match plane.save_tile_result(token, worksheet, &data, result.outcome.cost_record_seq).await {
                Ok(()) => {
                    data["chart_spec"] = json!(worksheet.chart_spec);
                    return (200, data);
                }
                Err(error) => error,
            }
        }
        Err(error) => error,
    };
    if matches!(error, ControlError::AuthFailed) {
        return (401, json!({"error": "authentication failed"}));
    }
    (status_for(&error), json!({
        "error": error.to_string(), "cost_cap": matches!(error, ControlError::CostCapExceeded),
        "cost_record": record.as_ref().map(record_to_json),
    }))
}

async fn route(server: &HttpServer, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> Response {
    if matches!((method, path), ("GET", "/") | ("GET", "/ui")) {
        return Response { status: 200, content_type: "text/html; charset=utf-8", body: UI_HTML.to_string() };
    }
    let asset = match (method, path) {
        ("GET", "/assets/vega.js") => Some(include_str!("../../../vendor/vega/vega.min.js")),
        ("GET", "/assets/vega-lite.js") => Some(include_str!("../../../vendor/vega/vega-lite.min.js")),
        ("GET", "/assets/vega-embed.js") => Some(include_str!("../../../vendor/vega/vega-embed.min.js")),
        ("GET", "/assets/analytics.js") => Some(include_str!("analytics.js")),
        _ => None,
    };
    if let Some(asset) = asset {
        return Response { status: 200, content_type: "text/javascript; charset=utf-8", body: asset.into() };
    }
    if let ["", "api", "dashboards", id] | ["", "api", "chart-spec", id] = path.split('/').collect::<Vec<_>>().as_slice() {
        if method == "GET" {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            return if path.starts_with("/api/dashboards/") {
                match server.plane.dashboard(token, id).await {
                    Ok((dashboard, tiles)) => json_response(200, dashboard_json(dashboard, tiles)),
                    Err(error) => error_response(&error),
                }
            } else {
                match server.plane.worksheet(token, id).await {
                    Ok(worksheet) => json_response(200, json!({"worksheet_id": id, "chart_spec": worksheet.chart_spec})),
                    Err(error) => error_response(&error),
                }
            };
        }
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
                Ok(list) => json_response(200, json!({"worksheets": list.iter().map(worksheet_json).collect::<Vec<_>>() })),
                Err(e) => error_response(&e),
            }
        }
        ("POST", "/api/worksheets") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(name) = json_body["name"].as_str() else { return json_response(400, json!({"error": "name is required"})); };
            let sql = json_body["sql"].as_str().unwrap_or("");
            let chart = json_body.get("chart_spec").map(|spec| (!spec.is_null()).then(|| spec.clone()));
            match server.plane.save_worksheet_with_chart(token, json_body["id"].as_str(), name, sql, chart).await {
                Ok(w) => json_response(200, worksheet_json(&w)),
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
        ("GET", "/api/dashboards") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            match server.plane.list_dashboards(token).await {
                Ok(dashboards) => json_response(200, json!({"dashboards": dashboards})),
                Err(error) => error_response(&error),
            }
        }
        ("POST", "/api/dashboards") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(name) = json_body["name"].as_str() else { return json_response(400, json!({"error": "name is required"})); };
            let Ok(ids) = serde_json::from_value::<Vec<String>>(json_body["worksheet_ids"].clone()) else {
                return json_response(400, json!({"error": "worksheet_ids must be an array of strings"}));
            };
            match server.plane.save_dashboard(token, json_body["id"].as_str(), name, &ids).await {
                Ok(dashboard) => json_response(200, json!(dashboard)),
                Err(error) => error_response(&error),
            }
        }
        ("POST", "/api/dashboards/delete") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(id) = json_body["id"].as_str() else { return json_response(400, json!({"error": "id is required"})); };
            match server.plane.delete_dashboard(token, id).await {
                Ok(()) => json_response(200, json!({"deleted": id})),
                Err(error) => error_response(&error),
            }
        }
        ("POST", "/api/chart-spec") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let (Some(id), Some(spec)) = (json_body["worksheet_id"].as_str(), json_body.get("chart_spec")) else {
                return json_response(400, json!({"error": "worksheet_id and chart_spec are required"}));
            };
            let chart = (!spec.is_null()).then(|| spec.clone());
            match server.plane.set_chart_spec(token, id, chart, false).await {
                Ok(()) => json_response(200, json!({"worksheet_id": id, "chart_spec": spec})),
                Err(error) => error_response(&error),
            }
        }
        ("POST", "/api/dashboards/refresh-tile") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let (Some(dashboard_id), Some(worksheet_id)) = (json_body["dashboard_id"].as_str(), json_body["worksheet_id"].as_str()) else {
                return json_response(400, json!({"error": "dashboard_id and worksheet_id are required"}));
            };
            let worksheet = match server.plane.tile_worksheet(token, dashboard_id, worksheet_id).await {
                Ok(worksheet) => worksheet,
                Err(error) => return error_response(&error),
            };
            let (status, data) = refresh_tile(&server.plane, token, &worksheet).await;
            json_response(status, data)
        }
        ("POST", "/api/dashboards/refresh") => {
            let Some(token) = bearer(headers) else { return auth_failed(); };
            let Some(id) = json_body["dashboard_id"].as_str() else {
                return json_response(400, json!({"error": "dashboard_id is required"}));
            };
            let (dashboard, tiles) = match server.plane.dashboard(token, id).await {
                Ok(data) => data,
                Err(error) => return error_response(&error),
            };
            let mut results = Vec::with_capacity(tiles.len());
            let mut total = 0.0;
            let mut metered = false;
            for tile in tiles {
                let (status, data) = refresh_tile(&server.plane, token, &tile.worksheet).await;
                if status == 401 { return auth_failed(); }
                if let Some(cost) = data["outcome"]["cost_dollars"].as_f64().or_else(|| data["cost_record"]["cost_dollars"].as_f64()) {
                    total += cost;
                    metered = true;
                }
                results.push(json!({"worksheet_id": tile.worksheet.id, "status": status, "data": data}));
            }
            let refreshed_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_millis() as u64;
            let summary = json!({"refreshed_at": refreshed_at, "cost_dollars": metered.then_some(total)});
            if !results.is_empty() {
                if let Err(error) = server.plane.save_dashboard_refresh(token, &dashboard, &summary).await {
                    return error_response(&error);
                }
            }
            json_response(200, json!({"tiles": results, "last_refresh": summary}))
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
