use std::str::FromStr;

use ethers::signers::{LocalWallet, Signer};

use crate::{
    config::AppConfig,
    error::{AppError, AppResult},
};

/// Thin wrapper responsible for loading an optional signer from configuration.
#[derive(Debug, Clone)]
pub struct WalletManager {
    signer: Option<LocalWallet>,
}

impl WalletManager {
    pub fn new(signer: Option<LocalWallet>) -> Self {
        Self { signer }
    }

    pub fn from_config(config: &AppConfig) -> AppResult<Self> {
        if let Some(ref key) = config.private_key {
            let trimmed = key.trim_start_matches("0x");
            let wallet = LocalWallet::from_str(trimmed)
                .map_err(|err| AppError::Wallet(format!("failed to parse private key: {err}")))?;
            let wallet = wallet.with_chain_id(config.default_chain_id);
            Ok(Self::new(Some(wallet)))
        } else {
            Ok(Self::new(None))
        }
    }

    pub fn signer(&self) -> Option<LocalWallet> {
        self.signer.clone()
    }
}
