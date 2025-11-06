use std::{sync::Arc, time::Duration};

use ethers::providers::{Http, Provider};
use rust_decimal::Decimal;

use walletmcp::{
    config::AppConfig,
    implementations::price::{resolve_token_price, TokenRegistry},
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
async fn price_chainlink_via_usd_dai_eth_real() {
    let provider = real_provider();
    let registry = TokenRegistry::with_defaults();

    let dai = registry
        .info_by_symbol("DAI")
        .expect("DAI must exist in defaults");

    let out = resolve_token_price(provider, &registry, dai.address, QuoteCurrency::ETH)
        .await
        .expect("chainlink DAI/ETH via USD should succeed");

    assert_eq!(out.base, "DAI");
    assert_eq!(out.quote, "ETH");
    assert_eq!(out.source, "chainlink (via USD)");
    let price = Decimal::from_str_exact(&out.price).expect("valid decimal");
    assert!(price > Decimal::ZERO);
}

