//! opensnow-mcp binary.
//!
//! Default: a local stdio MCP server over a local-mode database
//! (zero account, zero credentials). Flags:
//!   --local <dir>     local database directory (default ./opensnow-local)
//!   --cost-cap <n>    per-database cost cap in engine cost units
//!   --http <addr>     hosted HTTP transport instead of stdio; needs
//!                     OPENSNOW_CONTROL_DATABASE_URL (postgres URI) and
//!                     OPENSNOW_WAREHOUSE (local warehouse path)

use std::sync::Arc;

use opensnow_control::{ControlPlane, ControlStore};
use opensnow_mcp::http::{serve, HttpServer};
use opensnow_mcp::{LocalDb, McpServer};
use opensnow_storage::CatalogConfig;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), BoxError> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone());

    if let Some(addr) = flag("--http") {
        let uri = std::env::var("OPENSNOW_CONTROL_DATABASE_URL")
            .map_err(|_| -> BoxError { "OPENSNOW_CONTROL_DATABASE_URL is required for --http".into() })?;
        let warehouse = std::env::var("OPENSNOW_WAREHOUSE").unwrap_or_else(|_| "opensnow-warehouse".into());
        let pool = sqlx::PgPool::connect(&uri).await?;
        let store = ControlStore::new(pool);
        store.ensure_schema().await.map_err(|e| -> BoxError { e.to_string().into() })?;
        let storage = CatalogConfig::postgres("opensnow-http", uri, warehouse).connect().await.map_err(|e| -> BoxError { e })?;
        let plane = Arc::new(ControlPlane::new(store, Arc::new(storage)));
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        eprintln!("opensnow http transport listening on {addr}");
        serve(listener, HttpServer::new(plane)).await?;
        return Ok(());
    }

    let dir = flag("--local").unwrap_or_else(|| "opensnow-local".into());
    let cap = flag("--cost-cap").and_then(|v| v.parse::<u64>().ok());
    let db = LocalDb::open(dir, cap).await?;
    let server = McpServer::local(db);
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() { continue; }
        if let Some(reply) = server.handle_jsonrpc(&line, None).await {
            stdout.write_all(reply.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
