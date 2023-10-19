use crate::md_handlers::helper::cbag_market_to_exchange;
use crate::{
    functions::{add_client_to_subscription, subscribe_to_market_data, ConnectionState},
    routes_config::MarketDataType,
};
use futures_channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr};
use tokio_tungstenite::tungstenite::Message;

pub type Tx = UnboundedSender<axum::extract::ws::Message>;

#[derive(Debug, Serialize, Deserialize)]
struct SubscriptionMessage {
    currency_pair: String,
    size_filter: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderLevel {
    exchange: String,
    level: u32,
    price: String,
    qty: String,
    total_qty: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct LegacyCbboUpdate {
    // this is the form of the update received from cbag
    product: String,
    size_threshold: String,
    depth_limit: Option<String>,
    generated_timestamp: String,
    processed_timestamp: String,
    markets: Vec<String>,
    bids: Vec<OrderLevel>,
    asks: Vec<OrderLevel>,
    properties: HashMap<String, String>, //TODO: no idea what this is
    last_rx_nanos: u64,
    last_rx_nanos_market: String,
    internal_latency_nanos: u64,
    last_source_nanos: u64,
    last_source_nanos_market: String,
}

//TODO should this implement a trait?
pub fn handle_subscription(
    client_address: &SocketAddr,
    connection_state: &ConnectionState,
    subscription_msg: String,
    cbag_uri: String,
    sender: Tx,
    market_data_type: MarketDataType,
) {
    let parsed_sub_msg: SubscriptionMessage = match serde_json::from_str(&subscription_msg) {
        Ok(msg) => msg,
        Err(e) => {
            //TODO: remove the unwrap from here
            sender
                .unbounded_send(axum::extract::ws::Message::Text(
                    serde_json::json!({"error": "unable to parse subscription message"})
                        .to_string(),
                ))
                .unwrap();
            tracing::error!("Error parsing subscription message: {}", e);
            return;
        }
    };

    let parsed_size_filter: f64 = match parsed_sub_msg.size_filter.parse() {
        Ok(size_filter) => size_filter,
        Err(e) => {
            sender
                .unbounded_send(axum::extract::ws::Message::Text(
                    serde_json::json!({"error": "size_filter must be a number"}).to_string(),
                ))
                .unwrap();
            tracing::error!("Error parsing size filter: {}", e);
            return;
        }
    };

    //validate that size_filter is positive
    if parsed_size_filter < 0.0 {
        sender
            .unbounded_send(axum::extract::ws::Message::Text(
                serde_json::json!({"error": "size_filter must be greater than or equal to 0"})
                    .to_string(),
            ))
            .unwrap();
    }

    let ws_endpoint: String = format!(
        "/ws/legacy-cbbo/{}?quantity_filter={}&interval_ms={}&client={}&source={}&user={}",
        parsed_sub_msg.currency_pair,
        parsed_sub_msg.size_filter,
        1000,
        "merckx",
        "merckx",
        "merckx"
    );

    add_client_to_subscription(
        connection_state,
        client_address,
        &ws_endpoint,
        cbag_uri.clone(),
        sender,
        market_data_type,
    );

    // subscribe_to_market_data(&ws_endpoint, connection_state.clone(), cbag_uri, );
}

//TODO: change error to something more specific
pub fn transform_message(message: Message) -> Result<Message, String> {
    //parse message into LegacyCbboUpdate
    let mut parsed_message: LegacyCbboUpdate = match serde_json::from_str(&message.to_string()) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Error parsing message: {}", e);
            return Err(format!("Error parsing message: {}", e));
        }
    };

    // convert cbag market names to exchange names
    parsed_message.markets = parsed_message
        .markets
        .iter()
        .map(|market| cbag_market_to_exchange(market))
        .collect();

    let transformed_message = match serde_json::to_string(&parsed_message) {
        Ok(msg) => Message::Text(msg),
        Err(e) => {
            tracing::error!("Error serializing message: {}", e);
            return Err(format!("Error serializing message: {}", e));
        }
    };

    Ok(transformed_message)
}
