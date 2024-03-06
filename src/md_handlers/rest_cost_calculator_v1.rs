use crate::error::{ErrorCode, MerxErrorResponse};
use crate::subscriptions::{DirectStruct, Subscription};
use std::{collections::HashMap, net::SocketAddr};

use axum::{
    body::Body,
    extract::{OriginalUri, Query, State}, // Path,Json,
    http::{HeaderMap, Response},          // , Uri Request,
    response::IntoResponse,
    Extension,
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::auth::check_token_and_authenticate;
use crate::functions::URIs;
use crate::{
    routes_config::{MarketDataType, WebSocketLimitRoute},
    state::ConnectionState,
};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::Message;
use tracing::info;

pub type Tx = Sender<axum::extract::ws::Message>;

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct RestCostCalculatorV1RequestBody {
    currency_pair: String,
    exchanges: Vec<String>,
    quantity: String,
    side: String,
    use_fees: Option<bool>,
    use_funding_currency: Option<bool>,
}

pub async fn handle_request(
    State(connection_state): State<ConnectionState>,
    original_uri: OriginalUri,
    // Path(currency_pair): Path<String>,
    _uris: Extension<URIs>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    // req: Request<Body>,
    // Json(_body): Json<RestCostCalculatorV1RequestBody>,
) -> impl IntoResponse {
    match check_token_and_authenticate(
        &headers,
        &Query(params.clone()),
        &_uris.auth_uri,
        connection_state.clone(),
    )
    .await
    {
        Ok(_) => {
            info!(
                "Received Authenticated REST Forward Request {}",
                original_uri.0
            );
            let uri = original_uri.0.clone();

            let mut target_url = url::Url::parse(&format!(
                "http://{}/cost-calculator/{}",
                _uris.cbag_uri,
                uri.path().trim_start_matches("/api/cost_calculator/")
            ))
            .unwrap();

            for (key, value) in &params {
                target_url.query_pairs_mut().append_pair(key, value);
            }

            info!("Authenticated REST Forward Request to {}", target_url);

            // Make a GET request to the target server
            let client = Client::new();
            let target_res = client.get(target_url).send().await;

            match target_res {
                Ok(response) => {
                    // Extract the status code
                    let status = StatusCode::from_u16(response.status().as_u16()).unwrap();
                    // Extract the headers
                    let headers = response.headers().clone();
                    // Extract the body as bytes
                    let body_bytes = response.bytes().await.unwrap();
                    // Create an Axum response using the extracted parts
                    let mut axum_response = Response::new(Body::from(body_bytes));
                    *axum_response.status_mut() = status;
                    axum_response.headers_mut().extend(headers);

                    axum_response
                }
                Err(_err) => {
                    // Build a 404 response with a body of type `axum::http::response::Body`
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Body::from("Not Found"))
                        .unwrap()
                }
            }
        }
        Err(_) => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("Unauthorized".to_string()))
            .unwrap(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SubscriptionMessage {
    currency_pair: String,
    exchanges: Vec<String>,
    quantities: Vec<f64>,
}

pub fn handle_subscription(
    client_address: &SocketAddr,
    connection_state: &ConnectionState,
    subscription_msg: String,
    cbag_uri: String,
    sender: Tx,
    market_data_type: MarketDataType,
    username: &str,
    market_data_id: Option<String>,
    websocketlimit_route: Option<&WebSocketLimitRoute>,
) {
    info!(
        "Received otc cost calculator subscription message: {}",
        subscription_msg
    );

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

    let cbag_markets = match connection_state.validate_exchanges_vector(
        username,
        &parsed_sub_msg.exchanges,
        market_data_id,
    ) {
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

    // Convert quantities to a comma-separated string
    let quantities_str = parsed_sub_msg
        .quantities
        .iter()
        .map(|quantity| quantity.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let exchanges_str = cbag_markets
        .iter()
        .map(|exchange| {
            // Remove 'sim_' prefix if present
            let trimmed_exchange = if exchange.starts_with("SIM_") {
                &exchange["SIM_".len()..]
            } else {
                exchange
            };
            // Map to specific names or convert to uppercase
            match trimmed_exchange.to_lowercase().as_str() {
                "gdax" => String::from("COINBASE"),
                "huobipro" => String::from("HUOBI"),
                _ => trimmed_exchange.to_uppercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    // Construct the WebSocket URL string
    let ws_endpoint = format!(
        "/ws/cost-calculator/{}?side=both&target_quantity={}&interval_ms=2500&markets={}",
        parsed_sub_msg.currency_pair, quantities_str, exchanges_str
    );

    let subscription = Subscription::Direct(DirectStruct::new(market_data_type, ws_endpoint));

    match connection_state.add_client_to_subscription(
        client_address,
        subscription,
        cbag_uri,
        sender.clone(),
        Arc::clone(connection_state),
        websocketlimit_route,
        username.to_string(),
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

#[derive(Serialize, Deserialize, Debug)]
struct InputData {
    side: String,
    markets: String,
    target_quantity: String,
    sweep_orders: HashMap<String, Vec<String>>,
    totals: HashMap<String, serde_json::Value>,
    // others if needed
}

#[derive(Serialize, Deserialize, Debug)]
struct RecievedMessage {
    data: Vec<InputData>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OutputMessage {
    // symbol: String,
    asks: HashMap<String, Vec<Order>>,
    bids: HashMap<String, Vec<Order>>,
    errors: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Order {
    price: String,
    //
    totals: HashMap<String, serde_json::Value>, // Placeholder
    orders: HashMap<String, serde_json::Value>, // Placeholder
}

pub fn transform_message(message: Message) -> Result<Message, String> {
    let mut asks = HashMap::new();
    let mut bids = HashMap::new();

    let parsed_message: RecievedMessage = match serde_json::from_str(&message.to_string()) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Error parsing message: {}", message);
            tracing::error!("Error parsing message: {}", e);
            return Err(format!("Error parsing message: {}", e));
        }
    };

    for item in parsed_message.data {
        let side = if item.side == "buy" { "bids" } else { "asks" };
        let price = calculate_price(&item);

        let order = Order {
            price,
            totals: HashMap::new(), // Fill in as required
            orders: HashMap::new(), // Fill in as required
        };

        let quantity_key = item.target_quantity;
        match side {
            "bids" => bids
                .entry(quantity_key)
                .or_insert_with(Vec::new)
                .push(order),
            "asks" => asks
                .entry(quantity_key)
                .or_insert_with(Vec::new)
                .push(order),
            _ => return Err("Unrecognized side in data".to_string()),
        }
    }

    let converted_message = OutputMessage {
        // symbol,
        asks,
        bids,
        errors: None, // Populate based on your error handling logic
    };

    let transformed_message = match serde_json::to_string(&converted_message) {
        Ok(msg) => Message::Text(msg),
        Err(e) => {
            tracing::error!("Error serializing message: {}", e);
            return Err(format!("Error serializing message: {}", e));
        }
    };
    Ok(transformed_message)
}

fn calculate_price(_item: &InputData) -> String {
    return "100".to_string();
}
