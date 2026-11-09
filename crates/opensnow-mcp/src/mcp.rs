//! MCP server: JSON-RPC 2.0 with the tool set from SOLUTION_DESIGN.md
//! Section 6.3. One handler serves both transports: stdio over a local
//! database, and HTTP over the hosted control plane.
//!
//! Safety shape: `query` takes read statements only, `execute_write`
//! takes write statements only, and the hosted path still runs every
//! call through the control plane's token scope check. The default
//! token scope in the control plane is read, so a fresh agent token
//! cannot write until a write token is issued.

use std::sync::Arc;
use std::time::Instant;

use opensnow_control::{Access, ControlPlane};
use serde_json::{json, Value};

use crate::local::LocalDb;
use crate::{query_result_to_json, record_to_json, summary_to_json};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

enum Mode {
    Local(LocalDb),
    Hosted(Arc<ControlPlane>),
}

pub struct McpServer {
    mode: Mode,
}

const DOCS: &[(&str, &str)] = &[
    ("COMPATIBILITY.md", include_str!("../../../docs/COMPATIBILITY.md")),
    ("COST-MODEL.md", include_str!("../../../docs/COST-MODEL.md")),
    ("ENGINE-SERVICE.md", include_str!("../../../docs/ENGINE-SERVICE.md")),
    ("CONTROL-PLANE.md", include_str!("../../../docs/CONTROL-PLANE.md")),
    ("METERING.md", include_str!("../../../docs/METERING.md")),
    ("MCP.md", include_str!("../../../docs/MCP.md")),
    ("SOLUTION_DESIGN.md", include_str!("../../../SOLUTION_DESIGN.md")),
];

fn tool_defs() -> Value {
    let sql = json!({"type": "object", "properties": {"sql": {"type": "string"}}, "required": ["sql"]});
    let table = json!({"type": "object", "properties": {"table": {"type": "string"}}, "required": ["table"]});
    json!([
        {"name": "provision_database", "description": "Create a database and return its scoped token. Hosted: provisions a tenant in under 5 seconds. Local: returns the open local database.", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "admin_login": {"type": "string"}, "admin_password": {"type": "string"}, "cost_cap_units": {"type": "integer"}}}},
        {"name": "list_tables", "description": "List the tables in the caller's namespace.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "describe_table", "description": "Column names and types for one table.", "inputSchema": table},
        {"name": "search_schema", "description": "Find tables and columns whose names contain the query text, case-insensitive.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}},
        {"name": "sample_rows", "description": "Read up to limit sample rows from one table (default 10, max 100).", "inputSchema": {"type": "object", "properties": {"table": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["table"]}},
        {"name": "query", "description": "Run one read-only SQL statement. Write statements are refused; use execute_write.", "inputSchema": sql},
        {"name": "execute_write", "description": "Run one write statement (create, insert, update, delete, drop). Hosted: needs a write-scope token.", "inputSchema": sql},
        {"name": "cost_records", "description": "The caller's query log: every executed query with its cost in dollars.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "cost_summary", "description": "The caller's cost totals across all recorded queries.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "docs_search", "description": "Search the shipped opensnow docs and return matching lines.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}},
    ])
}

impl McpServer {
    pub fn local(db: LocalDb) -> Self { Self { mode: Mode::Local(db) } }
    pub fn hosted(plane: Arc<ControlPlane>) -> Self { Self { mode: Mode::Hosted(plane) } }

    fn require_token<'a>(&self, auth: Option<&'a str>) -> Result<Option<&'a str>, BoxError> {
        match self.mode {
            Mode::Local(_) => Ok(None),
            Mode::Hosted(_) => auth.filter(|t| !t.is_empty()).map(Some)
                .ok_or_else(|| -> BoxError { "authentication failed".into() }),
        }
    }

    async fn execute(&self, auth: Option<&str>, sql: &str, want: Access) -> Result<Value, BoxError> {
        // Classify first with the same guard both modes share, so the
        // read/write tool split holds before any token is spent.
        let access = match &self.mode {
            Mode::Local(db) => db.classify(sql)?,
            // The hosted guard needs the caller's real namespace, which
            // only execute_authenticated knows. Classification here is
            // statement kind only; execute re-runs the full guard with
            // the real namespace before anything runs.
            Mode::Hosted(_) => classify_kind(sql)?,
        };
        if access != want {
            return Err(match want {
                Access::Read => "query takes read statements only; use execute_write for writes".into(),
                Access::Write => "execute_write takes write statements only; use query for reads".into(),
            });
        }
        let result = match &self.mode {
            Mode::Local(db) => db.execute(sql).await?,
            Mode::Hosted(plane) => {
                let token = self.require_token(auth)?.unwrap();
                plane.execute_authenticated(token, sql).await.map_err(|e| -> BoxError { e.to_string().into() })?
            }
        };
        Ok(query_result_to_json(&result))
    }

    async fn list_tables(&self, auth: Option<&str>) -> Result<Value, BoxError> {
        let names = match &self.mode {
            Mode::Local(db) => db.list_tables().await?,
            Mode::Hosted(plane) => {
                let token = self.require_token(auth)?.unwrap();
                plane.list_tables(token).await.map_err(|e| -> BoxError { e.to_string().into() })?
            }
        };
        Ok(json!({"tables": names}))
    }

    async fn describe(&self, auth: Option<&str>, table: &str) -> Result<Value, BoxError> {
        if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || table.is_empty() {
            return Err("bad table name".into());
        }
        let columns: Vec<Value> = match &self.mode {
            Mode::Local(db) => db.describe_table(table).await?.iter()
                .map(|(name, data_type)| json!({"name": name, "type": data_type})).collect(),
            Mode::Hosted(plane) => {
                let token = self.require_token(auth)?.unwrap();
                plane.describe_table(token, table).await.map_err(|e| -> BoxError { e.to_string().into() })?
                    .iter().map(|c| json!({"name": c.name, "type": c.data_type})).collect()
            }
        };
        Ok(json!({"table": table, "columns": columns}))
    }

    async fn call_tool(&self, name: &str, args: &Value, auth: Option<&str>) -> Result<Value, BoxError> {
        match name {
            "provision_database" => {
                match &self.mode {
                    Mode::Local(db) => Ok(json!({
                        "mode": "local", "namespace": crate::local::LOCAL_NAMESPACE,
                        "path": db.root.display().to_string(),
                        "note": "local mode needs no account and no token",
                    })),
                    Mode::Hosted(plane) => {
                        let tenant_name = args["name"].as_str().ok_or("name is required")?;
                        let login = args["admin_login"].as_str().ok_or("admin_login is required")?;
                        let password = args["admin_password"].as_str().ok_or("admin_password is required")?;
                        let cap = args["cost_cap_units"].as_u64();
                        let start = Instant::now();
                        let p = plane.provision_tenant(tenant_name, login, password, cap, None).await
                            .map_err(|e| -> BoxError { e.to_string().into() })?;
                        Ok(json!({
                            "mode": "hosted", "tenant_id": p.tenant.id, "tenant_name": p.tenant.name,
                            "namespace": p.tenant.namespace, "token": p.token,
                            "token_scopes": ["write"], "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0,
                            "note": "the token is shown once and never stored in plaintext",
                        }))
                    }
                }
            }
            "list_tables" => self.list_tables(auth).await,
            "describe_table" => self.describe(auth, args["table"].as_str().ok_or("table is required")?).await,
            "search_schema" => {
                let query = args["query"].as_str().ok_or("query is required")?.to_ascii_lowercase();
                let tables = self.list_tables(auth).await?;
                let mut hits = Vec::new();
                for table in tables["tables"].as_array().cloned().unwrap_or_default() {
                    let table = table.as_str().unwrap_or_default().to_string();
                    if table.to_ascii_lowercase().contains(&query) { hits.push(json!({"table": table, "column": Value::Null})); }
                    if let Ok(desc) = self.describe(auth, &table).await {
                        if let Some(columns) = desc["columns"].as_array() {
                            for column in columns {
                                let col = column["name"].as_str().unwrap_or_default();
                                if col.to_ascii_lowercase().contains(&query) {
                                    hits.push(json!({"table": table, "column": col}));
                                }
                            }
                        }
                    }
                }
                Ok(json!({"matches": hits}))
            }
            "sample_rows" => {
                let table = args["table"].as_str().ok_or("table is required")?;
                if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || table.is_empty() {
                    return Err("bad table name".into());
                }
                let limit = args["limit"].as_u64().unwrap_or(10).min(100);
                self.execute(auth, &format!("SELECT * FROM {table} LIMIT {limit}"), Access::Read).await
            }
            "query" => self.execute(auth, args["sql"].as_str().ok_or("sql is required")?, Access::Read).await,
            "execute_write" => self.execute(auth, args["sql"].as_str().ok_or("sql is required")?, Access::Write).await,
            "cost_records" => {
                let records = match &self.mode {
                    Mode::Local(db) => db.cost_records().await?,
                    Mode::Hosted(plane) => {
                        let token = self.require_token(auth)?.unwrap();
                        plane.cost_records(token).await.map_err(|e| -> BoxError { e.to_string().into() })?
                    }
                };
                Ok(json!({"records": records.iter().map(record_to_json).collect::<Vec<_>>()}))
            }
            "cost_summary" => {
                let summary = match &self.mode {
                    Mode::Local(db) => db.cost_summary().await?,
                    Mode::Hosted(plane) => {
                        let token = self.require_token(auth)?.unwrap();
                        plane.cost_summary(token).await.map_err(|e| -> BoxError { e.to_string().into() })?
                    }
                };
                Ok(summary_to_json(&summary))
            }
            "docs_search" => {
                let query = args["query"].as_str().ok_or("query is required")?.to_ascii_lowercase();
                let mut hits = Vec::new();
                for (doc, text) in DOCS {
                    for line in text.lines() {
                        if line.to_ascii_lowercase().contains(&query) {
                            hits.push(json!({"doc": doc, "line": line.trim()}));
                            if hits.len() >= 20 { break; }
                        }
                    }
                    if hits.len() >= 20 { break; }
                }
                Ok(json!({"matches": hits}))
            }
            _ => Err(format!("unknown tool {name}").into()),
        }
    }

    /// Handles one JSON-RPC message. Returns None for notifications.
    pub async fn handle_jsonrpc(&self, body: &str, auth: Option<&str>) -> Option<String> {
        let request: Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some(error_reply(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default();
        if id.is_null() && !method.is_empty() {
            return None; // notification
        }
        let result: Result<Value, BoxError> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": request["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "opensnow", "version": "0.1.0"},
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_defs()})),
            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or_default().to_string();
                let args = request["params"]["arguments"].clone();
                match self.call_tool(&name, &args, auth).await {
                    Ok(value) => Ok(json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false})),
                    Err(e) => Ok(json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})),
                }
            }
            _ => return Some(error_reply(id, -32601, &format!("method not found: {method}"))),
        };
        match result {
            Ok(value) => Some(json!({"jsonrpc": "2.0", "id": id, "result": value}).to_string()),
            Err(e) => Some(error_reply(id, -32602, &e.to_string())),
        }
    }
}

/// Statement kind only, mirroring the control guard's allowed set.
fn classify_kind(sql: &str) -> Result<Access, BoxError> {
    use sqlparser::{dialect::GenericDialect, parser::Parser};
    if sql.contains("--") || sql.contains("/*") {
        return Err("comments are not accepted".into());
    }
    let mut statements = Parser::parse_sql(&GenericDialect {}, sql).map_err(|e| -> BoxError { e.to_string().into() })?;
    if statements.len() != 1 {
        return Err("exactly one statement is required".into());
    }
    match statements.pop().unwrap() {
        sqlparser::ast::Statement::Query(_) => Ok(Access::Read),
        sqlparser::ast::Statement::Insert(_) | sqlparser::ast::Statement::CreateTable(_)
        | sqlparser::ast::Statement::Drop { .. } | sqlparser::ast::Statement::Delete(_)
        | sqlparser::ast::Statement::Update { .. } => Ok(Access::Write),
        _ => Err("statement type is not allowed".into()),
    }
}

fn error_reply(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}
