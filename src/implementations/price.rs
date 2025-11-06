use std::{collections::HashMap, str::FromStr, sync::Arc};

use ethers::{
    providers::Middleware,
    types::{Address, U256},
};
use ethers_contract::abigen;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use crate::{
    error::{AppError, AppResult},
    implementations::{
        balance, erc20,
        uniswap::{UniswapQuoterV2, uniswap_quoter_v2::QuoteExactInputSingleParams},
    },
    types::{PriceOut, QuoteCurrency},
};

mod defaults;

// Addresses for mainnet reference contracts.
pub static UNISWAP_QUOTER_V2: Lazy<Address> =
    Lazy::new(|| Address::from_str("0x61fFE014bA17989E743c5F6cB21bF9697530B21e").unwrap());
pub static UNISWAP_SWAP_ROUTER: Lazy<Address> =
    Lazy::new(|| Address::from_str("0xE592427A0AEce92De3Edee1F18E0157C05861564").unwrap());

abigen!(
    ChainlinkAggregator,
    r#"[
        function latestRoundData() view returns (uint80, int256, uint256, uint256, uint80)
        function decimals() view returns (uint8)
    ]"#
);

/// Metadata describing a supported token, including common pricing hooks.
#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub symbol: String,
    pub address: Address,
    pub decimals: u8,
    pub chainlink_feeds: HashMap<QuoteCurrency, Address>,
    pub default_fee: u32,
}

impl TokenInfo {
    pub fn new(symbol: impl Into<String>, address: Address, decimals: u8) -> Self {
        let symbol_upper = symbol.into().to_uppercase();
        Self {
            symbol: symbol_upper,
            address,
            decimals,
            chainlink_feeds: HashMap::new(),
            default_fee: 3_000,
        }
    }

    pub fn with_feed(mut self, quote: QuoteCurrency, feed_address: Address) -> Self {
        self.chainlink_feeds.insert(quote, feed_address);
        self
    }

    pub fn with_fee(mut self, fee: u32) -> Self {
        self.default_fee = fee;
        self
    }
}

/// Registry of known tokens to ease symbol lookup and pricing fallbacks.
#[derive(Debug, Clone)]
pub struct TokenRegistry {
    by_symbol: HashMap<String, TokenInfo>,
    by_address: HashMap<Address, TokenInfo>,
}

impl TokenRegistry {
    pub fn new() -> Self {
        Self {
            by_symbol: HashMap::new(),
            by_address: HashMap::new(),
        }
    }

    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        defaults::populate_defaults(&mut registry);
        registry
    }

    pub fn add_token(&mut self, info: TokenInfo) {
        self.by_symbol.insert(info.symbol.clone(), info.clone());
        self.by_address.insert(info.address, info);
    }

    pub async fn ensure_token<M>(&mut self, provider: Arc<M>, address: Address) -> AppResult<()>
    where
        M: Middleware + 'static,
    {
        if self.by_address.contains_key(&address) {
            return Ok(());
        }

        let metadata = erc20::fetch_metadata(provider, address).await?;
        let symbol = if metadata.symbol.is_empty() {
            format!("TOKEN_{address:?}")
        } else {
            metadata.symbol
        };

        let info = TokenInfo::new(symbol, address, metadata.decimals);
        self.add_token(info);
        Ok(())
    }

    pub fn resolve_symbol(&self, symbol: &str) -> Option<Address> {
        self.by_symbol
            .get(&symbol.to_uppercase())
            .map(|info| info.address)
    }

    pub fn info_by_address(&self, address: Address) -> Option<&TokenInfo> {
        self.by_address.get(&address)
    }

    pub fn info_by_symbol(&self, symbol: &str) -> Option<&TokenInfo> {
        self.by_symbol.get(&symbol.to_uppercase())
    }

    pub fn quote_token(&self, quote: QuoteCurrency) -> Option<&TokenInfo> {
        match quote {
            QuoteCurrency::USD => self.info_by_symbol("USDC"),
            QuoteCurrency::ETH => self.info_by_symbol("WETH"),
        }
    }
}

/// Resolve token price with Chainlink-first policy and Uniswap fallback.
pub async fn resolve_token_price<M>(
    provider: Arc<M>,
    registry: &TokenRegistry,
    base: Address,
    quote: QuoteCurrency,
) -> AppResult<PriceOut>
where
    M: Middleware + 'static,
{
    let base_info = registry
        .info_by_address(base)
        .ok_or_else(|| AppError::InvalidInput(format!("unsupported token: {base:?}")))?;

    // Attempt direct Chainlink feed (base/quote).
    if let Some(feed_addr) = base_info.chainlink_feeds.get(&quote) {
        let price = fetch_chainlink_price(provider.clone(), *feed_addr).await?;
        return Ok(PriceOut {
            base: base_info.symbol.clone(),
            quote: quote.to_string(),
            price: price.to_string(),
            source: "chainlink".to_string(),
            decimals: price.scale() as u32,
        });
    }

    // Attempt Chainlink via USD pivot if quote is ETH.
    if quote == QuoteCurrency::ETH {
        if let Some(base_usd_feed) = base_info.chainlink_feeds.get(&QuoteCurrency::USD) {
            if let Some(eth_info) = registry.info_by_symbol("WETH") {
                if let Some(eth_usd_feed) = eth_info.chainlink_feeds.get(&QuoteCurrency::USD) {
                    let base_usd = fetch_chainlink_price(provider.clone(), *base_usd_feed).await?;
                    let eth_usd = fetch_chainlink_price(provider.clone(), *eth_usd_feed).await?;
                    if eth_usd.is_zero() {
                        return Err(AppError::Price(
                            "received zero ETH/USD price from Chainlink".into(),
                        ));
                    }
                    let price = base_usd / eth_usd;
                    return Ok(PriceOut {
                        base: base_info.symbol.clone(),
                        quote: quote.to_string(),
                        price: price.to_string(),
                        source: "chainlink (via USD)".to_string(),
                        decimals: price.scale() as u32,
                    });
                }
            }
        }
    }

    // Attempt Chainlink via ETH pivot if quote is USD.
    if quote == QuoteCurrency::USD {
        if let Some(base_eth_feed) = base_info.chainlink_feeds.get(&QuoteCurrency::ETH) {
            if let Some(eth_info) = registry.info_by_symbol("WETH") {
                if let Some(eth_usd_feed) = eth_info.chainlink_feeds.get(&QuoteCurrency::USD) {
                    let base_eth = fetch_chainlink_price(provider.clone(), *base_eth_feed).await?;
                    let eth_usd = fetch_chainlink_price(provider.clone(), *eth_usd_feed).await?;
                    let price = base_eth * eth_usd;
                    return Ok(PriceOut {
                        base: base_info.symbol.clone(),
                        quote: quote.to_string(),
                        price: price.to_string(),
                        source: "chainlink (via ETH)".to_string(),
                        decimals: price.scale() as u32,
                    });
                }
            }
        }
    }

    // Fall back to Uniswap price quotes.
    let quote_token = registry
        .quote_token(quote)
        .ok_or_else(|| AppError::Price("missing quote token configuration".into()))?;

    let decimal_price = fetch_uniswap_price(provider.clone(), base_info, quote_token).await?;
    let source = format!("uniswap_v3 (fee {})", base_info.default_fee);

    Ok(PriceOut {
        base: base_info.symbol.clone(),
        quote: quote.to_string(),
        price: decimal_price.to_string(),
        source,
        decimals: decimal_price.scale() as u32,
    })
}

async fn fetch_chainlink_price<M>(provider: Arc<M>, feed_address: Address) -> AppResult<Decimal>
where
    M: Middleware + 'static,
{
    let contract = ChainlinkAggregator::new(feed_address, provider);
    let decimals = contract
        .decimals()
        .call()
        .await
        .map_err(|err| AppError::Price(format!("failed to read feed decimals: {err}")))?;

    let round = contract
        .latest_round_data()
        .call()
        .await
        .map_err(|err| AppError::Price(format!("failed to read latest round: {err}")))?;

    let answer = round.1;
    let price_i128 = i128::from_str(&answer.to_string())
        .map_err(|err| AppError::Price(format!("invalid Chainlink answer: {err}")))?;

    if price_i128 <= 0 {
        return Err(AppError::Price(
            "Chainlink returned non-positive price".into(),
        ));
    }

    Ok(Decimal::from_i128_with_scale(price_i128, decimals as u32))
}

async fn fetch_uniswap_price<M>(
    provider: Arc<M>,
    base: &TokenInfo,
    quote: &TokenInfo,
) -> AppResult<Decimal>
where
    M: Middleware + 'static,
{
    let quoter = UniswapQuoterV2::new(*UNISWAP_QUOTER_V2, provider.clone());

    let amount_in = ten_pow(base.decimals as u32);
    let params = QuoteExactInputSingleParams {
        token_in: base.address,
        token_out: quote.address,
        amount_in,
        fee: base.default_fee,
        sqrt_price_limit_x96: U256::zero(),
    };

    let (amount_out, _, _, _) = quoter
        .quote_exact_input_single(params)
        .call()
        .await
        .map_err(|err| AppError::Price(format!("uniswap quote failed: {err}")))?;

    if amount_out.is_zero() {
        return Err(AppError::Price("uniswap returned zero amount out".into()));
    }

    let formatted = balance::format_with_decimals(&amount_out, quote.decimals as u32);
    Decimal::from_str_exact(&formatted)
        .map_err(|err| AppError::Price(format!("failed to parse uniswap result: {err}")))
}

fn ten_pow(decimals: u32) -> U256 {
    let ten = U256::from(10u8);
    ten.pow(U256::from(decimals))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use ethers::providers::{Http, Provider};
    use rust_decimal::Decimal;
    use std::{str::FromStr, sync::Arc, time::Duration};

    fn real_provider() -> Arc<Provider<Http>> {
        let cfg =
            AppConfig::load().expect("ETH_RPC_URL (or config) must be set for real-network tests");
        let provider = Provider::<Http>::try_from(cfg.eth_rpc_url)
            .expect("failed to construct provider")
            .interval(Duration::from_millis(200));
        Arc::new(provider)
    }

    #[test]
    fn ten_pow_works() {
        let result = ten_pow(18);
        assert_eq!(result, U256::from_dec_str("1000000000000000000").unwrap());
    }

    #[tokio::test]
    async fn resolve_token_price_unknown_token() {
        let provider = real_provider();
        let registry = TokenRegistry::with_defaults();

        let base = Address::from_str("0x00000000000000000000000000000000000000de").unwrap();
        let res = resolve_token_price(provider, &registry, base, QuoteCurrency::USD).await;

        match res {
            Err(AppError::InvalidInput(msg)) => {
                assert!(msg.contains("unsupported token"))
            }
            other => panic!("expected InvalidInput error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn resolve_token_price_missing_quote_token_config() {
        let provider = real_provider();
        let mut registry = TokenRegistry::new();

        let base = Address::from_str("0x0000000000000000000000000000000000000002").unwrap();
        registry.add_token(TokenInfo::new("FOO", base, 18));

        let res = resolve_token_price(provider, &registry, base, QuoteCurrency::USD).await;

        match res {
            Err(AppError::Price(msg)) => {
                assert!(msg.contains("missing quote token configuration"))
            }
            other => panic!("expected Price error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn resolve_token_price_chainlink_direct_success() {
        let provider = real_provider();
        let registry = TokenRegistry::with_defaults();

        let weth = registry
            .info_by_symbol("USDC")
            .expect("default registry should include WETH");

        let out = resolve_token_price(provider, &registry, weth.address, QuoteCurrency::USD)
            .await
            .expect("chainlink price should succeed");

        print!("response {:?}", out);

        assert_eq!(out.base, "USDC");
        assert_eq!(out.quote, "USD");
        assert_eq!(out.source, "chainlink");
        let price = Decimal::from_str_exact(&out.price).expect("valid decimal");
        assert!(price > Decimal::ZERO);
        assert!(out.decimals > 0);
    }

    #[tokio::test]
    async fn resolve_token_price_uniswap_fallback_success() {
        let provider = real_provider();
        let mut registry = TokenRegistry::with_defaults();

        let link = Address::from_str("0x95aD61b0a150d79219dCF64E1E6Cc01f0B64C4cE").unwrap();
        registry.add_token(TokenInfo::new("SHIB", link, 18).with_fee(3_000));

        let out = resolve_token_price(provider, &registry, link, QuoteCurrency::USD)
            .await
            .expect("uniswap fallback should succeed");

        print!("response {:?}", out);

        assert_eq!(out.base, "SHIB");
        assert_eq!(out.quote, "USD");
        assert_eq!(out.source, "uniswap_v3 (fee 3000)");
        let price = Decimal::from_str_exact(&out.price).expect("valid decimal");
        assert!(price > Decimal::ZERO);
    }

}
