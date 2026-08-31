//! `mcs51-mcp` — MCP stdio server for the 8051 development loop.
//!
//! Deliberately thin: read the environment, build the server, serve stdio.
//! Everything else lives in the library so it can be tested without a transport.

use mcs51_mcp::{config::Config, server::Server};
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // stdout is the MCP channel. One stray line on it corrupts the stream, so
    // every diagnostic goes to stderr, and ANSI is off because the peer is a
    // program, not a terminal.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Fails loudly if FIRMWARE_ROOT is set but unusable. Starting unconfined
    // when confinement was asked for would be the worst possible recovery.
    let config = Config::from_env().inspect_err(|e| {
        tracing::error!("configuration error: {e}");
    })?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        confinement = config.confinement(),
        firmware_root = ?config.firmware_root,
        "mcs51-mcp starting on stdio"
    );

    let service = Server::new(config).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
