use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures_util::{pin_mut, StreamExt};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

// for axum
use axum::{body::Body, extract::ws::Message as axum_Message, http::Uri};
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::{OriginalUri, Path, State},
    http::{Response, StatusCode},
    response::IntoResponse,
    TypedHeader,
};

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
use axum::extract::Extension;
use reqwest::Client;
use tracing::{error, info, warn};

pub type Tx = UnboundedSender<axum::extract::ws::Message>;
pub type ConnectionState = Arc<RwLock<HashMap<String, HashMap<SocketAddr, Tx>>>>;

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
                let mut locked_state = connection_state.write().unwrap();
                let listener_hash_map = locked_state.get(&ws_endpoint);
                let active_listeners = match listener_hash_map {
                    Some(listeners) => listeners.iter().map(|(_, ws_sink)| ws_sink),
                    None => {
                        info!("Subscription {} no longer required", &ws_endpoint);
                        closing_down = true;
                        break;
                    }
                };
                warn!("Disconnecting clients {}", active_listeners.len());
                for recp in active_listeners {
                    let ms = axum_Message::Close(Some(CloseFrame {
                        code: 1001,
                        reason: "Requested endpoint not found".into(),
                    }));
                    println!("Sending close frame");
                    match recp.unbounded_send(ms) {
                        Ok(_) => (),
                        Err(_try_send_error) => {
                            warn!("Sending error, client likely disconnected.");
                        }
                    }
                }
                //remove the subcription from the connection state
                locked_state.remove(&ws_endpoint);
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
                        // let data = message_text.clone().into_data();
                        // let data = msg.clone().into_data();
                        // tokio::io::stdout().write_all(&data).await.unwrap();

                        let number_of_active_listeners: usize;
                        // this is a read lock only
                        {
                            let locked_state = connection_state.read().unwrap();
                            let listener_hash_map = locked_state.get(&ws_endpoint);
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
                            let listener_hash_map = locked_state.get(&ws_endpoint);
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
                                locked_state.remove(&ws_endpoint);
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
    "Proxy is running"
}

#[axum_macros::debug_handler]
pub async fn axum_ws_handler(
    Path(_symbol): Path<String>,
    // Query(params): Query<HashMap<String, String>>,
    OriginalUri(original_uri): OriginalUri,
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<ConnectionState>,
    Extension(cbag_uri): Extension<String>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    info!("{addr} connected User-Agent {user_agent}. Requesting subscription: {original_uri}");
    // we can customize the callback by sending additional info such as original_uri.
    ws.on_upgrade(move |socket| axum_handle_socket(socket, addr, original_uri, state, cbag_uri))
}

/// Actual websocket statemachine (one will be spawned per connection)
// #[axum_macros::debug_handler]
async fn axum_handle_socket(
    websocket: WebSocket,
    client_address: SocketAddr,
    request_endpoint: Uri,
    connection_state: ConnectionState,
    cbag_uri: String,
) {
    // added by karun
    let (tx, rx): (
        UnboundedSender<axum_Message>,
        UnboundedReceiver<axum_Message>,
    ) = unbounded();
    // let (tx, rx) = unbounded();

    let request_endpoint_str = request_endpoint.to_string();
    let (outgoing, mut incoming) = websocket.split();

    // add the client to the connection state. If the url isn't already subscribed
    // to, we need to spawn a process to subscribe to the url. This has to be done
    // whilst there is a write lock on the connection state in case multiple
    // clients requesting the same url connect at the same time to avoid
    // race conditions
    {
        let mut connection_state_locked = connection_state.write().unwrap();
        // check if the endpoint is already subscribed to, if not
        // start a process to subscribe to the endpoint
        let already_subscribed = if !connection_state_locked.contains_key(&request_endpoint_str) {
            connection_state_locked.insert(request_endpoint_str.clone(), HashMap::new());
            false
        } else {
            true
        };
        // Access the inner HashMap and add a value to it
        if let Some(ws_endpoint_clients) = connection_state_locked.get_mut(&request_endpoint_str) {
            ws_endpoint_clients.insert(client_address, tx);
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
            );
        }
    }

    let endpoint_clone = request_endpoint_str.clone();
    let connection_state_clone = connection_state.clone();
    let client_address_clone = client_address;
    let check_subscription_still_active = tokio::spawn(async move {
        loop {
            sleep(Duration::from_millis(1000)).await;
            {
                let connection_state_locked = connection_state_clone.read().unwrap();
                // Check if the outer HashMap contains the key
                if let Some(subscribed_clients) = connection_state_locked.get(&endpoint_clone) {
                    // Check if the inner HashMap contains the key
                    if !subscribed_clients.contains_key(&client_address_clone) {
                        info!(
                            "Client {} Disconnected. ending check task",
                            &client_address_clone
                        );
                        return;
                    }
                } else {
                    warn!(
                        "Subcription {} not longer active, disconnecting client ws",
                        &endpoint_clone
                    );
                    return;
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

    let recv_task = tokio::spawn(async move {
        // used to disconnect the the websocket when a close frame is
        // recived
        while let Some(Ok(msg)) = incoming.next().await {
            if let axum::extract::ws::Message::Close(_msg) = msg {
                info!("{} Received a close frame", &client_address);
                return;
            }
        }
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(check_subscription_still_active, receive_from_others);
    // future::select(recv_task, receive_from_others).await;
    tokio::select! {
        _ = recv_task => {},
        _ = receive_from_others => {},
        _ = check_subscription_still_active => {},
    }

    info!("{} disconnected", &client_address);

    // remove from the client from the connection state
    {
        let mut connection_state_locked = connection_state.write().unwrap();
        if let Some(ws_endpoint_clients) = connection_state_locked.get_mut(&request_endpoint_str) {
            ws_endpoint_clients.remove(&client_address);
            info!(
                "{} There are {} clients remaining",
                &request_endpoint_str,
                ws_endpoint_clients.len()
            );
        }
    }
    // returning from the handler closes the websocket connection
    info!("Websocket context {} destroyed", client_address);
}
