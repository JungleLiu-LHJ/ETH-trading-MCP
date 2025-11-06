use std::sync::Arc;

use ethers::{
    providers::Middleware,
    types::{Address, U256},
};

use crate::{
    error::{AppError, AppResult},
    implementations::erc20,
    types::BalanceOut,
};

/// Resolve ETH or ERC-20 balances depending on whether a token address is supplied.
pub async fn resolve_balance<M>(
    provider: Arc<M>,
    address: Address,
    token: Option<Address>,
) -> AppResult<BalanceOut>
where
    M: Middleware + 'static,
{
    match token {
        Some(token_addr) => resolve_erc20_balance(provider, address, token_addr).await,
        None => resolve_eth_balance(provider, address).await,
    }
}

async fn resolve_eth_balance<M>(provider: Arc<M>, address: Address) -> AppResult<BalanceOut>
where
    M: Middleware + 'static,
{
    let raw_balance = provider
        .get_balance(address, None)
        .await
        .map_err(|err| AppError::Rpc(err.to_string()))?;

    let formatted = format_with_decimals(&raw_balance, 18);

    Ok(BalanceOut {
        symbol: "ETH".to_string(),
        raw: raw_balance.to_string(),
        decimals: 18,
        formatted,
    })
}

async fn resolve_erc20_balance<M>(
    provider: Arc<M>,
    owner: Address,
    token: Address,
) -> AppResult<BalanceOut>
where
    M: Middleware + 'static,
{
    let metadata = erc20::fetch_metadata(provider.clone(), token).await?;
    let raw = erc20::fetch_balance_of(provider, token, owner).await?;
    let formatted = format_with_decimals(&raw, metadata.decimals as u32);

    Ok(BalanceOut {
        symbol: metadata.symbol,
        raw: raw.to_string(),
        decimals: metadata.decimals as u32,
        formatted,
    })
}

/// Format a `U256` amount into a decimal string using the provided number of decimals.
pub fn format_with_decimals(raw: &U256, decimals: u32) -> String {
    if decimals == 0 {
        return raw.to_string();
    }

    let ten = U256::from(10u64);
    let power = ten.pow(U256::from(decimals));
    if power.is_zero() {
        return raw.to_string();
    }

    let integer = raw / power;
    let fraction = raw % power;

    if fraction.is_zero() {
        return integer.to_string();
    }

    let mut fraction_str = fraction.to_string();
    if fraction_str.len() < decimals as usize {
        let padding = decimals as usize - fraction_str.len();
        let prefix = "0".repeat(padding);
        fraction_str = format!("{prefix}{fraction_str}");
    }

    let trimmed_fraction = fraction_str.trim_end_matches('0').to_string();
    if trimmed_fraction.is_empty() {
        integer.to_string()
    } else {
        format!("{}.{}", integer, trimmed_fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::{
        core::abi::{encode, Token},
        providers::{Http, MockProvider, Provider},
    };
    use std::sync::Arc;
    use std::env;

    #[test]
    fn formats_without_decimals() {
        let value = U256::from(123u64);
        assert_eq!(format_with_decimals(&value, 0), "123");
    }

    #[test]
    fn formats_with_decimals() {
        let value = U256::from_dec_str("123456000000000000000").unwrap();
        assert_eq!(format_with_decimals(&value, 18), "123.456");
    }

    #[test]
    fn trims_trailing_zeroes() {
        let value = U256::from_dec_str("1000000000000000000").unwrap();
        assert_eq!(format_with_decimals(&value, 18), "1");
    }

    #[tokio::test]
    async fn resolve_eth_balance_formats_expected_output() {
        let mock = MockProvider::new();
        mock.push::<String, _>("0xde0b6b3a7640000".to_string()).unwrap(); // 1 ETH in wei

        let provider = Arc::new(Provider::new(mock));
        let address = Address::from_low_u64_be(1);

        let balance = super::resolve_eth_balance(provider, address).await.unwrap();

        assert_eq!(balance.symbol, "ETH");
        assert_eq!(balance.decimals, 18);
        assert_eq!(balance.raw, "1000000000000000000");
        assert_eq!(balance.formatted, "1");
    }

    #[tokio::test]
    async fn resolve_erc20_balance_uses_contract_metadata() {
        let mock = MockProvider::new();
        let raw_balance = U256::from(1_500_000u64);
        let balance_data = encode(&[Token::Uint(raw_balance)]);
        let symbol_data = encode(&[Token::String("TKN".to_string())]);
        let decimals_data = encode(&[Token::Uint(U256::from(6u8))]);

        // Responses are consumed in reverse order, so push balance first.
        mock.push::<String, _>(format!("0x{}", hex::encode(balance_data))).unwrap();
        mock.push::<String, _>(format!("0x{}", hex::encode(symbol_data))).unwrap();
        mock.push::<String, _>(format!("0x{}", hex::encode(decimals_data))).unwrap();

        let provider = Arc::new(Provider::new(mock));
        let owner = Address::from_low_u64_be(42);
        let token = Address::from_low_u64_be(7);

        let balance = super::resolve_erc20_balance(provider, owner, token).await.unwrap();

        assert_eq!(balance.symbol, "TKN");
        assert_eq!(balance.decimals, 6);
        assert_eq!(balance.raw, raw_balance.to_string());
        assert_eq!(balance.formatted, "1.5");
    }

    #[tokio::test]
    #[ignore = "Requires real RPC endpoint and funded address"]
    async fn resolve_eth_balance_live_fetches_real_value() {
        dotenvy::dotenv().ok();
        let rpc_url =
            env::var("ETH_RPC_URL").expect("set ETH_RPC_URL for live balance test");
        let address = env::var("BALANCE_TEST_ADDRESS")
            .expect("set BALANCE_TEST_ADDRESS")
            .parse::<Address>()
            .expect("BALANCE_TEST_ADDRESS must be a valid address");

        let provider = Arc::new(
            Provider::<Http>::try_from(rpc_url.as_str()).expect("failed to create provider"),
        );

        let balance = super::resolve_balance(provider, address, None)
            .await
            .expect("balance lookup failed");
        println!("Live ETH balance: {:?}", balance);
        assert_eq!(balance.symbol, "ETH");
    }

    #[tokio::test]
    #[ignore = "Requires real RPC endpoint and ERC-20 token configuration"]
    async fn resolve_erc20_balance_live_fetches_real_value() {
        dotenvy::dotenv().ok();
        let rpc_url =
            env::var("ETH_RPC_URL").expect("set ETH_RPC_URL for live ERC20 balance test");
        let address = env::var("BALANCE_TEST_ADDRESS")
            .expect("set BALANCE_TEST_ADDRESS")
            .parse::<Address>()
            .expect("BALANCE_TEST_ADDRESS must be a valid address");
        let token_address = env::var("BALANCE_TEST_TOKEN")
            .expect("set BALANCE_TEST_TOKEN")
            .parse::<Address>()
            .expect("BALANCE_TEST_TOKEN must be a valid address");

        let provider = Arc::new(
            Provider::<Http>::try_from(rpc_url.as_str()).expect("failed to create provider"),
        );

        let balance = super::resolve_balance(provider, address, Some(token_address))
            .await
            .expect("token balance lookup failed");
        println!("Live ERC-20 balance: {:?}", balance);
        assert!(!balance.symbol.is_empty());
    }
}
