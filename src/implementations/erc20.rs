use std::sync::Arc;

use ethers::{
    providers::Middleware,
    types::{Address, U256},
};
use ethers_contract::abigen;

use crate::error::{AppError, AppResult};

abigen!(
    Erc20Token,
    r#"[
        function balanceOf(address) view returns (uint256)
        function decimals() view returns (uint8)
        function symbol() view returns (string)
    ]"#
);

#[derive(Debug, Clone)]
pub struct Erc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

pub async fn fetch_metadata<M>(provider: Arc<M>, token: Address) -> AppResult<Erc20Metadata>
where
    M: Middleware + 'static,
{
    let contract = Erc20Token::new(token, provider);
    let decimals = contract
        .decimals()
        .call()
        .await
        .map_err(|err| AppError::Rpc(format!("failed to fetch ERC-20 decimals: {err}")))?;
    let symbol = contract
        .symbol()
        .call()
        .await
        .unwrap_or_else(|_| "ERC20".to_string());

    Ok(Erc20Metadata { symbol, decimals })
}

pub async fn fetch_balance_of<M>(
    provider: Arc<M>,
    token: Address,
    owner: Address,
) -> AppResult<U256>
where
    M: Middleware + 'static,
{
    let contract = Erc20Token::new(token, provider);
    contract
        .balance_of(owner)
        .call()
        .await
        .map_err(|err| AppError::Rpc(format!("failed to fetch token balance: {err}")))
}
