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

    if config.path.is_none() {
        let discovered = stores.available_paths().await;
        if discovered.is_empty() {
            info!("No xcstrings files discovered at startup");
        } else {
            info!(
                count = discovered.len(),
                "Discovered xcstrings files at startup"
            );
        }
    }

    let web_handle = {
        let addr = config.web_addr;
        let manager = stores.clone();
        tokio::spawn(async move {
            if let Err(err) = web::serve(addr, manager).await {
                error!(?err, "Web server stopped");
            }
        })
    };

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
    path: Option<PathBuf>,
    web_addr: SocketAddr,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let mut args = env::args_os().skip(1);

        let path = if let Ok(path) = env_var("STRINGS_PATH", "XCSTRINGS_PATH") {
            Some(PathBuf::from(path))
        } else {
            let mut candidate = args.next();
            if matches!(candidate.as_ref(), Some(arg) if arg == OsStr::new("--")) {
                candidate = args.next();
            }
            candidate.map(PathBuf::from)
        };

        let host =
            env_var("WEB_HOST", "XCSTRINGS_WEB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = if let Ok(port) = env_var("WEB_PORT", "XCSTRINGS_WEB_PORT") {
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

fn env_var(primary: &str, legacy: &str) -> Result<String, env::VarError> {
    env::var(primary).or_else(|primary_err| match primary_err {
        env::VarError::NotPresent => env::var(legacy),
        err => Err(err),
    })
}
