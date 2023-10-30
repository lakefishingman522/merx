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
    time_validated: DateTime<Utc>,
}

impl Symbols {
    // pub fn add_or_update_symbols(
    //     &mut self,
    //     currency_pairs_response_vec: &Vec<CurrencyPairsResponse>,
    // )->Result<(), String> {
    //     //TODO: this is done under a write lock, not good
    //     let cbbo_sizes: HashMap<String, Vec<f64>> = HashMap::new();
    //     for currency_pair in currency_pairs_response_vec {
    //         let mut cbbo_sizes_vec = Vec::new();
    //         for cbbo_size in &currency_pair.target_currency.cbbo_sizes {
    //                 //convert to f64
    //                 let cbbo_size = match cbbo_size.size.parse::<f64>() {
    //                     Ok(cbbo_size) => cbbo_size,
    //                     Err(e) => {
    //                         info!("Unable to parse cbbo_size: {:?}", e);
    //                         return Err("Unable to parse cbbo_size".to_string());
    //                     }
    //                 };
    //                 cbbo_sizes_vec.push(cbbo_size);
    //         }
    //         self.cbbo_sizes.insert(currency_pair.slug.clone(), cbbo_sizes_vec);
    //     }
    //     self.cbbo_sizes = cbbo_sizes;
    //     self.time_validated = Utc::now();
    //     Ok(())
    // }

    pub fn add_or_update_symbols(&mut self, symbols: Symbols) -> Result<(), String> {
        self.cbbo_sizes = symbols.cbbo_sizes.clone();
        self.time_validated = Utc::now();
        Ok(())
    }
}
