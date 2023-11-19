use crate::error::{ErrorCode, MerxErrorResponse};
use crate::md_handlers::helper::cbag_market_to_exchange;
use crate::subscriptions::{SnapshotStruct, Subscription};
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
struct CbagSnapshotUpdate {
    bid: Vec<(String, String, HashMap<String, String>)>,
    ask: Vec<(String, String, HashMap<String, String>)>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ClientSnapshotUpdate {
    bids: Vec<(String, String, HashMap<String, String>)>,
    asks: Vec<(String, String, HashMap<String, String>)>,
    generated_timestamp: String,
    product: String,
    markets: Vec<String>,
    depth_limit: u32,
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
    //check that state is ready
    if !connection_state.is_ready() {
        match sender.try_send(axum::extract::ws::Message::Text(
            MerxErrorResponse::new(ErrorCode::ServerInitializing).to_json_str(),
        )) {
            Ok(_) => {}
            Err(_try_send_error) => {
                // warn!("Buffer probably full.");
            }
        };
        return;
    }

    let parsed_sub_msg: SubscriptionMessage = match serde_json::from_str(&subscription_msg) {
        Ok(msg) => msg,
        Err(e) => {
            match sender.try_send(axum::extract::ws::Message::Text(
                MerxErrorResponse::new(ErrorCode::InvalidSubscriptionMessage).to_json_str(),
            )) {
                Ok(_) => {}
                Err(_try_send_error) => {
                    // warn!("Buffer probably full.");
                }
            };
            tracing::error!("Error parsing subscription message: {}", e);
            return;
        }
    };

    //check that the currency pair is valid
    match connection_state.is_pair_valid(&parsed_sub_msg.currency_pair) {
        Ok(_) => {}
        Err(merx_error_response) => {
            match sender.try_send(axum::extract::ws::Message::Text(
                merx_error_response.to_json_str(),
            )) {
                Ok(_) => {}
                Err(_try_send_error) => {
                    // warn!("Buffer probably full.");
                }
            };
            return;
        }
    }

    //check that the depth limit is between 1-200
    if let Some(depth_limit) = parsed_sub_msg.depth_limit {
        if !(1..=200).contains(&depth_limit) {
            match sender.try_send(axum::extract::ws::Message::Text(
                MerxErrorResponse::new_and_override_error_text(
                    ErrorCode::InvalidDepthLimit,
                    "Depth limit must be between 1 and 200",
                )
                .to_json_str(),
            )) {
                Ok(_) => {}
                Err(_try_send_error) => {
                    // warn!("Buffer probably full.");
                }
            };
            return;
        }
    }

    if parsed_sub_msg.exchanges.is_empty() {
        match sender.try_send(axum::extract::ws::Message::Text(
            MerxErrorResponse::new_and_override_error_text(
                ErrorCode::InvalidExchanges,
                "No Exchanges Found. At least one exchange must be provided",
            )
            .to_json_str(),
        )) {
            Ok(_) => {}
            Err(_try_send_error) => {
                // warn!("Buffer probably full.");
            }
        };
        return;
    }

    let cbag_markets =
        match connection_state.validate_exchanges_vector(username, &parsed_sub_msg.exchanges) {
            Ok(cbag_markets) => cbag_markets,
            Err(merx_error_response) => {
                match sender.try_send(axum::extract::ws::Message::Text(
                    merx_error_response.to_json_str(),
                )) {
                    Ok(_) => {}
                    Err(_try_send_error) => {
                        // warn!("Buffer probably full.");
                    }
                };
                return;
            }
        };

    let depth_limit = match parsed_sub_msg.depth_limit {
        Some(depth_limit) => depth_limit.to_string(),
        None => String::from("50"),
    };

    let subscription = Subscription::Snapshot(SnapshotStruct::new(
        market_data_type,
        parsed_sub_msg.currency_pair,
        cbag_markets,
        depth_limit.parse::<u32>().unwrap(),
        300,
    ));

    match connection_state.add_client_to_subscription(
        client_address,
        subscription,
        cbag_uri,
        sender.clone(),
        Arc::clone(connection_state),
    ) {
        Ok(_) => {}
        Err(merx_error_response) => {
            match sender.try_send(axum::extract::ws::Message::Text(
                merx_error_response.to_json_str(),
            )) {
                Ok(_) => {}
                Err(_try_send_error) => {
                    // warn!("Buffer probably full.");
                }
            }
        }
    }
}

//TODO: change error to something more specific
pub fn transform_message(message: Message, subscription: &Subscription) -> Result<Message, String> {
    //parse message into LegacyCbboUpdate
    let mut parsed_message: CbagSnapshotUpdate = match serde_json::from_str(&message.to_string()) {
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

    match subscription {
        Subscription::Snapshot(sub) => {
            let generated_timestamp = chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.6f%:z")
                .to_string();

            let markets: Vec<String> = sub
                .cbag_markets
                .iter()
                .map(|market| cbag_market_to_exchange(market))
                .collect();

            let client_snapshot_update = ClientSnapshotUpdate {
                bids: parsed_message.bid,
                asks: parsed_message.ask,
                generated_timestamp,
                depth_limit: sub.depth_limit,
                product: sub.currency_pair.clone(),
                markets: markets,
            };

            let transformed_message = match serde_json::to_string(&client_snapshot_update) {
                Ok(msg) => Message::Text(msg),
                Err(e) => {
                    tracing::error!("Error serializing message: {}", e);
                    return Err(format!("Error serializing message: {}", e));
                }
            };

            Ok(transformed_message)
        }
        _ => {
            tracing::error!("Internal Error: Subscription type not supported for MarketDepthV1");
            Err(String::from(
                "Internal Error: Subscription type not supported for MarketDepthV1",
            ))
        }
    }
}
