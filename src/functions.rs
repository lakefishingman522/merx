use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
    thread,
};

use futures::future;
use futures::stream::TryStreamExt;
use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures_util::{pin_mut, StreamExt};
// Import the TryStreamExt trait
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

// for axum
use axum::{body::Body, extract::ws::Message as axum_Message, http::Uri};
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    TypedHeader,
};

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
use axum::extract::Extension;
use reqwest::Client;
use tracing::{error, info, warn};

// use crate::routes_config::{ROUTES, SUB_TYPE};

use crate::md_handlers::{cbbo_v1, market_depth_v1};
use crate::routes_config::{MarketDataType, SubscriptionType, ROUTES, SUB_TYPE};
use crate::{
    auth::authenticate_token, md_handlers::rest_cost_calculator_v1::RestCostCalculatorV1RequestBody,
};

pub type Tx = UnboundedSender<axum::extract::ws::Message>;
// pub type ConnectionState = Arc<RwLock<HashMap<String, HashMap<SocketAddr, Tx>>>>;
// pub type SubscriptionCount = Arc<RwLock<HashMap<SocketAddr, u32>>>;

pub type SubscriptionState = HashMap<String, HashMap<SocketAddr, Tx>>;
pub type SubscriptionCount = HashMap<SocketAddr, u32>;

#[derive(Default)]
pub struct ConnectionStateStruct {
    subscription_state: SubscriptionState,
    subscription_count: SubscriptionCount,
}

impl ConnectionStateStruct {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Clone)]
pub struct URIs {
    pub cbag_uri: String,
    pub auth_uri: String,
}

pub type ConnectionState = Arc<RwLock<ConnectionStateStruct>>;

// helper function for conversions from tungstenite message to axum message
fn from_tungstenite(message: Message) -> Option<axum::extract::ws::Message> {
    match message {
        Message::Text(text) => Some(axum_Message::Text(text)),
        Message::Binary(binary) => Some(axum_Message::Binary(binary)),
        Message::Ping(ping) => Some(axum_Message::Ping(ping)),
        Message::Pong(pong) => Some(axum_Message::Pong(pong)),
        Message::Close(Some(close)) => Some(axum_Message::Close(Some(CloseFrame {
            code: close.code.into(),
            reason: close.reason,
        }))),
        Message::Close(None) => Some(axum_Message::Close(None)),
        // we can ignore `Frame` frames as recommended by the tungstenite maintainers
        // https://github.com/snapview/tungstenite-rs/issues/268
        Message::Frame(_) => None,
    }
}

#[allow(unused_assignments)]
pub fn subscribe_to_market_data(
    ws_endpoint: &str,
    connection_state: ConnectionState,
    cbag_uri: String,
    market_data_type: MarketDataType,
) -> JoinHandle<()> {
    info!("Starting a new subscription to {}", ws_endpoint);
    //TODO: add this as a parameter
    let ws_endpoint = ws_endpoint.to_owned();
    let full_url = format!("ws://{}{}", cbag_uri, ws_endpoint);

    tokio::spawn(async move {
        info!("Attempting to connect to {}", &ws_endpoint);
        let url = url::Url::parse(&full_url).unwrap();
        let mut closing_down = false;
        let mut consecutive_errors = 0;
        // TODO: after x number of timeouts, should check if clients are still connected
        loop {
            if consecutive_errors >= 5 {
                // disconnect from all sockets
                warn!(
                    "Unable to connect, closing down subscription {}",
                    &ws_endpoint
                );
                let mut locked_state = connection_state.write().unwrap(); //TODO: maybe only need a read lock when sending out the messages
                let listener_hash_map = locked_state.subscription_state.get(&ws_endpoint);
                let active_listeners = match listener_hash_map {
                    Some(listeners) => listeners.iter().map(|(_, ws_sink)| ws_sink),
                    None => {
                        info!("Subscription {} no longer required", &ws_endpoint);
                        closing_down = true;
                        break;
                    }
                };
                // warn!("Disconnecting clients {}", active_listeners.len());
                //TODO: send something about the subscription here if it wasn't valid
                for recp in active_listeners {
                    // send a message that the subscription was not valid

                    // let ms = axum_Message::Close(Some(CloseFrame {
                    //     code: 1001,
                    //     reason: "Requested endpoint not found".into(),
                    // }));
                    // info!("Sending close frame");

                    // match recp.unbounded_send(ms) {
                    //     Ok(_) => (),
                    //     Err(_try_send_error) => {
                    //         warn!("Sending error, client likely disconnected.");
                    //     }
                    // }
                }

                // clean up subscription counts for those clients who were connected
                // for this subscription
                let client_subscriptions = locked_state
                    .subscription_state
                    .get(&ws_endpoint)
                    .unwrap()
                    .clone();
                for (client_address, _) in client_subscriptions.iter() {
                    if let Some(count) = locked_state.subscription_count.get_mut(client_address) {
                        *count -= 1;
                        if *count == 0 {
                            locked_state.subscription_count.remove(client_address);
                            error!(
                                "Removing the subscription count for {} as it is 0",
                                client_address
                            );
                        }
                    }
                }

                //remove the subscription from the connection state
                locked_state.subscription_state.remove(&ws_endpoint);
                info!(
                    "Number of subscriptions left: {}",
                    locked_state.subscription_state.len()
                );
                break;
            }
            let result = match timeout(Duration::from_secs(3), connect_async(url.clone())).await {
                Ok(response) => response,
                Err(err) => {
                    // Error occurred or connection timed out
                    error!("Timeout Error: {:?}", err);
                    consecutive_errors += 1;
                    continue;
                }
            };

            let (ws_stream, _) = match result {
                Ok(whatever) => whatever,
                Err(err) => {
                    error!("Connection Error connecting to {}: {:?}", &ws_endpoint, err);
                    consecutive_errors += 1;
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };

            info!(
                "WebSocket handshake for {} has been successfully completed",
                &ws_endpoint
            );
            let (_write, mut read) = ws_stream.split();
            consecutive_errors = 0;
            loop {
                let message = read.next().await;
                let msg = match message {
                    Some(msg) => msg,
                    None => {
                        error!(
                            "Unable to read message, restarting subscription {}",
                            ws_endpoint
                        );
                        break;
                    }
                };
                match msg {
                    Ok(message_text) => {
                        // transform the raw message in accordance with its market data type
                        let message_transform_result: Result<Message, String> =
                            match market_data_type {
                                MarketDataType::CbboV1 => cbbo_v1::transform_message(message_text),
                                MarketDataType::MarketDepthV1 => {
                                    market_depth_v1::transform_message(message_text)
                                }
                                MarketDataType::Direct => Ok(message_text),
                                MarketDataType::RestCostCalculatorV1 => {
                                    Err("Unexpected Market Data Type".into())
                                }
                            };

                        let message_text = match message_transform_result {
                            Ok(message) => message,
                            Err(err) => {
                                error!("Error transforming message: {}", err);
                                continue;
                            }
                        };

                        // let data = message_text.clone().into_data();
                        // let data = msg.clone().into_data();
                        // tokio::io::stdout().write_all(&data).await.unwrap();

                        let number_of_active_listeners: usize;
                        // this is a read lock only
                        {
                            let locked_state = connection_state.read().unwrap();
                            let listener_hash_map =
                                locked_state.subscription_state.get(&ws_endpoint);
                            let active_listeners = match listener_hash_map {
                                Some(listeners) => listeners.iter().map(|(_, ws_sink)| ws_sink),
                                None => {
                                    info!("subscription no longer required");
                                    return;
                                    //todo quite this stream, no longer require
                                }
                            };
                            // when there are no more clients subscribed to this ws subscription,
                            // we can close in a write lock to avoid race conditions agains
                            // new clients subscriptions that might come in whilst the
                            // subscription is being removed from the connection state
                            number_of_active_listeners = active_listeners.len();
                            for recp in active_listeners {
                                let ms = from_tungstenite(message_text.clone()).unwrap();
                                match recp.unbounded_send(ms) {
                                    Ok(_) => (),
                                    Err(_try_send_error) => {
                                        warn!("Sending error, client likely disconnected.");
                                    }
                                }
                            }
                        }

                        if number_of_active_listeners == 0 {
                            let mut locked_state = connection_state.write().unwrap();
                            // check again there are no new clients for the subsctiption
                            // client_subscriptions = connection_state_lock.
                            let listener_hash_map =
                                locked_state.subscription_state.get(&ws_endpoint);
                            let active_listeners = match listener_hash_map {
                                Some(listeners) => listeners,
                                None => {
                                    info!("Subscription {} no longer required", &ws_endpoint);
                                    closing_down = true;
                                    break;
                                }
                            };
                            if active_listeners.is_empty() {
                                info!(
                                    "Removing subscription {} from subscriptions \
                            no more listeners",
                                    &ws_endpoint
                                );
                                locked_state.subscription_state.remove(&ws_endpoint);
                                closing_down = true;
                                break;
                            }
                        }
                    }
                    Err(err) => {
                        error!(
                            "Error parsing message on subscription {} Error {}",
                            &ws_endpoint, err
                        )
                    }
                }
            }
            if closing_down {
                break;
            }
        }
        info!("Subscription task for {} exiting", ws_endpoint);
    })
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
    info!("Auth Uri: {}", uris.auth_uri);

    let base_route = req.uri().path();

    //log the base route
    info!("base route {}", base_route);

    if !ROUTES.contains_key(&base_route) {
        warn!("Endpoint not available {}", base_route);
        return (StatusCode::BAD_REQUEST, "Endpoint not available").into_response();
        //TODO
    }
    let route = ROUTES.get(&base_route.to_string()).unwrap();

    if !SUB_TYPE.contains_key(&base_route) {
        warn!("Endpoint not available {}", base_route);
        return (StatusCode::BAD_REQUEST, "Endpoint not available").into_response();
        //TODO
    }
    let subscription_type = SUB_TYPE.get(&base_route.to_string()).unwrap();

    // if this isn't a direct subscription, then we have to authenticate
    if !matches!(subscription_type, SubscriptionType::DIRECT) {
        let token = if let Some(token) = headers.get("Authorization") {
            let token = if let Ok(tok) = token.to_str() {
                tok
            } else {
                warn!("Unable to parse token");
                return (StatusCode::BAD_REQUEST, "Unable to parse token").into_response();
            };
            token
            // info!("Authorization: {}", token);
        } else if let Some(token) = params.get("token") {
            info!("Token: {}", token);
            token
        } else {
            return (StatusCode::UNAUTHORIZED, "No token provided").into_response();
        };
        match authenticate_token(&uris.auth_uri, token).await {
            Ok(_) => {
                // authenticated ok
            }
            Err(_) => {
                warn!("Unable to authenticate token");
                return (StatusCode::UNAUTHORIZED, "Unable to authenticate token").into_response();
            }
        }
    }

    // respond with a 404 error
    // if original_uri != "/ws/snapshot/subscription" {
    //     return (StatusCode::BAD_REQUEST, "Placeholder Text").into_response(); //TODO
    // }

    // we can customize the callback by sending additional info such as original_uri.
    ws.on_upgrade(move |socket| {
        axum_handle_socket(
            socket,
            addr,
            original_uri,
            connection_state,
            uris.cbag_uri,
            route,
            subscription_type,
        )
    })
}

pub fn add_client_to_subscription(
    connection_state: &ConnectionState,
    client_address: &SocketAddr,
    request_endpoint_str: &String,
    cbag_uri: String,
    tx: UnboundedSender<axum::extract::ws::Message>,
    market_data_type: MarketDataType,
) {
    {
        let mut connection_state_locked = connection_state.write().unwrap();
        // check if the endpoint is already subscribed to, if not
        // start a process to subscribe to the endpoint
        let already_subscribed = if !connection_state_locked
            .subscription_state
            .contains_key(request_endpoint_str)
        {
            connection_state_locked
                .subscription_state
                .insert(request_endpoint_str.clone(), HashMap::new());
            false
        } else {
            true
        };
        let subscription_count = connection_state_locked
            .subscription_count
            .entry(*client_address)
            .or_insert(0);
        *subscription_count += 1;
        info!(
            "Subscription {} added. Total Subs: {}",
            &request_endpoint_str, *subscription_count
        );
        // Access the inner HashMap and add a value to it
        if let Some(ws_endpoint_clients) = connection_state_locked
            .subscription_state
            .get_mut(request_endpoint_str)
        {
            ws_endpoint_clients.insert(*client_address, tx);
            //insert into subscription count and add 1 to the count
            info!(
                "Subscription {} added. Total Subs: {}",
                &request_endpoint_str,
                ws_endpoint_clients.len()
            );
            if already_subscribed {
                info!(
                    "Subscription {} already exists, adding client to subscription. Total Subs: {}",
                    &request_endpoint_str,
                    ws_endpoint_clients.len()
                );
            }
        } else {
            panic!("Expected key in connection state not found")
        }
        if !already_subscribed {
            let _handle = subscribe_to_market_data(
                &request_endpoint_str,
                Arc::clone(&connection_state),
                cbag_uri,
                market_data_type,
            );
        }
    }
}

/// Actual websocket state machine (one will be spawned per connection)
// #[axum_macros::debug_handler]
async fn axum_handle_socket(
    websocket: WebSocket,
    client_address: SocketAddr,
    request_endpoint: Uri,
    connection_state: ConnectionState,
    cbag_uri: String,
    market_data_type: &MarketDataType,
    subscription_type: &SubscriptionType,
) {
    // added by karun
    let (tx, rx): (
        UnboundedSender<axum_Message>,
        UnboundedReceiver<axum_Message>,
    ) = unbounded();
    // let (tx, rx) = unbounded();

    let thread_id = thread::current().id();
    println!("This code is running on thread {:?}", thread_id);

    let request_endpoint_str = request_endpoint.to_string();
    let recv_task_tx = tx.clone();
    let recv_task_cbag_uri = cbag_uri.clone();
    let (outgoing, incoming) = websocket.split();

    // add the client to the connection state. If the url isn't already subscribed
    // to, we need to spawn a process to subscribe to the url. This has to be done
    // whilst there is a write lock on the connection state in case multiple
    // clients requesting the same url connect at the same time to avoid
    // race conditions

    // if its a direct subscription we can add it straight away
    // else we will wait for the client to send us subscription messages
    if matches!(subscription_type, SubscriptionType::DIRECT) {
        add_client_to_subscription(
            &connection_state,
            &client_address,
            &request_endpoint_str,
            cbag_uri,
            tx,
            market_data_type.clone(),
        );
    }

    let connection_state_clone = connection_state.clone();
    let client_address_clone = client_address;
    let subscription_type_clone = subscription_type.clone();
    let check_subscription_still_active = tokio::spawn(async move {
        // save the time at this point
        let connection_time = tokio::time::Instant::now();

        loop {
            sleep(Duration::from_millis(1000)).await;
            {
                if matches!(subscription_type_clone, SubscriptionType::SUBSCRIPTION)
                    && (tokio::time::Instant::now() - connection_time < Duration::from_secs(20))
                {
                    continue;
                }

                let connection_state_locked = connection_state_clone.read().unwrap();
                // Check if the outer HashMap contains the key

                // check if client address is in subscription count or if the subscription count is 0 for the client address
                if !connection_state_locked
                    .subscription_count
                    .contains_key(&client_address_clone)
                    || connection_state_locked
                        .subscription_count
                        .get(&client_address_clone)
                        .unwrap()
                        == &0
                {
                    info!(
                        "Client {} Disconnected or subscription no longer active. ending check task",
                        &client_address_clone
                    );
                    info!("returning");
                    return;
                }

                // if let Some(subscribed_clients) = connection_state_locked.subscription_state.get(&endpoint_clone) {
                //     // Check if the inner HashMap contains the key
                //     if !subscribed_clients.contains_key(&client_address_clone) {
                //         info!(
                //             "Client {} Disconnected. ending check task",
                //             &client_address_clone
                //         );
                //         return;
                //     }
                // } else {
                //     warn!(
                //         "Subcription {} not longer active, disconnecting client ws",
                //         &endpoint_clone
                //     );
                //     return;
                // }
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
    let recv_task_client_address = client_address;
    // let recv_task_tx = tx.clone();

    // this is unused but good to log incase there are incoming messages
    let recv_task = incoming.try_for_each(|msg| {

        // log message if we are unable to convert it to text, else long an error
        // if we are unable to convert it to text
        // let msg_text = match msg.to_text() {
        //     Ok(msg_text) => {
        //         info!(
        //             "Received a message from {}: {}",
        //             client_address,
        //             msg_text
        //         );
        //     },
        //     Err(_err) => {
        //         error!("Unable to convert message to text from {}", client_address);
        //         // return future::ok(());
        //     }
        // };

        match msg {
            axum::extract::ws::Message::Close(_msg) => {
                info!("{} Received a close frame", &client_address);
                // return future::ok(())
            }
            axum::extract::ws::Message::Ping(_msg) => {
                // info!("{} Received a ping frame", &client_address);
                // send back a pong
                let pong_msg = axum_Message::Pong(_msg);
                match recv_task_tx.clone().unbounded_send(pong_msg) {
                    Ok(_) => (),
                    Err(_try_send_error) => {
                        warn!("Sending error, client likely disconnected. {}",
                        client_address);
                    }
                }
            }
            axum::extract::ws::Message::Pong(_msg) => {
                // info!("{} Received a pong frame", &client_address);
            }
            axum::extract::ws::Message::Text(msg_str) => {
                info!("{} Received a text frame", &client_address);
                if matches!(subscription_type, SubscriptionType::SUBSCRIPTION) {
                    match market_data_type {
                        MarketDataType::CbboV1 => {
                            cbbo_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_uri.clone(),
                                recv_task_tx.clone(),
                                market_data_type.clone(),
                            );
                        }
                        MarketDataType::MarketDepthV1 => {
                            market_depth_v1::handle_subscription(
                                &client_address,
                                &recv_task_connection_state,
                                msg_str,
                                recv_task_cbag_uri.clone(),
                                recv_task_tx.clone(),
                                market_data_type.clone(),
                            );
                        }
                        RestCostCalculatorV1RequestBody => {
                            error!("unexpected Market Data Type");
                        }
                        MarketDataType::Direct => {
                            info!("{} Received a message, ignoring", &client_address);
                        }
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

    //write a future to be able to read and print websocket messages from incoming without spawning
    // a new task

    // let recv_task = tokio::spawn(async move {
    //     // used to disconnect the the websocket when a close frame is
    //     // received of if the client is no longer subscribed

    //     let mut check_state_interval = interval(Duration::from_secs(5));
    //     loop {
    //         tokio::select! {
    //             // Use async/await to work with the future returned by incoming.next()
    //             Some(msg_result) = incoming.next() => {
    //                 // Handle WebSocket messages
    //                 let msg = msg_result.unwrap();
    //                 match msg {
    //                     axum::extract::ws::Message::Close(_msg) => {
    //                         info!("{} Received a close frame", &client_address);
    //                         return;
    //                     }
    //                     axum::extract::ws::Message::Ping(_msg) => {
    //                         info!("{} Received a ping frame", &client_address);
    //                     }
    //                     axum::extract::ws::Message::Pong(_msg) => {
    //                         info!("{} Received a pong frame", &client_address);
    //                     }
    //                     axum::extract::ws::Message::Text(_msg) => {
    //                         info!("{} Received a text frame", &client_address);
    //                     }
    //                     axum::extract::ws::Message::Binary(_msg) => {
    //                         info!("{} Received a binary frame", &client_address);
    //                     }
    //                 }
    //             }
    //             _ = check_state_interval.tick() => {
    //                 // Perform your state check every 5 seconds
    //                 // This code block will be executed when the interval ticks
    //                 info!("Checking state...");
    //                 // Add your state-checking logic here
    //             }
    //         }
    //     }

    // //add a timeout to the incoming stream
    // while let Some(Ok(msg)) = incoming.next().await {

    //     // if let axum::extract::ws::Message::Close(_msg) = msg {
    //     //     info!("{} Received a close frame", &client_address);
    //     //     return;
    //     // } else if let axum::extract::ws::Message::Ping(_msg) = msg {
    //     //     info!("{} Received a ping frame", &client_address);
    //     //     continue;
    //     // } else if let axum::extract::ws::Message::Pong(_msg) = msg {
    //     //     info!("{} Received a pong frame", &client_address);
    //     //     continue;
    //     // } else if let axum::extract::ws::Message::Text(_msg) = msg {
    //     //     info!("{} Received a text frame", &client_address);
    //     //     continue;
    //     // } else if let axum::extract::ws::Message::Binary(_msg) = msg {
    //     //     info!("{} Received a binary frame", &client_address);
    //     //     continue;
    //     // } else {
    //     //     info!("{} Received an unknown frame", &client_address);
    //     //     continue;
    //     // }
    // }
    // });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(
        check_subscription_still_active,
        receive_from_others,
        recv_task
    );
    // future::select(recv_task, receive_from_others).await;
    tokio::select! {
        _ = recv_task => { info!("recv task ended");},
        _ = receive_from_others => {
            info!("receive_from_others ended");
        },
        _ = check_subscription_still_active => {info!("check_subscription_still_active ended");},
    }

    info!("{} disconnected", &client_address);

    //stop the recv task
    // recv_task.abort();

    // remove from the client from the connection state
    {
        let mut connection_state_locked = connection_state.write().unwrap();

        // TODO: this could probably be more efficient
        for (subscription, subscribed_clients) in
            connection_state_locked.subscription_state.iter_mut()
        {
            subscribed_clients.remove(&client_address);
            info!(
                "{} There are {} clients remaining",
                &subscription,
                subscribed_clients.len()
            );
        }
        // remove the client address from the subscription_count if it is in there
        if connection_state_locked
            .subscription_count
            .contains_key(&client_address)
        {
            connection_state_locked
                .subscription_count
                .remove(&client_address);
        }

        // if let Some(ws_endpoint_clients) = connection_state_locked.subscription_state.get_mut(&request_endpoint_str) {
        //     ws_endpoint_clients.remove(&client_address);
        //     info!(
        //         "{} There are {} clients remaining",
        //         &request_endpoint_str,
        //         ws_endpoint_clients.len()
        //     );
        // }
    }
    // returning from the handler closes the websocket connection
    info!("Websocket context {} destroyed", client_address);
}
