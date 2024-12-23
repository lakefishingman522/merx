use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use futures::future;
use futures::stream::TryStreamExt;
use futures_util::{pin_mut, SinkExt, StreamExt};
// use futures_util::{pin_mut};
// use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
// use futures_channel::mpsc::{channel, Receiver, Sender};
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    Mutex,
};
// Import the TryStreamExt trait
use lazy_static::lazy_static;
use tokio::time::{sleep, Duration, Instant};

// for axum
use axum::{
    body::Body,
    extract::ws::{CloseFrame, Message as axum_Message},
    http::Uri,
};
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    TypedHeader,
};

use axum::headers;

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::Extension;
use reqwest::Client;
use tracing::{error, info, warn};

// use crate::routes_config::{ROUTES, SUB_TYPE};

use crate::auth::check_token_and_authenticate;
use crate::md_handlers::{cbbo_v1, market_depth_v1, rest_cost_calculator_v1};
use crate::routes_config::{
    MarketDataType, SubscriptionType, WebSocketLimitRoute, ROUTES, SUB_TYPE, WS_LIMIT_ROUTES,
};
use crate::state::{fetch_chart, ConnectionState};
use crate::subscriptions::{DirectStruct, Subscription};

pub type Tx = Sender<axum::extract::ws::Message>;

lazy_static! {
    // bound for the number of messages to hold in a channel
    static ref SENDER_BOUND: usize = 500;
}

const MAX_SESSION_TIME: Duration = Duration::from_secs(60 * 60 * 3);

#[derive(Clone)]
pub struct URIs {
    pub cbag_uri: String,
    pub cbag_depth_uri: String,
    pub auth_uri: String,
    pub chart_uri: String,
    pub trades_uri: String,
}

pub async fn fallback(uri: Uri, OriginalUri(original_uri): OriginalUri) -> (StatusCode, String) {
    warn!("Request for unknown route {}", original_uri);
    (StatusCode::NOT_FOUND, format!("No route for {}", uri))
}

pub async fn forward_request(
    OriginalUri(original_uri): OriginalUri,
    Extension(cbag_uri): Extension<String>,
) -> impl IntoResponse {
    info!("Received REST Forward Request {}", original_uri);
    let target_url = format!("http://{}{}", cbag_uri, original_uri);

    // Make a GET request to the target server
    let client = Client::new();
    let target_res = client.get(&target_url).send().await;

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

pub async fn root() -> &'static str {
    "Service is running"
}

#[axum_macros::debug_handler]
pub async fn axum_ws_handler(
    // Path(_symbol): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    OriginalUri(original_uri): OriginalUri,
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    // authorization_header: Option<TypedHeader<headers::Authorization<Token>>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    req: Request<Body>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    info!("{addr} connected User-Agent {user_agent}. Requesting subscription: {original_uri}");

    let base_route = req.uri().path().trim_end_matches('/');

    if !ROUTES.contains_key(base_route) {
        warn!("Endpoint not available {}", base_route);
        return (StatusCode::BAD_REQUEST, "Endpoint not available").into_response();
        //TODO
    }
    let route = ROUTES.get(base_route).unwrap();

    if !SUB_TYPE.contains_key(base_route) {
        warn!("Endpoint not available {}", base_route);
        return (StatusCode::BAD_REQUEST, "Endpoint not available").into_response();
        //TODO
    }
    let subscription_type = SUB_TYPE.get(base_route).unwrap();
    let websocketlimit_route = WS_LIMIT_ROUTES.get(base_route);

    let auth_uri = uris.auth_uri.clone();
    let uris_clone = uris.clone();

    // if this isn't a direct subscription, then we have to authenticate
    let username = match subscription_type {
        SubscriptionType::Direct => "DIRECT".to_string(),
        SubscriptionType::PublicSubscription => "PUBLIC".to_string(),
        _ => {
            match check_token_and_authenticate(
                &headers,
                &Query(params.clone()),
                &auth_uri,
                connection_state.clone(),
            )
            .await
            {
                Ok(username) => username,
                Err(_) => {
                    warn!("Unable to authenticate token");
                    return (StatusCode::UNAUTHORIZED, "Unable to authenticate token")
                        .into_response();
                }
            }
        }
    };

    let market_data_id = params.get("market_data_id").cloned();
    // we can customize the callback by sending additional info such as original_uri.
    ws.on_upgrade(move |socket| {
        axum_handle_socket(
            socket,
            addr,
            original_uri,
            connection_state,
            uris_clone,
            route,
            subscription_type,
            websocketlimit_route,
            username,
            market_data_id,
        )
    })
}

/// Actual websocket state machine (one will be spawned per connection)
// #[axum_macros::debug_handler]
async fn axum_handle_socket(
    websocket: WebSocket,
    client_address: SocketAddr,
    request_endpoint: Uri,
    connection_state: ConnectionState,
    uris: URIs,
    market_data_type: &MarketDataType,
    subscription_type: &SubscriptionType,
    websocketlimit_route: Option<&WebSocketLimitRoute>,
    username: String,
    market_data_id: Option<String>,
) {
    // added by karun
    let (tx, mut rx): (Sender<axum_Message>, Receiver<axum_Message>) = channel(*SENDER_BOUND);
    // let (tx, rx) = unbounded();

    // let thread_id = thread::current().id();
    // println!("This code is running on thread {:?}", thread_id);

    let request_endpoint_str = request_endpoint.to_string();
    let tx_task_tx = tx.clone();
    let recv_task_cbag_uri = uris.cbag_uri.clone();
    let recv_task_cbag_depth_uri = uris.cbag_depth_uri.clone();
    let (outgoing, incoming) = websocket.split();
    let outgoing = Arc::new(Mutex::new(outgoing));

    // add the client to the connection state. If the url isn't already subscribed
    // to, we need to spawn a process to subscribe to the url. This has to be done
    // whilst there is a write lock on the connection state in case multiple
    // clients requesting the same url connect at the same time to avoid
    // race conditions

    // if its a direct subscription we can add it straight away
    // else we will wait for the client to send us subscription messages
    if matches!(subscription_type, SubscriptionType::Direct) {
        let subscription = Subscription::Direct(DirectStruct::new(
            *market_data_type,
            request_endpoint_str.clone(),
        ));

        match connection_state.add_client_to_subscription(
            &client_address,
            subscription,
            uris.cbag_uri,
            tx.clone(),
            Arc::clone(&connection_state),
            websocketlimit_route,
            username.clone(),
        ) {
            Ok(_) => {}
            Err(merx_error_response) => {
                match tx.try_send(axum::extract::ws::Message::Text(
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

    let connection_state_clone = connection_state.clone();
    let client_address_clone = client_address;
    let subscription_type_clone = subscription_type.clone();
    let username_clone = username.clone();

    let outgoing_clone = Arc::clone(&outgoing);
    let check_subscription_still_active = tokio::spawn(async move {
        // save the time at this point
        let connection_time = tokio::time::Instant::now();
        loop {
            sleep(Duration::from_millis(1000)).await;
            {
                if (matches!(subscription_type_clone, SubscriptionType::Subscription)
                    || matches!(
                        subscription_type_clone,
                        SubscriptionType::PublicSubscription
                    ))
                    && (tokio::time::Instant::now() - connection_time < Duration::from_secs(20))

                {
                    continue;
                }

                if !connection_state_clone.is_client_still_active(&client_address) {
                    info!(
                        "Client {} {} Disconnected or subscription no longer active. ending check task",
                        &client_address_clone, &username_clone
                    );
                    // Send close_frame msg to websocket client
                    let close_frame = CloseFrame {
                        code: 1000, // Normal closure
                        reason: "Subscription no longer active".into(),
                    };

                    let close_msg = axum_Message::Close(Some(close_frame));
                    let mut out = outgoing_clone.lock().await;
                    if let Err(e) = out.send(close_msg).await {
                        warn!("Error sending close message: {}", e);
                    }
                    return;
                }
                if matches!(
                    subscription_type_clone,
                    SubscriptionType::PublicSubscription
                ) {
                    if exceeds_max_session_time(connection_time) {
                        info!("exceed max session time");

                        // Send close_frame msg to websocket client
                        let close_frame = CloseFrame {
                            code: 1000, // Normal closure
                            reason: "Exceeds max session time".into(),
                        };
                        let close_msg = axum_Message::Close(Some(close_frame));
                        let mut out = outgoing_clone.lock().await;
                        if let Err(e) = out.send(close_msg).await {
                            warn!("Error sending close message: {}", e);
                        }

                        return;
                    }
                }
            }
        }
    });

    // this is unused but good to log incase there are incoming messages
    // let broadcast_incoming = incoming.try_for_each(|msg| {
    //     info!(
    //         "Received a message from {}: {}",
    //         client_address,
    //         msg.to_text().unwrap()
    //     );
    //     future::ok(())
    // });

    let recv_task_connection_state = connection_state.clone();
    let recv_task_username = username.clone();

    // this is unused but good to log incase there are incoming messages
    let recv_task = incoming.try_for_each(|msg| {
        match msg {
            axum::extract::ws::Message::Close(_msg) => {
                info!(
                    "{} {} Received a close frame",
                    &client_address, recv_task_username
                );
            }
            axum::extract::ws::Message::Ping(_msg) => {
                // info!("{} Received a ping frame", &client_address);
                // send back a pong
                let pong_msg = axum_Message::Pong(_msg);
                match tx_task_tx.clone().try_send(pong_msg) {
                    Ok(_) => (),
                    Err(_try_send_error) => {
                        warn!(
                            "Sending error, client likely disconnected. {}",
                            client_address
                        );
                    }
                }
            }
            axum::extract::ws::Message::Pong(_msg) => {
                // info!("{} Received a pong frame", &client_address);
            }
            axum::extract::ws::Message::Text(msg_str) => {
                // info!("{} Received a text frame", &client_address);
                match subscription_type {
                    SubscriptionType::Subscription => match market_data_type {
                        MarketDataType::CbboV1 => {
                            cbbo_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_uri.clone(),
                                tx_task_tx.clone(),
                                *market_data_type,
                                Some(username.clone()),
                                market_data_id.clone(),
                                false,
                                websocketlimit_route,
                            );
                        }
                        MarketDataType::MarketDepthV1 => {
                            market_depth_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_depth_uri.clone(),
                                tx_task_tx.clone(),
                                *market_data_type,
                                username.clone().as_str(),
                                market_data_id.clone(),
                                websocketlimit_route,
                            );
                        }
                        MarketDataType::RestCostCalculatorV1 => {
                            info!("recieved websocket connection for otc cost calculator");
                            rest_cost_calculator_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_depth_uri.clone(),
                                tx_task_tx.clone(),
                                *market_data_type,
                                username.clone().as_str(),
                                market_data_id.clone(),
                                websocketlimit_route,
                            );
                        }
                        MarketDataType::Direct => {
                            info!("{} Received a message, ignoring", &client_address);
                        }
                    },
                    SubscriptionType::PublicSubscription => match market_data_type {
                        MarketDataType::CbboV1 => {
                            cbbo_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_uri.clone(),
                                tx_task_tx.clone(),
                                *market_data_type,
                                None,
                                market_data_id.clone(),
                                true,
                                websocketlimit_route,
                            );
                        }

                        MarketDataType::MarketDepthV1 => {
                            market_depth_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_depth_uri.clone(),
                                tx_task_tx.clone(),
                                *market_data_type,
                                username.clone().as_str(),
                                market_data_id.clone(),
                                websocketlimit_route,
                            );
                        }
                        _ => {
                            info!("{} Received an unexpected text frame", &client_address);
                        }
                    },
                    _ => {
                        info!("{} Received an unexpected text frame", &client_address);
                    }
                }
            }
            axum::extract::ws::Message::Binary(_msg) => {
                info!("{} Received a binary frame", &client_address);
            }
        }

        future::ok(())
        // futures_util::future::ready(())
        // Ok(())
    });

    let outgoing_clone = Arc::clone(&outgoing);
    let receive_from_others = async {
        while let Some(msg) = rx.recv().await {
            match outgoing_clone.lock().await.send(msg).await {
                Ok(_) => {}
                Err(_try_send_error) => {
                    warn!("Sending error, client likely disconnected.");
                    break;
                }
            }
        }
    };

    pin_mut!(
        check_subscription_still_active,
        receive_from_others,
        recv_task
    );
    // future::select(recv_task, receive_from_others).await;
    tokio::select! {
        _ = recv_task => {},
        _ = receive_from_others => {
            info!("receive_from_others ended");
        },
        _ = check_subscription_still_active => {
            //info!("check_subscription_still_active ended");
        },
    }

    // info!("{} {} disconnected", &client_address, username);

    connection_state.remove_client_from_state(
        &client_address,
        websocketlimit_route,
        username.clone(),
    );

    // returning from the handler closes the websocket connection
    info!(
        "Websocket context {} {} destroyed",
        client_address, username
    );
}

fn exceeds_max_session_time(connection_time: Instant) -> bool {
    (Instant::now() - connection_time) > MAX_SESSION_TIME
}

pub async fn get_state(
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    match check_token_and_authenticate(
        &headers,
        &Query(params),
        &uris.auth_uri,
        connection_state.clone(),
    )
    .await
    {
        Ok(_) => (),
        Err(_) => {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::from("Unauthorized".to_string()))
                .unwrap()
        }
    }

    let json_string = connection_state.to_json();
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json_string))
        .unwrap()
}

pub async fn authenticate_user(
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    match check_token_and_authenticate(
        &headers,
        &Query(params),
        &uris.auth_uri,
        connection_state.clone(),
    )
    .await
    {
        Ok(_) => Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Authorized".to_string()))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("Unauthorized".to_string()))
            .unwrap(),
    }
}

pub async fn get_cached_response(
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    // OriginalUri(original_uri): OriginalUri,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    req: Request<Body>,
) -> impl IntoResponse {
    match check_token_and_authenticate(
        &headers,
        &Query(params),
        &uris.auth_uri,
        connection_state.clone(),
    )
    .await
    {
        Ok(_) => {
            let endpoint = req.uri().path().trim_end_matches('/');
            match connection_state.get_cached_response(endpoint) {
                Ok(response) => Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(response))
                    .unwrap(),
                Err(err) => {
                    error!(
                        "Error getting cached response for endpoint {}: {}",
                        endpoint, err
                    );
                    Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .body(Body::from("Awaiting data, please try later".to_string()))
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

pub async fn get_cached_ohlc_response(
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if !params.contains_key("product") {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("product is required".to_string()))
            .unwrap();
    }

    let product = params.get("product").expect("product is required");
    if let Err(err) = connection_state.is_pair_valid(product) {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(err.to_json_str()))
            .unwrap();
    }

    let chart = connection_state.get_ohlc_chart(product.as_str());

    return match chart {
        Some(chart) => Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(chart))
            .unwrap(),
        None => {
            connection_state.subscribe_ohlc_chart(
                product.clone(),
                Arc::clone(&connection_state),
                uris.chart_uri.clone(),
            );
            let chart = fetch_chart(
                product.as_str(),
                uris.chart_uri.as_str(),
                &connection_state.chart_exchanges,
            )
            .await;
            return match chart {
                Some(chart) => Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(chart))
                    .unwrap(),
                None => Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::from("Awaiting data, please try later".to_string()))
                    .unwrap(),
            };
        }
    };
}

pub async fn volume(
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    // OriginalUri(original_uri): OriginalUri,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    match check_token_and_authenticate(
        &headers,
        &Query(params.clone()),
        &uris.auth_uri,
        connection_state.clone(),
    )
    .await
    {
        Ok(_) => {
            let uri = uris.trades_uri.clone();
            let endpoint = "volume".to_string();
            let mut url =
                url::Url::parse(&format!("http://{uri}/{endpoint}?")).expect("Invalid base URL");

            for (key, value) in &params {
                url.query_pairs_mut().append_pair(key, value);
            }

            let client = reqwest::Client::new();
            let res = client.get(url).send().await;
            if let Ok(response) = res {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(response.text().await.unwrap()))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::from("Awaiting data, please try later".to_string()))
                    .unwrap()
            }
        }
        Err(_) => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("Unauthorized".to_string()))
            .unwrap(),
    }
}

pub async fn get_chart(
    State(connection_state): State<ConnectionState>,
    Extension(uris): Extension<URIs>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    match check_token_and_authenticate(
        &headers,
        &Query(params.clone()),
        &uris.auth_uri,
        connection_state.clone(),
    )
    .await
    {
        Ok(_) => {
            // Validate required keys
            let required_keys = vec![
                "interval",
                "product",
                "start_time",
                "end_time",
                "size",
                "exchanges",
            ];

            for key in &required_keys {
                if !params.contains_key(*key) {
                    return Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::from(format!("{} is required", key)))
                        .unwrap();
                }
            }

            let endpoint = "ohlc".to_string(); // assuming "ohlc" is the endpoint being used
            let base_url = format!("http://{}", uris.chart_uri.clone()); // assuming this is the base URL

            let mut url = url::Url::parse(&format!("{}/{}", base_url, endpoint)).unwrap();

            for (key, value) in &params {
                url.query_pairs_mut().append_pair(key, value);
            }

            let client = reqwest::Client::new();
            let res = client.get(url).send().await;

            if let Ok(response) = res {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(response.text().await.unwrap()))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::from("Awaiting data, please try later".to_string()))
                    .unwrap()
            }
        }
        Err(_) => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("Unauthorized".to_string()))
            .unwrap(),
    }
}
