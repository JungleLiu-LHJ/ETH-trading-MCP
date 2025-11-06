use std::{str::FromStr, sync::Arc, time::Duration};

use ethers::providers::{Http, Provider};
use ethers::types::Address;
use rust_decimal::Decimal;

use walletmcp::{
    config::AppConfig,
    implementations::price::{resolve_token_price, TokenInfo, TokenRegistry},
    types::QuoteCurrency,
};

fn real_provider() -> Arc<Provider<Http>> {
    let cfg = AppConfig::load().expect("ETH_RPC_URL must be configured for real-network tests");
    let provider = Provider::<Http>::try_from(cfg.eth_rpc_url)
        .expect("failed to build provider from ETH_RPC_URL")
        .interval(Duration::from_millis(200));
    Arc::new(provider)
}

#[tokio::test]
async fn price_uniswap_fallback_link_usd_real() {
    let provider = real_provider();
    let mut registry = TokenRegistry::with_defaults();

    // LINK mainnet token (18 decimals), intentionally without chainlink feed to trigger Uniswap.
    let link = Address::from_str("0x514910771AF9Ca656af840dff83E8264EcF986CA").unwrap();
    registry.add_token(TokenInfo::new("LINK", link, 18));

    let out = resolve_token_price(provider, &registry, link, QuoteCurrency::USD)
        .await
        .expect("Uniswap fallback LINK/USD should succeed");

    assert_eq!(out.base, "LINK");
    assert_eq!(out.quote, "USD");
    assert!(out.source.starts_with("uniswap_v3"));
    let price = Decimal::from_str_exact(&out.price).expect("valid decimal");
    assert!(price > Decimal::ZERO);
}

