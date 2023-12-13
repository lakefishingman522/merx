use crate::error::{ErrorCode, MerxErrorResponse};
use crate::md_handlers::helper::cbag_market_to_exchange;
use crate::subscriptions::{LegacyCbboStruct, Subscription};
use crate::{routes_config::MarketDataType, state::ConnectionState};
// use futures_channel::mpsc::Sender;
use core::f64;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::{collections::HashMap, net::SocketAddr};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::Message;
use tracing::info;

pub type Tx = Sender<axum::extract::ws::Message>;

#[derive(Debug, Serialize, Deserialize)]
struct SubscriptionMessage {
    currency_pair: String,
    size_filter: String,
    // sample: Option<f64>,
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
#[allow(dead_code)]
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
    #[serde(skip_serializing)]
    last_rx_nanos_market: Option<String>,
    internal_latency_nanos: Option<u64>,
    last_source_nanos: Option<u64>,
    #[serde(skip_serializing)]
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
    username: Option<String>,
    market_data_id: Option<String>,
    enforce_subscription_whitelist: bool,
) {
    info!("Received subscription message: {}", subscription_msg);
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

    if enforce_subscription_whitelist {
        match connection_state.is_pair_whitelisted(&parsed_sub_msg.currency_pair) {
            Ok(_) => {},
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
    }

    //validate the currency pair
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
            }
            return;
        }
    }

    let parsed_size_filter: f64 = match parsed_sub_msg.size_filter.parse() {
        Ok(size_filter) => size_filter,
        Err(e) => {
            match sender.try_send(axum::extract::ws::Message::Text(
                MerxErrorResponse::new_and_override_error_text(
                    ErrorCode::InvalidSizeFilter,
                    "Could not parse size filter into a decimal",
                )
                .to_json_str(),
            )) {
                Ok(_) => {}
                Err(_try_send_error) => {
                    // warn!("Buffer probably full.");
                }
            };
            tracing::error!("Error parsing size filter: {}", e);
            return;
        }
    };

    //validate that size_filter is positive
    if parsed_size_filter < 0.0 {
        match sender.try_send(axum::extract::ws::Message::Text(
            MerxErrorResponse::new_and_override_error_text(
                ErrorCode::InvalidSizeFilter,
                "Size filter must be positive",
            )
            .to_json_str(),
        )) {
            Ok(_) => {}
            Err(_try_send_error) => {
                // warn!("Buffer probably full.");
            }
        }
        {};
    }

    let interval_ms = 500;
    // let interval_ms: u32 = match parsed_sub_msg.sample {
    //     Some(sample) => {
    //         if !(0.1..=60.0).contains(&sample) {
    //             match sender.try_send(axum::extract::ws::Message::Text(
    //                 MerxErrorResponse::new_and_override_error_text(
    //                     ErrorCode::InvalidSample,
    //                     "Sample must be between 0.1 and 60.0",
    //                 )
    //                 .to_json_str(),
    //             )) {
    //                 Ok(_) => {}
    //                 Err(_try_send_error) => {
    //                     // warn!("Buffer probably full.");
    //                 }
    //             };
    //             return;
    //         }
    //         (sample * 1000.0).round() as u32
    //     }
    //     None => 500,
    // };

    //validate the size filter is valid
    match connection_state.is_size_filter_valid(&parsed_sub_msg.currency_pair, parsed_size_filter) {
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

    // let all_cbag_markets = match connection_state.get_all_cbag_markets_string(username) {
    //     Ok(markets) => markets,
    //     Err(e) => {
    //         sender
    //             .try_send(axum::extract::ws::Message::Text(
    //                 serde_json::json!({ "error": e }).to_string(),
    //             ))
    //             .unwrap();
    //         return;
    //     }
    // };

    let client_id = match username{
        Some(username) => {
            match connection_state.get_client_id(username.as_str()) {
                Ok(id) => Some(id),
                Err(_) => {
                    match sender.try_send(axum::extract::ws::Message::Text(
                        MerxErrorResponse::new(ErrorCode::ServerError).to_json_str(),
                    )) {
                        Ok(_) => {}
                        Err(_try_send_error) => {
                            // warn!("Buffer probably full.");
                        }
                    };
                    return;
                }
            }
        },
        None => None
    };


    let client_id = match client_id {
        Some(client_id) => {
            match market_data_id {
                Some(id) =>  Some(client_id.to_string() + "_" + id.as_str()),
                None => Some(client_id)
            }
        }
        None => None
    };

    let subscription = Subscription::LegacyCbbo(LegacyCbboStruct::new(
        market_data_type,
        parsed_sub_msg.currency_pair,
        parsed_sub_msg.size_filter,
        interval_ms,
        client_id,
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
            };
        }
    }
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

    for bid in parsed_message.bids.iter_mut() {
        bid.exchange = cbag_market_to_exchange(&bid.exchange);
    }
    for ask in parsed_message.asks.iter_mut() {
        ask.exchange = cbag_market_to_exchange(&ask.exchange);
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
