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
async fn price_chainlink_direct_weth_usd_real() {
    let provider = real_provider();
    let registry = TokenRegistry::with_defaults();

    let weth = registry
        .info_by_symbol("WETH")
        .expect("WETH must exist in defaults");

    let out = resolve_token_price(provider, &registry, weth.address, QuoteCurrency::USD)
        .await
        .expect("chainlink WETH/USD price should succeed");

    assert_eq!(out.base, "WETH");
    assert_eq!(out.quote, "USD");
    assert!(out.source.starts_with("chainlink"));
    let price = Decimal::from_str_exact(&out.price).expect("valid decimal");
    assert!(price > Decimal::ZERO);
}
