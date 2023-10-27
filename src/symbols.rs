use chrono::prelude::*;
use chrono::Duration;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Currency {
    slug: String,
    cbbo_sizes: Vec<CbboSize>,
    default_min_size: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CbboSize {
    id: u32,
    size: String,
    usd_value: Option<f64>,
    currency: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Exchange {
    slug: String,
    name: String,
    api_name: Option<String>,
    api_symbol: Option<String>,
    maker_fee: Option<f64>,
    taker_fee: Option<f64>,
    tick_size: String,
    max_post_size: Option<String>,
    min_qty_incr: String,
    min_order_size: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CurrencyPairsResponse {
    slug: String,
    target_currency: Currency,
    funding_currency: Currency,
    exchanges: Vec<Exchange>,
    tick_size: String,
    product_type: String,
}

pub struct Symbols {
    cbbo_sizes: HashMap<String, Vec<f64>>,
    time_validated: DateTime<Utc>,
}
