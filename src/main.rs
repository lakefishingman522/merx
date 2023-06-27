use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use futures_util::{future, pin_mut, stream::TryStreamExt, StreamExt};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

// for axum
use axum::{body::Body, extract::ws::Message as axum_Message, http::Uri};
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::{OriginalUri, Path, State},
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router, TypedHeader,
};

//allows to extract the IP of connecting user
use argh::FromArgs;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
use axum::extract::Extension;
use reqwest::Client;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use lib::*;

pub mod lib;

#[derive(FromArgs)]
/// A Market Data Proxy for CBAG market data requests
struct Args {
    #[argh(option, default = "String::from(\"none\")")]
    /// the uri for the cbag. Can be cbag load balancer
    cbag_uri: String,

    /// optional: specify port for gRPC server. 50051 by default
    #[argh(option, default = "50051")]
    port: u16,
}

#[tokio::main]
async fn main() {
    // Configure the tracing subscriber
    let filter = EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into());
    tracing_subscriber::fmt::Subscriber::builder()
        .with_env_filter(filter)
        .init();

    // parse args
    let args: Args = argh::from_env();
    if args.cbag_uri == "none" {
        panic!("cbag-uri is required")
    }
    let cbag_uri = args.cbag_uri.clone();
    let port = args.port;

    info!("Running Market Data Proxy");
    info!("CBAG Uri  : {}", cbag_uri);
    info!("Proxy port: {}", port);

    // connection state which will hold a state of all subscriptions
    // and their respective clients
    let connection_state = ConnectionState::new(RwLock::new(HashMap::new()));
    let cbag_uri_clone = cbag_uri.clone();

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        // REST endpoints
        .route("/version", get(forward_request))
        .route("/book/:symbol", get(forward_request))
        .route("/properties/:symbol", get(forward_request))
        .route("/legacy-cbbo/:symbol", get(forward_request))
        .route("/cost-calculator/:symbol", get(forward_request))
        // WS Endpoints
        .route("/ws/snapshot/:subscription", get(axum_ws_handler))
        .route("/ws/bookstats/:symbol", get(axum_ws_handler))
        .route("/ws/legacy-cbbo/:symbol", get(axum_ws_handler))
        .route("/ws/cost-calculator/:symbol", get(axum_ws_handler))
        .fallback(fallback)
        .with_state(connection_state.clone())
        .layer(Extension(cbag_uri_clone))
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // run our app with hyper, listening globally on port 3000
    // let listener = tokio::net::TcpListener::bind("0.0.0.0:9089").await.unwrap();
    // axum::serve(listener, app).await.unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("Starting Axum Server on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

