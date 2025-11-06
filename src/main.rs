mod config;
mod error;
mod implementations;
mod layers;
mod types;
mod wallet;

use std::sync::Arc;

use config::AppConfig;
use error::{AppError, AppResult};
use ethers::providers::{Http, Provider};
use layers::{
    mcp::McpServer,
    service::{ServiceContext, ServiceLayer},
};
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        error!("fatal error: {err}");
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    init_tracing();

    info!("loading configuration");
    let config = AppConfig::load()?;

    info!("connecting to provider");
    let provider = build_provider(&config.eth_rpc_url)?;
    let provider = Arc::new(provider);

    info!("initialising wallet manager");
    let wallet = Arc::new(wallet::WalletManager::from_config(&config)?);

    let registry = implementations::price::TokenRegistry::with_defaults();
    let registry = Arc::new(RwLock::new(registry));

    let service_ctx = Arc::new(ServiceContext::new(provider.clone(), registry, wallet));
    let service = ServiceLayer::new(service_ctx);

    info!("starting MCP stdio server");
    let server = McpServer::new(service);
    server.run_stdio().await
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_line_number(true)
        .init();
}

fn build_provider(url: &str) -> AppResult<Provider<Http>> {
    Provider::<Http>::try_from(url)
        .map_err(|err| AppError::Config(format!("failed to create provider: {err}")))
}
