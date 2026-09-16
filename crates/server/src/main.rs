//! Native binary (design brief §9): holds the current screen spec, serves
//! it as JSON to the device (`GET /screen`) and to the browser preview, and
//! accepts updates (`PUT /screen`). Runs on the developer's machine — the
//! local server the device's firmware polls over WiFi (design brief §10).
//!
//! ```text
//! cargo run -p server -- --bind 0.0.0.0:8080
//! curl -X PUT -H 'content-type: application/json' \
//!      --data-binary @crates/screen-spec/samples/kitchen.json \
//!      http://127.0.0.1:8080/screen
//! open http://127.0.0.1:8080/
//! ```

mod routes;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::routes::AssetSource;
use crate::state::AppState;

/// LAN content server + browser preview for the reTerminal E1002.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Address to listen on. Use 0.0.0.0:<port> to be reachable by the device.
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: SocketAddr,

    /// Where the current screen spec is persisted (survives restarts).
    /// Pass an empty string to keep it in memory only.
    #[arg(long, default_value = "screen.json")]
    state_file: String,

    /// Serve the preview page from this directory instead of the copy
    /// embedded in the binary (handy while editing preview.js).
    #[arg(long)]
    assets_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();
    let args = Args::parse();

    let state_file = (!args.state_file.is_empty()).then(|| PathBuf::from(&args.state_file));
    let state = Arc::new(AppState::load(state_file)?);
    let assets = match args.assets_dir {
        Some(dir) => AssetSource::Dir(dir),
        None => AssetSource::Embedded,
    };
    let app = routes::router(state, assets);

    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(
        "listening on http://{}/ (device fetches /screen)",
        args.bind
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
        })
        .await?;
    Ok(())
}
