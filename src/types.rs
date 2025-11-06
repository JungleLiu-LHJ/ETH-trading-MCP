use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Deserialize)]
pub struct GetBalanceParams {
    pub address: String,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BalanceOut {
    pub symbol: String,
    pub raw: String,
    pub decimals: u32,
    pub formatted: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "UPPERCASE")]
pub enum QuoteCurrency {
    USD,
    ETH,
}

impl Default for QuoteCurrency {
    fn default() -> Self {
        QuoteCurrency::USD
    }
}

impl fmt::Display for QuoteCurrency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuoteCurrency::USD => write!(f, "USD"),
            QuoteCurrency::ETH => write!(f, "ETH"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GetTokenPriceParams {
    pub base: String,
    #[serde(default)]
    pub quote: QuoteCurrency,
}

#[derive(Debug, Serialize)]
pub struct PriceOut {
    pub base: String,
    pub quote: String,
    pub price: String,
    pub source: String,
    pub decimals: u32,
}

#[derive(Debug, Deserialize)]
pub struct SwapTokensParams {
    pub from_token: String,
    pub to_token: String,
    pub amount_in_wei: String,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: u32,
    #[serde(default = "default_fee")]
    pub fee: u32,
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub sqrt_price_limit: Option<String>,
}

fn default_slippage_bps() -> u32 {
    100 // 1%
}

fn default_fee() -> u32 {
    3_000
}

#[derive(Debug, Serialize)]
pub struct SwapSimOut {
    pub amount_out_estimate: String,
    pub gas_estimate: String,
    pub calldata_hex: String,
    pub router: String,
    pub amount_out_min: String,
}
