/*
When making subscriptions to cbag, all subscriptions are held in
connection state so that subscriptions are not duplicated.
This is done by using a hashmap with the key being the subscription.
All subscriptions are part of the same enum so that they can be hashed.
The enum object hold all relevant parameters for each subscriptions.
This further enables easy use of subscription parameters when transforming
the websocket response, or adding parameters to it
before sending it to the clients.
*/

use crate::routes_config::MarketDataType;

#[derive(Eq, Hash, PartialEq, Clone)]
pub enum Subscription {
    Snapshot(SnapshotStruct),
    LegacyCbbo(LegacyCbboStruct),
    Direct(DirectStruct),
}

impl SubTraits for Subscription {
    fn get_url(&self) -> String {
        match self {
            Subscription::Snapshot(sub) => sub.get_url(),
            Subscription::LegacyCbbo(sub) => sub.get_url(),
            Subscription::Direct(sub) => sub.get_url(),
        }
    }

    fn get_market_data_type(&self) -> MarketDataType {
        match self {
            Subscription::Snapshot(sub) => sub.get_market_data_type(),
            Subscription::LegacyCbbo(sub) => sub.get_market_data_type(),
            Subscription::Direct(sub) => sub.get_market_data_type(),
        }
    }

    fn get_timeout_duration_ms(&self) -> u64 {
        match self {
            Subscription::Snapshot(sub) => sub.get_timeout_duration_ms(),
            Subscription::LegacyCbbo(sub) => sub.get_timeout_duration_ms(),
            Subscription::Direct(sub) => sub.get_timeout_duration_ms(),
        }
    }
}

pub trait SubTraits {
    fn get_url(&self) -> String;
    fn get_market_data_type(&self) -> MarketDataType;
    fn get_timeout_duration_ms(&self) -> u64;
}

#[derive(Eq, Hash, PartialEq, Clone)]
pub struct SnapshotStruct {
    market_data_type: MarketDataType,
    pub currency_pair: String,
    pub cbag_markets: Vec<String>,
    pub depth_limit: u32,
    interval_ms: u32,
}

impl SnapshotStruct {
    pub fn new(
        market_data_type: MarketDataType,
        currency_pair: String,
        cbag_markets: Vec<String>,
        depth_limit: u32,
        interval_ms: u32,
    ) -> Self {
        Self {
            market_data_type,
            currency_pair,
            cbag_markets,
            depth_limit,
            interval_ms,
        }
    }
}

impl SubTraits for SnapshotStruct {
    fn get_url(&self) -> String {
        let mut markets_string: String = String::new();
        for market in &self.cbag_markets {
            markets_string.push_str(market);
            markets_string.push(',');
        }

        format!(
            "/ws/snapshot/{}?markets={}&depth_limit={}&interval_ms={}",
            self.currency_pair, markets_string, self.depth_limit, self.interval_ms
        )
    }

    fn get_market_data_type(&self) -> MarketDataType {
        self.market_data_type
    }

    fn get_timeout_duration_ms(&self) -> u64 {
        // timeout should be at least 2500 ms or 4x interval
        std::cmp::max(self.interval_ms as u64 * 4, 2500)
    }
}

#[derive(Eq, Hash, PartialEq, Clone)]
pub struct LegacyCbboStruct {
    market_data_type: MarketDataType,
    currency_pair: String,
    size_filter: String,
    interval_ms: u32,
    client_id: Option<String>,
}

impl SubTraits for LegacyCbboStruct {
    fn get_url(&self) -> String {
        match &self.client_id {
            Some(client_id) => {
                format!(
                    "/ws/legacy-cbbo/{}?quantity_filter={}&interval_ms={}&client={}&user=merx",
                    self.currency_pair, self.size_filter, self.interval_ms, client_id
                )
            }
            None => {
                format!(
                    "/ws/legacy-cbbo/{}?quantity_filter={}&interval_ms={}&user=merx",
                    self.currency_pair, self.size_filter, self.interval_ms
                )
            }
        }
    }

    fn get_market_data_type(&self) -> MarketDataType {
        self.market_data_type
    }

    fn get_timeout_duration_ms(&self) -> u64 {
        // timeout should be at least 2500 ms or 4x interval
        std::cmp::max(self.interval_ms as u64 * 4, 2500)
    }
}

impl LegacyCbboStruct {
    pub fn new(
        market_data_type: MarketDataType,
        currency_pair: String,
        size_filter: String,
        interval_ms: u32,
        client_id: Option<String>,
    ) -> Self {
        Self {
            market_data_type,
            currency_pair,
            size_filter,
            interval_ms,
            client_id,
        }
    }
}

#[derive(Eq, Hash, PartialEq, Clone)]
pub struct DirectStruct {
    market_data_type: MarketDataType,
    endpoint: String,
}

impl DirectStruct {
    pub fn new(market_data_type: MarketDataType, endpoint: String) -> Self {
        Self {
            market_data_type,
            endpoint,
        }
    }
}

impl SubTraits for DirectStruct {
    fn get_url(&self) -> String {
        self.endpoint.clone()
    }

    fn get_market_data_type(&self) -> MarketDataType {
        self.market_data_type
    }

    fn get_timeout_duration_ms(&self) -> u64 {
        // set a 10 second timeout for direct subscriptions
        10000
    }
}

//
use chrono::{DateTime, Utc};

#[derive(Clone)]
pub struct TimeLog {
    pub temp: DateTime<Utc>,
    // pub tried: u8,
    pub locked: Option<DateTime<Utc>>,
}
