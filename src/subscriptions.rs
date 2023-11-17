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

enum Subscription {
    MarketDepthV1,
    CbboV1,
}

struct Snapshot {
    market_data_type: MarketDataType,
    currency_pair: String,
    cbag_markets: Vec<String>,
    depth_limit: u32,
    interval_ms: u32,
}

impl Snapshot {
    fn get_url(&self) -> String {
        let mut markets_string: String = String::new();
        for market in self.cbag_markets {
            markets_string.push_str(&&market);
            markets_string.push_str(",");
        }

        let depth_limit = match self.depth_limit {
            Some(depth_limit) => depth_limit.to_string(),
            None => String::from("50"),
        };

        let ws_endpoint: String = format!(
            "/ws/snapshot/{}?markets={}&depth_limit={}&interval_ms=300",
            self.currency_pair, markets_string, depth_limit,
        );
    }
}

struct LegacyCbbo {
    market_data_type: MarketDataType,
    currency_pair: String,
    exchanges: Vec<String>,
    size_filter: f64,
    interval_ms: u32,
    client_id: String,
}
