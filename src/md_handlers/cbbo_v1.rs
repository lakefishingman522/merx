use crate::error::{ErrorCode, MerxErrorResponse};
use crate::md_handlers::helper::cbag_market_to_exchange;
use crate::{routes_config::MarketDataType, state::ConnectionState};
// use futures_channel::mpsc::Sender;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::{collections::HashMap, net::SocketAddr};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::Message;

pub type Tx = Sender<axum::extract::ws::Message>;

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
    properties: HashMap<String, Value>, //TODO: no idea what this is
    last_rx_nanos: Option<u64>,
    last_rx_nanos_market: Option<String>,
    internal_latency_nanos: Option<u64>,
    last_source_nanos: Option<u64>,
    last_source_nanos_market: Option<String>,
}

//TODO should this implement a trait?
pub fn handle_subscription(
    client_address: &SocketAddr,
    connection_state: &ConnectionState,
    subscription_msg: String,
    cbag_uri: String,
    sender: Tx,
    market_data_type: MarketDataType,
    username: &str,
) {
    let parsed_sub_msg: SubscriptionMessage = match serde_json::from_str(&subscription_msg) {
        Ok(msg) => msg,
        Err(e) => {
            //TODO: remove the unwrap from here
            sender
                .try_send(axum::extract::ws::Message::Text(
                    serde_json::json!({"error": "unable to parse subscription message"})
                        .to_string(),
                ))
                .unwrap();
            tracing::error!("Error parsing subscription message: {}", e);
            return;
        }
    };

    //validate the currency pair
    match connection_state.is_pair_valid(&parsed_sub_msg.currency_pair) {
        Ok(_) => {}
        Err(merx_error_response) => {
            sender
                .try_send(axum::extract::ws::Message::Text(
                    merx_error_response.to_json_str(),
                ))
                .unwrap();
            return;
        }
    }

    let parsed_size_filter: f64 = match parsed_sub_msg.size_filter.parse() {
        Ok(size_filter) => size_filter,
        Err(e) => {
            sender
                .try_send(axum::extract::ws::Message::Text(
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
            .try_send(axum::extract::ws::Message::Text(
                serde_json::json!({"error": "size_filter must be greater than or equal to 0"})
                    .to_string(),
            ))
            .unwrap();
    }

    //validate the size filter is valid
    match connection_state.is_size_filter_valid(&parsed_sub_msg.currency_pair, parsed_size_filter) {
        Ok(_) => {}
        Err(merx_error_response) => {
            sender
                .try_send(axum::extract::ws::Message::Text(
                    merx_error_response.to_json_str(),
                ))
                .unwrap();
            return;
        }
    }

    let all_cbag_markets = match connection_state.get_all_cbag_markets_string(username) {
        Ok(markets) => markets,
        Err(e) => {
            sender
                .try_send(axum::extract::ws::Message::Text(
                    serde_json::json!({ "error": e }).to_string(),
                ))
                .unwrap();
            return;
        }
    };

    let client_id = match connection_state.get_client_id(username) {
        Ok(id) => id,
        Err(e) => {
            sender
                .try_send(axum::extract::ws::Message::Text(
                    serde_json::json!({ "error": e }).to_string(),
                ))
                .unwrap();
            return;
        }
    };

    // let ws_endpoint: String = format!(
    //     "/ws/legacy-cbbo/{}?quantity_filter={}&interval_ms={}&client={}&source={}&user={}",
    //     parsed_sub_msg.currency_pair,
    //     parsed_sub_msg.size_filter,
    //     1000,
    //     "merx",
    //     all_cbag_markets,
    //     "merx"
    // );

    let ws_endpoint: String = format!(
        "/ws/legacy-cbbo/{}?quantity_filter={}&interval_ms={}&client={}&user={}",
        parsed_sub_msg.currency_pair, parsed_sub_msg.size_filter, 1000, client_id, "merx"
    );

    connection_state.add_client_to_subscription(
        client_address,
        &ws_endpoint,
        cbag_uri.clone(),
        sender,
        market_data_type,
        Arc::clone(&connection_state),
    );

    // subscribe_to_market_data(&ws_endpoint, connection_state.clone(), cbag_uri, );
}

//TODO: change error to something more specific
pub fn transform_message(message: Message) -> Result<Message, String> {
    //parse message into LegacyCbboUpdate
    let mut parsed_message: LegacyCbboUpdate = match serde_json::from_str(&message.to_string()) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Error parsing message: {}", message);
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
