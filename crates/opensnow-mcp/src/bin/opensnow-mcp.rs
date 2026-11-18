//! opensnow-mcp binary.
//!
//! Default: a local stdio MCP server over a local-mode database
//! (zero account, zero credentials). Flags:
//!   --local <dir>     local database directory (default ./opensnow-local)
//!   --cost-cap <n>    per-database cost cap in engine cost units
//!   --pg <addr>       hosted PostgreSQL wire transport
//!   --http <addr>     hosted HTTP transport
//! Both hosted transports need
//!                     OPENSNOW_CONTROL_DATABASE_URL (postgres URI) and
//!                     OPENSNOW_WAREHOUSE (local warehouse path)

use std::sync::Arc;

use opensnow_control::{ControlPlane, ControlStore};
use opensnow_mcp::http::{serve, HttpServer};
use opensnow_mcp::pgwire::{load_tls_config, serve_pg};
use opensnow_mcp::{LocalDb, McpServer};
use opensnow_storage::CatalogConfig;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), BoxError> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone());

    let http = flag("--http");
    let pg = flag("--pg");
    if args.iter().any(|arg| arg == "--pg") && pg.as_ref().is_none_or(|addr| addr.starts_with("--")) {
        return Err("--pg requires a listen address".into());
    }
    if http.is_some() || pg.is_some() {
        let tls = if pg.is_some() {
            match (std::env::var_os("OPENSNOW_PG_TLS_CERT"), std::env::var_os("OPENSNOW_PG_TLS_KEY")) {
                (Some(cert), Some(key)) => Some(Arc::new(load_tls_config(cert.as_ref(), key.as_ref())?)),
                (None, None) => None,
                _ => return Err("set both OPENSNOW_PG_TLS_CERT and OPENSNOW_PG_TLS_KEY or neither".into()),
            }
        } else {
            None
        };
        let uri = std::env::var("OPENSNOW_CONTROL_DATABASE_URL")
            .map_err(|_| -> BoxError { "OPENSNOW_CONTROL_DATABASE_URL is required for --http or --pg".into() })?;
        let warehouse = std::env::var("OPENSNOW_WAREHOUSE").unwrap_or_else(|_| "opensnow-warehouse".into());
        let pool = sqlx::PgPool::connect(&uri).await?;
        let store = ControlStore::new(pool);
        store.ensure_schema().await.map_err(|e| -> BoxError { e.to_string().into() })?;
        let storage = CatalogConfig::postgres("opensnow-http", uri, warehouse).connect().await?;
        let plane = Arc::new(ControlPlane::new(store, Arc::new(storage)));
        let http_server = async {
            if let Some(addr) = http {
                let listener = tokio::net::TcpListener::bind(addr).await?;
                eprintln!("opensnow http transport listening on {}", listener.local_addr()?);
                serve(listener, HttpServer::new(plane.clone())).await?;
            }
            Ok::<_, BoxError>(())
        };
        let pg_server = async {
            if let Some(addr) = pg {
                let listener = tokio::net::TcpListener::bind(addr).await?;
                eprintln!("opensnow postgres transport listening on {}", listener.local_addr()?);
                serve_pg(listener, plane.clone(), tls).await?;
            }
            Ok::<_, BoxError>(())
        };
        tokio::try_join!(http_server, pg_server)?;
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
