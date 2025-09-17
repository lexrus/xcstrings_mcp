use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context;
use rmcp::service::ServiceExt;
use tokio::signal;
use tracing::{error, info, warn};

use xcstrings_mcp::{mcp_server::XcStringsMcpServer, store::XcStringsStore, web};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .init();

    let config = Config::from_env()?;
    info!(path = %config.path.display(), web_addr = %config.web_addr, "Starting xcstrings MCP server");

    let store = Arc::new(
        XcStringsStore::load_or_create(&config.path)
            .await
            .with_context(|| {
                format!("unable to load xcstrings file at {}", config.path.display())
            })?,
    );

    let web_handle = {
        let store = store.clone();
        let addr = config.web_addr;
        tokio::spawn(async move {
            if let Err(err) = web::serve(addr, store).await {
                error!(?err, "Web server stopped");
            }
        })
    };

    let mcp_handle = {
        let server = XcStringsMcpServer::new(store.clone());
        tokio::spawn(async move {
            let transport = (tokio::io::stdin(), tokio::io::stdout());
            match server.router().serve(transport).await {
                Ok(running) => {
                    if let Err(err) = running.waiting().await {
                        error!(?err, "MCP service finished with error");
                    }
                }
                Err(err) => {
                    error!(?err, "Failed to start MCP service");
                }
            }
        })
    };

    tokio::select! {
        _ = signal::ctrl_c() => {
            warn!("Received Ctrl+C — shutting down");
        }
        _ = web_handle => {
            warn!("Web server task exited");
        }
        _ = mcp_handle => {
            warn!("MCP task exited");
        }
    }

    Ok(())
}

struct Config {
    path: PathBuf,
    web_addr: SocketAddr,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let mut args = env::args_os().skip(1);

        let path = if let Ok(path) = env::var("XCSTRINGS_PATH") {
            PathBuf::from(path)
        } else if let Some(arg) = args.next() {
            PathBuf::from(arg)
        } else {
            PathBuf::from("Localizable.xcstrings")
        };

        let host = env::var("XCSTRINGS_WEB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = if let Ok(port) = env::var("XCSTRINGS_WEB_PORT") {
            port
        } else {
            args.next()
                .and_then(|arg| arg.into_string().ok())
                .unwrap_or_else(|| "8787".to_string())
        };

        let port: u16 = port.parse().context("invalid web port")?;
        let web_addr: SocketAddr = format!("{}:{}", host, port)
            .parse()
            .context("invalid web address")?;

        Ok(Self { path, web_addr })
    }
}
