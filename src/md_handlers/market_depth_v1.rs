use crate::md_handlers::helper::{cbag_market_to_exchange, exchange_to_cbag_market};
use crate::{routes_config::MarketDataType, state::ConnectionState};
// use futures_channel::mpsc::Sender;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::{collections::HashMap, net::SocketAddr};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::Message;

pub type Tx = Sender<axum::extract::ws::Message>;

#[derive(Debug, Serialize, Deserialize)]
struct SubscriptionMessage {
    currency_pair: String,
    exchanges: Vec<String>,
    depth_limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SnapshotUpdate {
    bid: Vec<(String, String, HashMap<String, String>)>,
    ask: Vec<(String, String, HashMap<String, String>)>,
}

//TODO should this implement a trait?
pub fn handle_subscription(
    client_address: &SocketAddr,
    connection_state: &ConnectionState,
    subscription_msg: String,
    cbag_uri: String,
    mut sender: Tx,
    market_data_type: MarketDataType,
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

    //check that the depth limit is between 1-200
    if let Some(depth_limit) = parsed_sub_msg.depth_limit {
        if depth_limit < 1 || depth_limit > 200 {
            sender
                .try_send(axum::extract::ws::Message::Text(
                    serde_json::json!({"error": "depth_limit must be between 1-200"}).to_string(),
                ))
                .unwrap();
            return;
        }
    }

    if parsed_sub_msg.exchanges.is_empty() {
        sender
            .try_send(axum::extract::ws::Message::Text(
                serde_json::json!({"error": "exchanges must contain at least 1 exchange"})
                    .to_string(),
            ))
            .unwrap();
        return;
    }

    let mut markets: String = String::new();
    for exchange in parsed_sub_msg.exchanges {
        markets.push_str(&&exchange_to_cbag_market(&exchange));
        markets.push_str(",");
    }

    let depth_limit = match parsed_sub_msg.depth_limit {
        Some(depth_limit) => depth_limit.to_string(),
        None => String::from("50"),
    };

    let ws_endpoint: String = format!(
        "/ws/snapshot/{}?markets={}&depth_limit={}&interval_ms=300",
        parsed_sub_msg.currency_pair, markets, depth_limit,
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
    let mut parsed_message: SnapshotUpdate = match serde_json::from_str(&message.to_string()) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Error parsing message: {}", e);
            return Err(format!("Error parsing message: {}", e));
        }
    };

    // change cbag markets to exchange names
    for bid in parsed_message.bid.iter_mut() {
        bid.2 = bid
            .2
            .iter()
            .map(|(market, price)| (cbag_market_to_exchange(market), price.clone()))
            .collect();
    }
    for ask in parsed_message.ask.iter_mut() {
        ask.2 = ask
            .2
            .iter()
            .map(|(market, price)| (cbag_market_to_exchange(market), price.clone()))
            .collect();
    }

    let transformed_message = match serde_json::to_string(&parsed_message) {
        Ok(msg) => Message::Text(msg),
        Err(e) => {
            tracing::error!("Error serializing message: {}", e);
            return Err(format!("Error serializing message: {}", e));
        }
    };

    Ok(transformed_message)
}
