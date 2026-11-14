//! `stormd`: headless storm engine, controlled by the storm desktop app over HTTP.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;
use storm_engine::{EngineConfig, LocalBackend};

#[derive(Parser, Debug)]
#[command(name = "stormd", version, about = "Headless storm torrent engine")]
struct Args {
    /// Address to listen on.
    #[arg(long, env = "STORMD_LISTEN", default_value_t = SocketAddr::from(([0, 0, 0, 0], storm_proto::DEFAULT_PORT)))]
    listen: SocketAddr,

    /// Where the database, session state and token live.
    #[arg(long, env = "STORMD_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Default download folder (only used until changed in settings).
    #[arg(long, env = "STORMD_DOWNLOAD_DIR")]
    download_dir: Option<PathBuf>,

    /// API token. If unset, one is generated once and stored in `<data-dir>/token`.
    #[arg(long, env = "STORMD_TOKEN", hide_env_values = true)]
    token: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,librqbit=warn,librqbit_dht=warn".into()),
        )
        .init();
    let args = Args::parse();

    let defaults = EngineConfig::platform_default()?;
    let data_dir = args.data_dir.unwrap_or_else(|| defaults.data_dir.join("stormd"));
    let config =
        EngineConfig { default_download_dir: args.download_dir.unwrap_or(defaults.default_download_dir), data_dir };
    std::fs::create_dir_all(&config.data_dir)?;

    let (token, token_source) = match args.token {
        Some(t) if !t.trim().is_empty() => (t.trim().to_string(), None),
        _ => load_or_create_token(&config.data_dir.join("token"))?,
    };

    let backend = LocalBackend::start(config.clone()).await?;
    let app = storm_server::router(backend.clone(), token.clone());
    let listener =
        tokio::net::TcpListener::bind(args.listen).await.with_context(|| format!("binding {}", args.listen))?;

    tracing::info!("data dir: {}", config.data_dir.display());
    tracing::info!("listening on http://{}", listener.local_addr()?);
    match token_source {
        Some(path) => tracing::info!("API token (stored in {}): {token}", path.display()),
        None => tracing::info!("API token: provided via --token / STORMD_TOKEN"),
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
        })
        .await?;
    backend.shutdown().await;
    Ok(())
}

fn load_or_create_token(path: &Path) -> anyhow::Result<(String, Option<PathBuf>)> {
    if let Ok(t) = std::fs::read_to_string(path)
        && !t.trim().is_empty()
    {
        return Ok((t.trim().to_string(), Some(path.to_owned())));
    }
    let token = storm_server::generate_token();
    std::fs::write(path, &token).with_context(|| format!("writing {}", path.display()))?;
    Ok((token, Some(path.to_owned())))
}
