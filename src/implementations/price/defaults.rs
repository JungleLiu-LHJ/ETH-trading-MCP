use std::{collections::HashMap, str::FromStr};

use ethers::types::Address;
use serde::Deserialize;
use serde_json::from_str;

use crate::types::QuoteCurrency;

use super::{TokenInfo, TokenRegistry};

#[derive(Debug, Deserialize)]
struct TokenDefaultsEntry {
    symbol: String,
    address: String,
    decimals: u8,
    #[serde(default)]
    chainlink_feeds: HashMap<QuoteCurrency, String>,
    #[serde(default = "default_fee")]
    default_fee: u32,
}

const DEFAULTS_JSON: &str = include_str!("../../../config/token_defaults.json");

pub(crate) fn populate_defaults(registry: &mut TokenRegistry) {
    let entries: Vec<TokenDefaultsEntry> = from_str(DEFAULTS_JSON)
        .expect("failed to parse token_defaults.json");

    for entry in entries {
        let address = Address::from_str(&entry.address)
            .unwrap_or_else(|_| panic!("invalid token address for {}", entry.symbol));

        let mut info = TokenInfo::new(entry.symbol, address, entry.decimals);

        for (quote, feed_addr) in entry.chainlink_feeds {
            let feed = Address::from_str(&feed_addr)
                .unwrap_or_else(|_| panic!("invalid feed address for {:?}", quote));
            info = info.with_feed(quote, feed);
        }

        info = info.with_fee(entry.default_fee);
        registry.add_token(info);
    }
}

fn default_fee() -> u32 {
    3_000
}
