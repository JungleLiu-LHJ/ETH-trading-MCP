use crate::error::{AppError, AppResult};
use dotenvy::dotenv;
use serde::Deserialize;
use std::{env, fs, path::Path};

const DEFAULT_CONFIG_PATH: &str = "Config.toml";
const DEFAULT_CHAIN_ID: u64 = 1;

/// Strongly-typed configuration derived from a `Config.toml` or environment variables.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub eth_rpc_url: String,
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default = "default_chain_id")]
    pub default_chain_id: u64,
}

fn default_chain_id() -> u64 {
    DEFAULT_CHAIN_ID
}

impl AppConfig {
    /// Load configuration, preferring a user-provided config file and falling back to env vars.
    pub fn load() -> AppResult<Self> {
        dotenv().ok();

        let configured_path =
            env::var("MCP_CONFIG_PATH").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        let config_path = Path::new(&configured_path);

        if config_path.exists() {
            let raw = fs::read_to_string(config_path)
                .map_err(|err| AppError::Config(format!("failed to read config file: {err}")))?;
            let mut cfg: AppConfig = toml::from_str(&raw)
                .map_err(|err| AppError::Config(format!("failed to parse config file: {err}")))?;
            cfg.apply_chain_id_default();
            return Ok(cfg);
        }

        Self::from_env()
    }

    /// Helper used when no config file is present.
    fn from_env() -> AppResult<Self> {
        let eth_rpc_url = env::var("ETH_RPC_URL")
            .map_err(|_| AppError::Config("ETH_RPC_URL missing (config file not found)".into()))?;

        let private_key = env::var("PRIVATE_KEY").ok();
        let default_chain_id = env::var("DEFAULT_CHAIN_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CHAIN_ID);

        Ok(Self {
            eth_rpc_url,
            private_key,
            default_chain_id,
        })
    }

    /// Ensure we never surface a zero chain id from user input.
    fn apply_chain_id_default(&mut self) {
        if self.default_chain_id == 0 {
            self.default_chain_id = DEFAULT_CHAIN_ID;
        }
    }
}
