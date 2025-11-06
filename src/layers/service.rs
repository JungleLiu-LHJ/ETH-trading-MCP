use std::sync::Arc;

use crate::{
    error::{AppError, AppResult},
    implementations::{
        balance,
        price::{self, TokenRegistry},
        swap,
    },
    types::{
        BalanceOut, GetBalanceParams, GetTokenPriceParams, PriceOut, SwapSimOut, SwapTokensParams,
    },
    wallet::WalletManager,
};
use ethers::{
    providers::{Http, Provider},
    types::Address,
};
use tokio::sync::RwLock;
use tracing::{info, instrument};

/// Shared context that higher layers pass around. Keeps provider, registry, and wallet handles.
#[derive(Clone)]
pub struct ServiceContext {
    pub provider: Arc<Provider<Http>>,
    pub registry: Arc<RwLock<TokenRegistry>>,
    pub wallet: Arc<WalletManager>,
}

impl ServiceContext {
    pub fn new(
        provider: Arc<Provider<Http>>,
        registry: Arc<RwLock<TokenRegistry>>,
        wallet: Arc<WalletManager>,
    ) -> Self {
        Self {
            provider,
            registry,
            wallet,
        }
    }
}

/// Middle layer that exposes business-level operations while delegating heavy work to implementation modules.
#[derive(Clone)]
pub struct ServiceLayer {
    ctx: Arc<ServiceContext>,
}

impl ServiceLayer {
    pub fn new(ctx: Arc<ServiceContext>) -> Self {
        Self { ctx }
    }

    /// Balance lookup entry point. Handles optional ERC-20 parameter resolution.
    #[instrument(skip(self), fields(address = %params.address, token = %params.token.as_deref().unwrap_or("ETH")))]
    pub async fn get_balance(&self, params: GetBalanceParams) -> AppResult<BalanceOut> {
        let registry_snapshot = self.snapshot_registry().await;
        let address = parse_address_or_symbol(&params.address, &registry_snapshot)?;
        let token = match params.token {
            Some(token_str) => Some(parse_address_or_symbol(&token_str, &registry_snapshot)?),
            None => None,
        };

        let result = balance::resolve_balance(self.ctx.provider.clone(), address, token).await?;
        info!("balance lookup succeeded");
        Ok(result)
    }

    /// Price lookup with Chainlink-first policy and Uniswap fallback.
    #[instrument(skip(self), fields(base = %params.base, quote = %params.quote))]
    pub async fn get_token_price(&self, params: GetTokenPriceParams) -> AppResult<PriceOut> {
        let base_address = self.resolve_input(&params.base).await?;

        // Ensure registry knows about base token for metadata-driven pricing.
        self.ensure_registry_token(base_address).await?;
        let registry_snapshot = self.snapshot_registry().await;

        let price = price::resolve_token_price(
            self.ctx.provider.clone(),
            &registry_snapshot,
            base_address,
            params.quote,
        )
        .await?;

        info!("price lookup succeeded via {}", price.source);
        Ok(price)
    }

    /// Build and simulate Uniswap V3 calldata without broadcasting.
    #[instrument(skip(self), fields(from = %params.from_token, to = %params.to_token))]
    pub async fn swap_tokens(&self, params: SwapTokensParams) -> AppResult<SwapSimOut> {
        let from_token = self.resolve_input(&params.from_token).await?;
        let to_token = self.resolve_input(&params.to_token).await?;

        // Swap simulations require decimals, so ensure both tokens exist in the registry cache.
        self.ensure_registry_token(from_token).await?;
        self.ensure_registry_token(to_token).await?;

        let signer = self.ctx.wallet.signer().ok_or_else(|| {
            AppError::Wallet("swap simulation requires PRIVATE_KEY/signing config".into())
        })?;

        let result = swap::simulate_swap(
            self.ctx.provider.clone(),
            signer,
            from_token,
            to_token,
            params,
        )
        .await?;

        info!("swap simulation succeeded");
        Ok(result)
    }

    /// Resolve a symbol or raw address string into an Ethereum address.
    async fn resolve_input(&self, input: &str) -> AppResult<Address> {
        if let Ok(addr) = input.parse::<Address>() {
            return Ok(addr);
        }

        let registry_snapshot = self.snapshot_registry().await;
        registry_snapshot.resolve_symbol(input).ok_or_else(|| {
            AppError::InvalidInput(format!("unknown token symbol or address: {input}"))
        })
    }

    async fn ensure_registry_token(&self, address: Address) -> AppResult<()> {
        let mut registry = self.ctx.registry.write().await;
        registry
            .ensure_token(self.ctx.provider.clone(), address)
            .await
    }

    /// Convenience helper to avoid holding locks while we await downstream futures.
    async fn snapshot_registry(&self) -> TokenRegistry {
        self.ctx.registry.read().await.clone()
    }
}

fn parse_address_or_symbol(input: &str, registry: &TokenRegistry) -> AppResult<Address> {
    if let Ok(addr) = input.parse::<Address>() {
        return Ok(addr);
    }

    registry
        .resolve_symbol(input)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown token symbol or address: {input}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::implementations::price::{TokenInfo, TokenRegistry};
    use ethers::types::Address;
    use std::str::FromStr;

    fn dummy_registry() -> TokenRegistry {
        let mut registry = TokenRegistry::new();
        registry.add_token(TokenInfo::new(
            "WETH",
            Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap(),
            18,
        ));
        registry
    }

    #[test]
    fn parse_known_symbol() {
        let registry = dummy_registry();
        let address = parse_address_or_symbol("weth", &registry).unwrap();
        assert_eq!(
            address,
            Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap()
        );
    }

    #[test]
    fn parse_unknown_symbol() {
        let registry = dummy_registry();
        let err = parse_address_or_symbol("FOO", &registry).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }
}
