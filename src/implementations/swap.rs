use std::{
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use ethers::{
    providers::Middleware,
    types::{Address, TransactionRequest, U256, transaction::eip2718::TypedTransaction},
};

use crate::{
    error::{AppError, AppResult},
    implementations::{
        balance, erc20,
        price::{UNISWAP_QUOTER_V2, UNISWAP_SWAP_ROUTER},
        uniswap::{
            UniswapQuoterV2, UniswapRouter, uniswap_quoter_v2::QuoteExactInputSingleParams,
            uniswap_router::ExactInputSingleParams,
        },
    },
    types::SwapTokensParams,
};
use ethers::signers::Signer;

/// Simulate a Uniswap V3 single-hop swap and return calldata plus gas/amount estimates.
pub async fn simulate_swap<M>(
    provider: Arc<M>,
    signer: ethers::signers::LocalWallet,
    from_token: Address,
    to_token: Address,
    params: SwapTokensParams,
) -> AppResult<crate::types::SwapSimOut>
where
    M: Middleware + 'static,
{
    let SwapTokensParams {
        amount_in_wei,
        slippage_bps,
        fee,
        recipient,
        sqrt_price_limit,
        ..
    } = params;

    if slippage_bps > 10_000 {
        return Err(AppError::Swap(
            "slippage cannot exceed 100% (10_000 bps)".into(),
        ));
    }

    let amount_in = parse_amount(&amount_in_wei)?;
    if amount_in.is_zero() {
        return Err(AppError::Swap(
            "amount_in_wei must be greater than zero".into(),
        ));
    }

    // Load token metadata to format human-readable outputs.
    let to_meta = erc20::fetch_metadata(provider.clone(), to_token).await?;

    // Convert optional sqrt price limit into the format expected by Uniswap contracts.
    let sqrt_price_limit_value = sqrt_price_limit
        .as_deref()
        .map(parse_amount)
        .transpose()?
        .unwrap_or_else(U256::zero);

    let quoter = UniswapQuoterV2::new(*UNISWAP_QUOTER_V2, provider.clone());
    let quote_params = QuoteExactInputSingleParams {
        token_in: from_token,
        token_out: to_token,
        amount_in,
        fee,
        sqrt_price_limit_x96: sqrt_price_limit_value,
    };

    let (amount_out, _, _, _) = quoter
        .quote_exact_input_single(quote_params)
        .call()
        .await
        .map_err(|err| AppError::Swap(format!("uniswap quoter call failed: {err}")))?;

    if amount_out.is_zero() {
        return Err(AppError::Swap("quote returned zero output amount".into()));
    }

    let amount_out_min = apply_slippage(amount_out, slippage_bps)?;

    let router = UniswapRouter::new(*UNISWAP_SWAP_ROUTER, provider.clone());
    let deadline = current_unix_timestamp() + 900; // 15 minute validity window keeps calldata realistic.
    let recipient = recipient
        .and_then(|value| Address::from_str(&value).ok())
        .unwrap_or_else(|| signer.address());
    // Build swap calldata using the same parameters we quoted with above.
    let call = router
        .exact_input_single(ExactInputSingleParams {
            token_in: from_token,
            token_out: to_token,
            fee,
            recipient,
            deadline: U256::from(deadline),
            amount_in,
            amount_out_minimum: amount_out_min,
            sqrt_price_limit_x96: sqrt_price_limit_value,
        })
        .value(U256::zero());

    let calldata = call
        .calldata()
        .ok_or_else(|| AppError::Internal("failed to build swap calldata".into()))?
        .clone();

    let tx: TypedTransaction = TransactionRequest::new()
        .to(*UNISWAP_SWAP_ROUTER)
        .from(signer.address())
        .data(calldata.clone())
        .value(U256::zero())
        .into();

    let gas_estimate = provider
        .estimate_gas(&tx, None)
        .await
        .map_err(|err| AppError::Swap(format!("gas estimation failed: {err}")))?;

    provider
        .call(&tx, None)
        .await
        .map_err(|err| AppError::Swap(format!("eth_call simulation failed: {err}")))?;

    let amount_out_decimal = balance::format_with_decimals(&amount_out, to_meta.decimals as u32);
    let amount_out_min_decimal =
        balance::format_with_decimals(&amount_out_min, to_meta.decimals as u32);

    Ok(crate::types::SwapSimOut {
        amount_out_estimate: amount_out_decimal,
        gas_estimate: gas_estimate.to_string(),
        calldata_hex: format!("0x{}", hex::encode(&calldata)),
        router: format!("{:#x}", *UNISWAP_SWAP_ROUTER),
        amount_out_min: amount_out_min_decimal,
    })
}

fn parse_amount(raw: &str) -> AppResult<U256> {
    U256::from_dec_str(raw)
        .map_err(|_| AppError::InvalidInput(format!("invalid numeric value: {raw}")))
}

fn apply_slippage(amount: U256, slippage_bps: u32) -> AppResult<U256> {
    let basis = U256::from(10_000u32);
    let numerator = U256::from(10_000u32 - slippage_bps);
    Ok((amount * numerator) / basis)
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        implementations::{balance, erc20},
        types::SwapTokensParams,
    };
    use ethers::{
        abi::{self, Token},
        providers::{Http, Provider},
        signers::{LocalWallet, Signer},
        types::{Address, U256},
    };
    use serde_json::json;
    use std::{env, str::FromStr, sync::Arc, time::Duration};

    #[test]
    fn slippage_calculation() {
        let amount = U256::from(1_000_000u64);
        let result = apply_slippage(amount, 100).unwrap();
        assert_eq!(result, U256::from(990_000u64));
    }

    #[tokio::test]
    async fn simulate_swap_unit_happy_path() {
        let (mocked_provider, mock) = Provider::mocked();
        let provider = Arc::new(mocked_provider);

        let wallet: LocalWallet = "0x59c6995e998f97a5a0044966f0945382d0b7adf99019cba46777e1fbbf3a1b02"
            .parse()
            .unwrap();
        let wallet = wallet.with_chain_id(1u64);

        let from_token = Address::from_low_u64_be(1);
        let to_token = Address::from_low_u64_be(2);
        let amount_in = U256::from_dec_str("100000000000000000").unwrap(); // 0.1 tokens
        let amount_out = U256::from_dec_str("250000000000000000").unwrap(); // 0.25 tokens

        let decimals_data = abi::encode(&[Token::Uint(U256::from(18u8))]);
        let symbol_data = abi::encode(&[Token::String("TKN".into())]);
        let quote_data = abi::encode(&[
            Token::Uint(amount_out),
            Token::Uint(U256::from(1_000_000u64)),
            Token::Uint(U256::from(25u32)),
            Token::Uint(U256::from(150_000u64)),
        ]);

        // Responses are consumed in reverse order.
        mock.push::<String, _>("0x".to_string()).unwrap(); // provider.call
        mock.push::<String, _>("0x5208".to_string()).unwrap(); // estimate_gas -> 21000
        mock.push::<String, _>(format!("0x{}", hex::encode(&quote_data)))
            .unwrap();
        mock.push::<String, _>(format!("0x{}", hex::encode(&symbol_data)))
            .unwrap();
        mock.push::<String, _>(format!("0x{}", hex::encode(&decimals_data)))
            .unwrap();

        let params = SwapTokensParams {
            from_token: format!("{:#x}", from_token),
            to_token: format!("{:#x}", to_token),
            amount_in_wei: amount_in.to_string(),
            slippage_bps: 100,
            fee: 3_000,
            recipient: None,
            sqrt_price_limit: None,
        };

        let output =
            simulate_swap(provider, wallet, from_token, to_token, params).await.unwrap();

        let expected_amount = balance::format_with_decimals(&amount_out, 18);
        let expected_min =
            balance::format_with_decimals(&apply_slippage(amount_out, 100).unwrap(), 18);

        assert_eq!(output.amount_out_estimate, expected_amount);
        assert_eq!(output.amount_out_min, expected_min);
        assert_eq!(output.gas_estimate, U256::from(0x5208u64).to_string());
        assert_eq!(output.router, format!("{:#x}", *UNISWAP_SWAP_ROUTER));
        assert!(output.calldata_hex.starts_with("0x"));
        assert!(
            !output.calldata_hex.trim_start_matches("0x").is_empty(),
            "expected calldata to be non-empty"
        );
    }

    /// Talks to the real network using credentials from `.env`.
    /// Run manually: `cargo test simulate_swap_real_network_smoke -- --ignored`
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn simulate_swap_real_network_smoke() {
        dotenvy::dotenv().ok();

        let rpc_url = env::var("ETH_RPC_URL")
            .expect("ETH_RPC_URL env var must be set to run simulate_swap_real_network_smoke");
        let private_key = env::var("PRIVATE_KEY")
            .expect("PRIVATE_KEY env var must be set to run simulate_swap_real_network_smoke");

        let base_provider = Provider::<Http>::try_from(rpc_url)
            .expect("failed to connect to ETH_RPC_URL")
            .interval(Duration::from_millis(200));

        let chain_id = base_provider
            .get_chainid()
            .await
            .expect("failed to fetch chain id")
            .as_u64();

        let wallet = private_key
            .parse::<LocalWallet>()
            .or_else(|_| format!("0x{private_key}").parse::<LocalWallet>())
            .expect("PRIVATE_KEY env var is not a valid wallet secret")
            .with_chain_id(chain_id);

        let from_token_env =
            env::var("SWAP_FROM_TOKEN").unwrap_or_else(|_| "0xC02aaa39b223FE8D0A0e5C4F27eAD9083C756Cc2".into());
        let to_token_env =
            env::var("SWAP_TO_TOKEN").unwrap_or_else(|_| "0x6B175474E89094C44Da98b954EedeAC495271d0F".into());
        let amount_in_wei_env =
            env::var("SWAP_AMOUNT_IN_WEI").unwrap_or_else(|_| "100000000000000".into()); // 0.0001 WETH

        let slippage_bps = env::var("SWAP_SLIPPAGE_BPS")
            .ok()
            .map(|value| value.parse::<u32>().expect("SWAP_SLIPPAGE_BPS must be a u32"))
            .unwrap_or(100);

        let fee = env::var("SWAP_POOL_FEE")
            .ok()
            .map(|value| value.parse::<u32>().expect("SWAP_POOL_FEE must be a u32"))
            .unwrap_or(3_000);

        // Exercise serde defaults for SwapTokensParams.
        let params_json = json!({
            "from_token": from_token_env,
            "to_token": to_token_env,
            "amount_in_wei": amount_in_wei_env,
        });

        print!("params_json {:?}", params_json);
        let mut params: SwapTokensParams =
            serde_json::from_value(params_json).expect("failed to deserialize SwapTokensParams");
        assert_eq!(params.slippage_bps, 100, "default slippage_bps should be 100 bps");
        assert_eq!(params.fee, 3_000, "default fee should be 0.3% pool");

        params.slippage_bps = slippage_bps;
        params.fee = fee;
        params.recipient = Some(format!("{:#x}", wallet.address()));

        let from_token =
            Address::from_str(&params.from_token).expect("SWAP_FROM_TOKEN must be a valid address");
        let to_token =
            Address::from_str(&params.to_token).expect("SWAP_TO_TOKEN must be a valid address");
        let amount_in =
            U256::from_dec_str(&params.amount_in_wei).expect("SWAP_AMOUNT_IN_WEI must be decimal");

        let provider = Arc::new(base_provider);

        let balance = erc20::fetch_balance_of(provider.clone(), from_token, wallet.address())
            .await
            .expect("failed to fetch holder balance");

        print!("balance {:?}, amount {:?}", balance,amount_in);
        assert!(
            balance >= amount_in,
            "wallet {:#x} does not have enough balance of {} to cover {} wei",
            wallet.address(),
            params.from_token,
            params.amount_in_wei
        );

        let sim_out = simulate_swap(provider, wallet.clone(), from_token, to_token, params)
            .await
            .expect("simulate_swap failed");

        assert!(
            !sim_out.amount_out_estimate.is_empty(),
            "simulation produced empty amount_out_estimate"
        );
    }
}
