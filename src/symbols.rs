use chrono::prelude::*;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, MerxErrorResponse};

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
pub struct Exchange {
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
    pub exchanges: Vec<Exchange>,
    tick_size: String,
    product_type: Option<String>,
}

#[derive(Default)]
pub struct Symbols {
    pub cbbo_sizes: HashMap<String, Vec<f64>>,
    currency_pairs_json_response: String,
    time_validated: DateTime<Utc>,
    cached_responses: HashMap<String, String>,
}

impl Symbols {
    pub fn add_or_update_symbols(
        &mut self,
        symbols: Symbols,
        currency_pairs_json: String,
    ) -> Result<(), String> {
        self.cbbo_sizes = symbols.cbbo_sizes;
        self.currency_pairs_json_response = currency_pairs_json;
        self.time_validated = Utc::now();
        Ok(())
    }

    pub fn has_symbols(&self) -> bool {
        !self.cbbo_sizes.is_empty()
    }

    pub fn is_pair_valid(&self, pair: &str) -> Result<(), MerxErrorResponse> {
        if self.has_symbols() {
            if self.cbbo_sizes.get(pair).is_some() {
                return Ok(());
            }
            return Err(MerxErrorResponse::new(ErrorCode::InvalidCurrencyPair));
        }
        Err(MerxErrorResponse::new(ErrorCode::AwaitingSymbolData))
    }

    pub fn is_size_filter_valid(
        &self,
        pair: &str,
        size_filter: f64,
    ) -> Result<(), MerxErrorResponse> {
        if self.has_symbols() {
            if let Some(cbbo_sizes) = self.cbbo_sizes.get(pair) {
                for cbbo_size in cbbo_sizes {
                    if size_filter == *cbbo_size {
                        return Ok(());
                    }
                }
                return Err(MerxErrorResponse::new_and_override_error_text(
                    ErrorCode::InvalidSizeFilter,
                    &format!(
                        "{} is an invalid size filter for {}. Available size filters are {:?}",
                        size_filter, pair, cbbo_sizes
                    ),
                ));
            }
            return Err(MerxErrorResponse::new(ErrorCode::InvalidCurrencyPair));
        }
        Err(MerxErrorResponse::new(ErrorCode::AwaitingSymbolData))
    }

    pub fn get_currency_pairs_json(&self) -> Result<String, String> {
        // if there are no symbols return an error
        if !self.has_symbols() {
            return Err("Awaiting symbol data".to_string());
        }
        Ok(self.currency_pairs_json_response.clone())
    }

    pub fn add_or_update_cached_response(&mut self, endpoint: &str, response: String) {
        self.cached_responses.insert(endpoint.to_string(), response);
    }

    pub fn get_cached_response(&self, endpoint: &str) -> Result<String, String> {
        if let Some(response) = self.cached_responses.get(endpoint) {
            return Ok(response.clone());
        }
        //TODO have a better response here
        Err("not found".to_string())
    }
}
