use std::{str::FromStr, sync::Arc, time::Duration};

use anyhow::{ensure, Context, Result};
use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{Address, U256},
};

use walletmcp::implementations::{erc20, swap::simulate_swap};
use walletmcp::types::SwapTokensParams;

/// This test talks to a live network. It is ignored by default; run it manually with:
/// `cargo test -- --ignored swap_tokens_params_mainnet_smoke`
/// Required env vars:
///   MAINNET_RPC_URL - HTTPS endpoint for Ethereum mainnet
///   WALLET_PK       - hex-encoded private key with enough token balance & approval
/// Optional overrides:
///   SWAP_FROM_TOKEN, SWAP_TO_TOKEN, SWAP_AMOUNT_IN_WEI, SWAP_SLIPPAGE_BPS, SWAP_POOL_FEE
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn swap_tokens_params_mainnet_smoke() -> Result<()> {
    dotenvy::dotenv().ok();

    let rpc_url = std::env::var("MAINNET_RPC_URL")
        .context("MAINNET_RPC_URL env var must be set to run this test")?;
    let private_key = std::env::var("WALLET_PK")
        .context("WALLET_PK env var must be set to run this test")?;

    let base_provider = Provider::<Http>::try_from(rpc_url)
        .context("failed to connect to MAINNET_RPC_URL")?
        .interval(Duration::from_millis(200));

    let chain_id = base_provider
        .get_chainid()
        .await
        .context("failed to fetch chain id")?
        .as_u64();

    let wallet = private_key
        .parse::<LocalWallet>()
        .context("invalid private key in WALLET_PK")?
        .with_chain_id(chain_id);

    let from_token_str = std::env::var("SWAP_FROM_TOKEN")
        .unwrap_or_else(|_| "0xC02aaa39b223FE8D0A0e5C4F27eAD9083C756Cc2".into());
    let to_token_str = std::env::var("SWAP_TO_TOKEN")
        .unwrap_or_else(|_| "0x6B175474E89094C44Da98b954EedeAC495271d0F".into());
    let amount_in_wei = std::env::var("SWAP_AMOUNT_IN_WEI")
        .unwrap_or_else(|_| "10000000000000000".into()); // 0.01 WETH

    let slippage_bps = std::env::var("SWAP_SLIPPAGE_BPS")
        .ok()
        .map(|value| value.parse::<u32>().context("could not parse SWAP_SLIPPAGE_BPS"))
        .transpose()?
        .unwrap_or(100);

    let fee = std::env::var("SWAP_POOL_FEE")
        .ok()
        .map(|value| value.parse::<u32>().context("could not parse SWAP_POOL_FEE"))
        .transpose()?
        .unwrap_or(3_000);

    // Exercise serde defaults for SwapTokensParams.
    let params_json = serde_json::json!({
        "from_token": from_token_str,
        "to_token": to_token_str,
        "amount_in_wei": amount_in_wei,
    });

    let mut params: SwapTokensParams = serde_json::from_value(params_json)
        .context("failed to deserialize SwapTokensParams")?;
    assert_eq!(
        params.slippage_bps, 100,
        "default slippage_bps should be 100 bps (1%)"
    );
    assert_eq!(params.fee, 3_000, "default fee should be 0.3% pool");

    params.slippage_bps = slippage_bps;
    params.fee = fee;
    params.recipient = Some(format!("{:#x}", wallet.address()));

    let from_token = Address::from_str(&params.from_token)
        .context("invalid SWAP_FROM_TOKEN address")?;
    let to_token =
        Address::from_str(&params.to_token).context("invalid SWAP_TO_TOKEN address")?;

    let amount_in = U256::from_dec_str(&params.amount_in_wei)
        .context("amount_in_wei is not a valid decimal string")?;

    let provider = Arc::new(SignerMiddleware::new(base_provider, wallet.clone()));

    let balance = erc20::fetch_balance_of(provider.clone(), from_token, wallet.address())
        .await
        .context("failed to fetch sender balance")?;

    ensure!(
        balance >= amount_in,
        "holder address {:#x} does not have enough balance of token {} to cover {} wei",
        wallet.address(),
        params.from_token,
        params.amount_in_wei
    );

    let sim_out = simulate_swap(provider, wallet, from_token, to_token, params)
        .await
        .map_err(|err| anyhow::anyhow!("simulate_swap failed: {err}"))?;

    ensure!(
        !sim_out.amount_out_estimate.is_empty(),
        "simulation produced empty amount_out_estimate"
    );

    Ok(())
}
