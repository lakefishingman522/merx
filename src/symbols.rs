use chrono::prelude::*;
use chrono::Duration;
use log::info;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Currency {
    slug: String,
    pub cbbo_sizes: Vec<CbboSize>,
    default_min_size: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CbboSize {
    id: u32,
    pub size: String,
    usd_value: Option<f64>,
    currency: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Exchange {
    slug: String,
    name: String,
    api_name: Option<String>,
    api_symbol: Option<String>,
    maker_fee: Option<String>,
    taker_fee: Option<String>,
    tick_size: Option<String>,
    max_post_size: Option<String>,
    min_qty_incr: Option<String>,
    min_order_size: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CurrencyPairsResponse {
    pub slug: String,
    pub target_currency: Currency,
    funding_currency: Currency,
    exchanges: Vec<Exchange>,
    tick_size: String,
    product_type: String,
}

#[derive(Default)]
pub struct Symbols {
    pub cbbo_sizes: HashMap<String, Vec<f64>>,
    currency_pairs_json_response: String,
    time_validated: DateTime<Utc>,
}

impl Symbols {
    pub fn add_or_update_symbols(
        &mut self,
        symbols: Symbols,
        currency_pairs_json: String,
    ) -> Result<(), String> {
        self.cbbo_sizes = symbols.cbbo_sizes.clone();
        self.currency_pairs_json_response = currency_pairs_json;
        self.time_validated = Utc::now();
        Ok(())
    }

    pub fn has_symbols(&self) -> bool {
        !self.cbbo_sizes.is_empty()
    }

    pub fn is_pair_valid(&self, pair: &str) -> bool {
        self.cbbo_sizes.contains_key(pair)
    }

    pub fn is_size_filter_valid(&self, pair: &str, size_filter: f64) -> Result<(), String> {
        if let Some(cbbo_sizes) = self.cbbo_sizes.get(pair) {
            for cbbo_size in cbbo_sizes {
                if size_filter == *cbbo_size {
                    return Ok(());
                }
            }
            return Err(format!(
                "{} is an invalid size filter for {}. Available size filters are {:?}",
                size_filter, pair, cbbo_sizes
            ));
        }
        return Err("Invalid currency pair".to_string());
    }
}
