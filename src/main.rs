use core::num;
use std::{
    collections::HashMap,
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
};

use futures_util::{
    future, pin_mut,
    stream::{SplitStream, TryStreamExt},
    StreamExt,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
};

use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, WebSocketStream};
use tokio_tungstenite::{
    tungstenite::{client, connect},
    MaybeTlsStream,
};

use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

// for axum
use axum::{
    extract::ws::{rejection::ConnectionNotUpgradable, Message as axum_Message},
    http::Uri,
};
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::{self, OriginalUri, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router, Server, TypedHeader,
};

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use tracing_subscriber::{fmt::Subscriber, EnvFilter};

// use axum_extra::TypedHeader;
use serde::{Deserialize, Serialize};

type Tx = UnboundedSender<axum::extract::ws::Message>;
type WSClientMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

type ConnectionState = Arc<RwLock<HashMap<String, HashMap<SocketAddr, Tx>>>>;

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

fn subscribe_to_market_data(
    ws_endpoint: &str,
    connection_state: ConnectionState,
) -> JoinHandle<()> {
    println!("Starting a new subscription to {}", ws_endpoint);
    //TODO: add this as a parameter
    let ws_endpoint = ws_endpoint.to_owned();
    let full_url = format!(
        "{}{}",
        "ws://internal-prod-cbag-726087086.ap-northeast-1.elb.amazonaws.com:8080", ws_endpoint
    );

    tokio::spawn(async move {
        println!("running subscribe to market data");
        let url = url::Url::parse(&full_url).unwrap();

        let result = match timeout(Duration::from_secs(3), connect_async(url)).await {
            Ok(response) => response,
            Err(err) => {
                // Error occurred or connection timed out
                println!("Timeout Error: {:?}", err);
                return ();
            }
        };

        let (ws_stream, _) = match result {
            Ok(whatever) => whatever,
            Err(err) => {
                println!("Connection Error: {:?}", err);
                return ();
            }
        };

        // let (ws_stream, _) = result.unwrap();

        // let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
        println!("WebSocket handshake has been successfully completed");
        let (write, mut read) = ws_stream.split();
        // let message = read.next().await else {
        //     println!("issue on stream");
        // }
        loop {
            // read.for_each(|message| async {
            let message = read.next().await;
            let msg = message.unwrap();
            match msg {
                Ok(message_text) => {
                    let data = message_text.clone().into_data();
                    // let data = msg.clone().into_data();
                    // tokio::io::stdout().write_all(&data).await.unwrap();

                    let number_of_active_listeners: usize;
                    // this is a read lock only
                    {
                        let locked_state = connection_state.read().unwrap();

                        // let the_listeners = locked_state.get("hello").iter().map(|(_, ws_sink)| ws_sink);
                        // let the_listeners = locked_state.get("hello").and_then(iter().map(|(ws_sink, _)| ws_sink));
                        // let the_listeners = locked_state.get("hello").unwrap_or_default().iter().map(|(_, ws_sink)| ws_sink);

                        // let the_listeners = locked_state
                        //     .get("hello")
                        //     .and_then(|ws_sink| Some(ws_sink.iter().map(|(_, ws_sink)| ws_sink)))
                        //     .unwrap_or_else(|| std::iter::empty());
                        // println!("received data from {}\n", url_string);
                        let listener_hash_map = locked_state.get(&ws_endpoint);

                        let active_listeners = match listener_hash_map {
                            Some(listeners) => listeners.iter().map(|(_, ws_sink)| ws_sink),
                            None => {
                                println!("subsciption no longer required");
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
                            println!("sending to listerner");
                            let ms = from_tungstenite(message_text.clone()).unwrap();
                            match recp.unbounded_send(ms) {
                                Ok(_) => (),
                                Err(try_send_error) => {
                                    println!("Sending error");
                                }
                            }
                        }
                    }
                    println!("there are {}", number_of_active_listeners);
                    if number_of_active_listeners == 0 {
                        let mut locked_state = connection_state.write().unwrap();
                        // check again there are no new clients for the subsctiption
                        // client_subscriptions = connection_state_lock.
                        let listener_hash_map = locked_state.get(&ws_endpoint);
                        let active_listeners = match listener_hash_map {
                            Some(listeners) => {
                                listeners
                            },
                            None => {
                                println!("subsciption no longer required");
                                return;
                            }
                        };
                        if active_listeners.len() == 0{
                            println!("Removing subscription {} from subscriptions", &ws_endpoint);
                            locked_state.remove(&ws_endpoint);
                            break
                        }
                    }
                }
                Err(err) => {}
            }
            // // let data = msg.clone().into_data();
            // // tokio::io::stdout().write_all(&data).await.unwrap();

            // // this is a read lock only
            // let locked_state = connection_state.read().unwrap();

            // // let the_listeners = locked_state.get("hello").iter().map(|(_, ws_sink)| ws_sink);
            // // let the_listeners = locked_state.get("hello").and_then(iter().map(|(ws_sink, _)| ws_sink));
            // // let the_listeners = locked_state.get("hello").unwrap_or_default().iter().map(|(_, ws_sink)| ws_sink);

            // // let the_listeners = locked_state
            // //     .get("hello")
            // //     .and_then(|ws_sink| Some(ws_sink.iter().map(|(_, ws_sink)| ws_sink)))
            // //     .unwrap_or_else(|| std::iter::empty());
            // // println!("received data from {}\n", url_string);
            // let listener_hash_map = locked_state.get(url_string);
            // let active_listeners = match listener_hash_map {
            //     Some(listeners) => listeners.iter().map(|(_, ws_sink)| ws_sink),
            //     None => {
            //         // println!("subsciption no longer required");
            //         return
            //         //todo quite this stream, no longer require
            //     }
            // };

            // for recp in active_listeners {
            //     println!("sending to listerner");
            //     let ms = from_tungstenite(msg.clone()).unwrap();
            //     recp.unbounded_send(ms).unwrap();
            // }
            // })
            // .await;
        }
        println!("Subscription task for {} exiting", ws_endpoint);
    })
}

//testing lifetimes
async fn process_messages(
    read: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    peer_map: WSClientMap,
) {
    read.for_each(|message| async {
        let msg = message.unwrap();
        let data = msg.clone().into_data();
        // tokio::io::stdout().write_all(&data).await.unwrap();

        let locked_state = peer_map.lock().unwrap();

        let the_listeners = locked_state.iter().map(|(_, ws_sink)| ws_sink);
        for recp in the_listeners {
            println!("sending to listener");
            let ms = from_tungstenite(msg.clone()).unwrap();
            recp.unbounded_send(ms).unwrap();
        }
    })
    .await;
}

#[tokio::main]
async fn main() {
    // to hold all the ws connections
    println!("Running main..");
    let state = WSClientMap::new(Mutex::new(HashMap::new()));
    // let connection_state = ConnectionState::new(RwLock::new(HashMap::new()));
    let connection_state = ConnectionState::new(RwLock::new(HashMap::new()));

    // let connect_addr =
    //     env::args().nth(1).unwrap_or_else(|| panic!("this program requires at least one argument"));
    // let url = url::Url::parse(&connect_addr).unwrap();

    let url = url::Url::parse("ws://internal-prod-cbag-726087086.ap-northeast-1.elb.amazonaws.com:8080/ws/bookstats/BTC-USDT.PERP?markets=BINANCE,BINANCEFUTURES,DYDX,COINBASE&depth_limit=10&iterval_ms=1000").unwrap();

    let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded();
    tokio::spawn(read_stdin(stdin_tx));

    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    println!("WebSocket handshake has been successfully completed");

    let (write, read) = ws_stream.split();

    let stdin_to_ws = stdin_rx.map(Ok).forward(write);

    let ws_to_stdout = tokio::spawn(process_messages(read, state.clone()));

    {
        match connection_state.try_read() {
            Ok(n) => println!("GOT the read lock"),
            Err(_) => println!("NOT GOT the read lock"),
        };
    }

    subscribe_to_market_data("/ws/bookstats/BTC-USDT.PERP", Arc::clone(&connection_state));

    // let stdin_to_ws_task = tokio::spawn(async {
    //     pin_mut!(stdin_to_ws);
    //     stdin_to_ws.await
    // });
    // let ws_to_stdout_task = tokio::spawn(async {
    //     pin_mut!(ws_to_stdout);
    //     ws_to_stdout.await
    // });

    // pin_mut!(stdin_to_ws_task), ws_to_stdout);

    // axum stuff only
    // initialize tracing
    // tracing_subscriber::fmt::init();

    // Configure the tracing subscriber
    let filter = EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into());

    tracing_subscriber::fmt::Subscriber::builder()
        .with_env_filter(filter)
        .init();

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        // `POST /users` goes to `create_user`
        // .route("/users", post(create_user));
        .route("/ws", get(axum_ws_handler))
        .route("/ws/:subscription", get(axum_ws_handler))
        .route("/ws/bookstats/:subscription", get(axum_ws_handler))
        .with_state(connection_state.clone())
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // run our app with hyper, listening globally on port 3000
    // let listener = tokio::net::TcpListener::bind("0.0.0.0:9089").await.unwrap();
    // axum::serve(listener, app).await.unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 9089));
    tracing::debug!("listening on {}", addr);
    println!("starting websocket server");
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();

    // end of axum stuff

    // // Set up a server to listen for websocket connections
    // let addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:9080".to_string());
    // // let state = WSClientMap::new(Mutex::new(HashMap::new()));
    // // Create the event loop and TCP listener we'll accept connections on.
    // let try_socket = TcpListener::bind(&addr).await;
    // let listener = try_socket.expect("Failed to bind");
    // println!("Listening on: {}", addr);

    // // listen_for_connections(listener, state).await;
    // tokio::spawn(listen_for_connections(listener, state.clone()));

    // future::select(stdin_to_ws, ws_to_stdout).await;
    pin_mut!(stdin_to_ws, ws_to_stdout);
    future::select(stdin_to_ws, ws_to_stdout).await;

    // ws_to_stdout.await.unwrap();
}

#[axum_macros::debug_handler]
async fn axum_ws_handler(
    Path(subscription): Path<String>,
    // Query(params): Query<HashMap<String, String>>,
    OriginalUri(original_uri): OriginalUri,
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<ConnectionState>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    println!("`{user_agent}` at {addr} connected. requesting subscription: {subscription}");
    // println!("OriginalUri {original_uri}");
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    ws.on_upgrade(move |socket| axum_handle_socket(socket, addr, original_uri, state))
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Proxy is running"
}

/// Actual websocket statemachine (one will be spawned per connection)
// #[axum_macros::debug_handler]
async fn axum_handle_socket(
    mut websocket: WebSocket,
    client_address: SocketAddr,
    request_endpoint: Uri,
    connection_state: ConnectionState,
) {
    // added by karun
    let (tx, rx): (
        UnboundedSender<axum_Message>,
        UnboundedReceiver<axum_Message>,
    ) = unbounded();
    // let (tx, rx) = unbounded();

    let request_endpoint_str = request_endpoint.to_string();
    let (outgoing, incoming) = websocket.split();

    // add the client to the connection state. If the url isn't already subscribed
    // to, we need to spawn a process to subscribe to the url. This has to be done
    // whilst there is a write lock on the connection state in case multiple
    // clients requesting the same url connect at the same time to avoid
    // race conditions
    {
        let mut connection_state_locked = connection_state.write().unwrap();
        // check if the endpoint is already subscribed to, if not
        // start a process to subscribe to the endpoint
        let already_subscribed: bool;
        if !connection_state_locked.contains_key(&request_endpoint_str) {
            connection_state_locked.insert(request_endpoint_str.clone(), HashMap::new());
            already_subscribed = false;
        } else {
            already_subscribed = true;
        }
        // Access the inner HashMap and add a value to it
        if let Some(ws_endpoint_clients) = connection_state_locked.get_mut(&request_endpoint_str) {
            ws_endpoint_clients.insert(client_address, tx);
        } else {
            panic!("Expected key in connection state not found")
        }
        if !already_subscribed {
            let _handle =
                subscribe_to_market_data(&request_endpoint_str, Arc::clone(&connection_state));
            println!("Got the handle, all done!");
        }
    }

    // let peers = peer_map.lock().unwrap();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        println!(
            "Received a message from {}: {}",
            client_address,
            msg.to_text().unwrap()
        );

        // We want to broadcast the message to everyone except ourselves.
        // let broadcast_recipients =
        //     peers.iter().filter(|(peer_addr, _)| peer_addr != &&addr).map(|(_, ws_sink)| ws_sink);

        // for recp in broadcast_recipients {
        //     recp.unbounded_send(msg.clone()).unwrap();
        // }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    // let receive_from_others = rx
    //     .map(Ok)
    //     .forward(outgoing.into())
    //     .map(|result| {
    //         if let Err(e) = result {
    //             eprintln!("Error: {:?}", e);
    //         }
    //     });

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &client_address);

    // remove from the client from the connection state
    {
        let mut connection_state_locked = connection_state.write().unwrap();
        if let Some(ws_endpoint_clients) = connection_state_locked.get_mut(&request_endpoint_str) {
            ws_endpoint_clients.remove(&client_address);
            println!("There are {} clients remaining", ws_endpoint_clients.len());
        } else {
            panic!("Expected key in connection state not found")
        }
    }
    // returning from the handler closes the websocket connection
    println!("Websocket context {} destroyed", client_address);
}

// Our helper method which will read data from stdin and send it along the
// sender provided.
async fn read_stdin(tx: futures_channel::mpsc::UnboundedSender<Message>) {
    let mut stdin = tokio::io::stdin();
    loop {
        let mut buf = vec![0; 1024];
        let n = match stdin.read(&mut buf).await {
            Err(_) | Ok(0) => break,
            Ok(n) => n,
        };
        buf.truncate(n);
        tx.unbounded_send(Message::binary(buf)).unwrap();
    }
}
