use std::{
    collections::HashMap,
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use futures_util::{future, pin_mut, StreamExt, stream::{TryStreamExt, SplitStream}};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, WebSocketStream};
use tokio_tungstenite::MaybeTlsStream;

use futures_channel::mpsc::{UnboundedSender, unbounded, UnboundedReceiver};


// for axum
use axum::{Server,
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::State,
    routing::{get, post},
    http::StatusCode,
    response::IntoResponse,
    Json, Router, TypedHeader
};
use axum::extract::ws::Message as axum_Message;

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};

// use axum_extra::TypedHeader;
use serde::{Deserialize, Serialize};


type Tx = UnboundedSender<axum::extract::ws::Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;


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


 async fn subscribe_to_market_data(){

 }

//testing lifetimes
async fn process_messages(read: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>, peer_map: PeerMap){
    read.for_each(|message| async {
        let msg = message.unwrap();
        let data = msg.clone().into_data();
        tokio::io::stdout().write_all(&data).await.unwrap();

        let locked_state = peer_map.lock().unwrap();

        let the_listeners = locked_state.iter().map(|(_, ws_sink)| ws_sink);
        for recp in the_listeners {
            println!("sending to listerner");
            let ms = from_tungstenite(msg.clone()).unwrap();
            recp.unbounded_send(ms).unwrap();
        }
    }).await;
}


#[tokio::main]
async fn main() {

    // to hold all the ws connections
    let state = PeerMap::new(Mutex::new(HashMap::new()));

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


    // let ws_to_stdout = {
    //     read.for_each(|message| async {
    //         let msg = message.unwrap();
    //         let data = msg.clone().into_data();
    //         tokio::io::stdout().write_all(&data).await.unwrap();

    //         let locked_state = state.lock().unwrap();

    //         let the_listeners = locked_state.iter().map(|(_, ws_sink)| ws_sink);
    //         for recp in the_listeners {
    //             println!("sending to listerner");
    //             let ms = from_tungstenite(msg.clone()).unwrap();
    //             recp.unbounded_send(ms).unwrap();
    //         }
    //     })
    // };

    // pin_mut!(stdin_to_ws, ws_to_stdout);
    // pin_mut!(stdin_to_ws);


    let ws_to_stdout = tokio::spawn(process_messages(read, state.clone()));

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
    tracing_subscriber::fmt::init();

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        // `POST /users` goes to `create_user`
        // .route("/users", post(create_user));
        .route("/ws", get(axum_ws_handler))
        .with_state(state.clone());
        // logging so we can see whats going on
        // .layer(
        //     TraceLayer::new_for_http()
        //         .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        // );

    // run our app with hyper, listening globally on port 3000
    // let listener = tokio::net::TcpListener::bind("0.0.0.0:9089").await.unwrap();
    // axum::serve(listener, app).await.unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 9089));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();

    // end of axum stuff




    // // Set up a server to listen for websocket connections
    // let addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:9080".to_string());
    // // let state = PeerMap::new(Mutex::new(HashMap::new()));
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
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<PeerMap>
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    println!("`{user_agent}` at {addr} connected.");
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    ws.on_upgrade(move |socket| axum_handle_socket(socket, addr, state))
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}



/// Actual websocket statemachine (one will be spawned per connection)
// #[axum_macros::debug_handler]
async fn axum_handle_socket(mut websocket: WebSocket, who: SocketAddr, peer_map: PeerMap) {


    // added by karun
    let (tx, rx) : (UnboundedSender<axum_Message>, UnboundedReceiver<axum_Message>) = unbounded();
    // let (tx, rx) = unbounded();
    peer_map.lock().unwrap().insert(who, tx);
    let (outgoing, incoming) = websocket.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        println!("Received a message from {}: {}", who, msg.to_text().unwrap());
        let peers = peer_map.lock().unwrap();

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

    println!("{} disconnected", &who);
    peer_map.lock().unwrap().remove(&who);



    // end of added by karun









    //send a ping (unsupported by some browsers) just to kick things off and get a response
    // if socket.send(Message::Ping(vec![1, 2, 3])).await.is_ok() {
    //     println!("Pinged {}...", who);
    // } else {
    //     println!("Could not send ping {}!", who);
    //     // no Error here since the only thing we can do is to close the connection.
    //     // If we can not send messages, there is no way to salvage the statemachine anyway.
    //     return;
    // }

    // // receive single message from a client (we can either receive or send with socket).
    // // this will likely be the Pong for our Ping or a hello message from client.
    // // waiting for message from a client will block this task, but will not block other client's
    // // connections.
    // if let Some(msg) = socket.recv().await {
    //     if let Ok(msg) = msg {
    //         if process_message(msg, who).is_break() {
    //             return;
    //         }
    //     } else {
    //         println!("client {who} abruptly disconnected");
    //         return;
    //     }
    // }

    // // Since each client gets individual statemachine, we can pause handling
    // // when necessary to wait for some external event (in this case illustrated by sleeping).
    // // Waiting for this client to finish getting its greetings does not prevent other clients from
    // // connecting to server and receiving their greetings.
    // for i in 1..5 {
    //     if socket
    //         .send(Message::Text(format!("Hi {i} times!")))
    //         .await
    //         .is_err()
    //     {
    //         println!("client {who} abruptly disconnected");
    //         return;
    //     }
    //     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // }

    // // By splitting socket we can send and receive at the same time. In this example we will send
    // // unsolicited messages to client based on some sort of server's internal event (i.e .timer).
    // let (mut sender, mut receiver) = socket.split();

    // // Spawn a task that will push several messages to the client (does not matter what client does)
    // let mut send_task = tokio::spawn(async move {
    //     let n_msg = 20;
    //     for i in 0..n_msg {
    //         // In case of any websocket error, we exit.
    //         if sender
    //             .send(Message::Text(format!("Server message {i} ...")))
    //             .await
    //             .is_err()
    //         {
    //             return i;
    //         }

    //         tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    //     }

    //     println!("Sending close to {who}...");
    //     if let Err(e) = sender
    //         .send(Message::Close(Some(CloseFrame {
    //             code: axum::extract::ws::close_code::NORMAL,
    //             reason: Cow::from("Goodbye"),
    //         })))
    //         .await
    //     {
    //         println!("Could not send Close due to {}, probably it is ok?", e);
    //     }
    //     n_msg
    // });

    // // This second task will receive messages from client and print them on server console
    // let mut recv_task = tokio::spawn(async move {
    //     let mut cnt = 0;
    //     while let Some(Ok(msg)) = receiver.next().await {
    //         cnt += 1;
    //         // print message and break if instructed to do so
    //         if process_message(msg, who).is_break() {
    //             break;
    //         }
    //     }
    //     cnt
    // });

    // // If any one of the tasks exit, abort the other.
    // tokio::select! {
    //     rv_a = (&mut send_task) => {
    //         match rv_a {
    //             Ok(a) => println!("{} messages sent to {}", a, who),
    //             Err(a) => println!("Error sending messages {:?}", a)
    //         }
    //         recv_task.abort();
    //     },
    //     rv_b = (&mut recv_task) => {
    //         match rv_b {
    //             Ok(b) => println!("Received {} messages", b),
    //             Err(b) => println!("Error receiving messages {:?}", b)
    //         }
    //         send_task.abort();
    //     }
    // }

    // returning from the handler closes the websocket connection
    println!("Websocket context {} destroyed", who);
}

// // tokio_tungstenite
// async fn handle_connection(peer_map: PeerMap, raw_stream: TcpStream, addr: SocketAddr) {
//     println!("Incoming TCP connection from: {}", addr);
//     let ws_stream = tokio_tungstenite::accept_async(raw_stream)
//         .await
//         .expect("Error during the websocket handshake occurred");
//     println!("WebSocket connection established: {}", addr);
//     // Insert the write part of this peer to the peer map.
//     let (tx, rx) = unbounded();
//     peer_map.lock().unwrap().insert(addr, tx);

//     let (mut outgoing, mut incoming) = ws_stream.split();

//     let broadcast_incoming = incoming.try_for_each(|msg| {
//         println!("Received a message from {}: {}", addr, msg.to_text().unwrap());
//         let peers = peer_map.lock().unwrap();

//         // We want to broadcast the message to everyone except ourselves.
//         // let broadcast_recipients =
//         //     peers.iter().filter(|(peer_addr, _)| peer_addr != &&addr).map(|(_, ws_sink)| ws_sink);

//         // for recp in broadcast_recipients {
//         //     recp.unbounded_send(msg.clone()).unwrap();
//         // }

//         future::ok(())
//     });

//     let receive_from_others = rx.map(Ok).forward(outgoing);

//     pin_mut!(broadcast_incoming, receive_from_others);
//     future::select(broadcast_incoming, receive_from_others).await;

//     println!("{} disconnected", &addr);
//     peer_map.lock().unwrap().remove(&addr);
// }


// async fn listen_for_connections(listener: TcpListener, state: PeerMap){
//     println!("listening for connections");
//     while let Ok((stream, addr)) = listener.accept().await {
//         tokio::spawn(handle_connection(state.clone(), stream, addr));
//     }
// }

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
