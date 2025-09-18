use std::{env, ffi::OsStr, net::SocketAddr, path::PathBuf, sync::Arc};

use rmcp::service::ServiceExt;
use tokio::signal;
use tracing::{error, info, warn};

use anyhow::Context;
use xcstrings_mcp::{mcp_server::XcStringsMcpServer, store::XcStringsStoreManager, web};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .init();

    let config = Config::from_env()?;
    match config.path.as_ref() {
        Some(path) => {
            info!(path = %path.display(), web_addr = %config.web_addr, "Starting xcstrings MCP server");
        }
        None => {
            info!(web_addr = %config.web_addr, "Starting xcstrings MCP server in dynamic-path mode");
        }
    }

    let stores = Arc::new(
        XcStringsStoreManager::new(config.path.clone())
            .await
            .map_err(|err| anyhow::anyhow!(err))?,
    );

    let default_store = if config.path.is_some() {
        Some(
            stores
                .default_store()
                .await
                .map_err(|err| anyhow::anyhow!(err))?,
        )
    } else {
        None
    };

    if default_store.is_none() {
        info!("Web UI disabled until a default xcstrings path is configured");
    }

    let web_handle = default_store.clone().map(|store| {
        let addr = config.web_addr;
        tokio::spawn(async move {
            if let Err(err) = web::serve(addr, store).await {
                error!(?err, "Web server stopped");
            }
        })
    });

    let mcp_handle = {
        let server = XcStringsMcpServer::new(stores.clone());
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

    if let Some(web_handle) = web_handle {
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
    } else {
        tokio::select! {
            _ = signal::ctrl_c() => {
                warn!("Received Ctrl+C — shutting down");
            }
            _ = mcp_handle => {
                warn!("MCP task exited");
            }
        }
    }

    Ok(())
}

struct Config {
    path: Option<PathBuf>,
    web_addr: SocketAddr,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let mut args = env::args_os().skip(1);

        let path = if let Ok(path) = env::var("XCSTRINGS_PATH") {
            Some(PathBuf::from(path))
        } else {
            let mut candidate = args.next();
            if matches!(candidate.as_ref(), Some(arg) if arg == OsStr::new("--")) {
                candidate = args.next();
            }
            candidate.map(PathBuf::from)
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
