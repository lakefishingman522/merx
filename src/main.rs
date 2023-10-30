use argh::FromArgs;
use axum::extract::Extension;
use axum::routing::post;
use axum::{routing::get, Router};
use std::sync::Arc;
use std::{net::SocketAddr, sync::RwLock};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

// use cade_md_proxy::subscribe_to_market_data;
// mod functions;

mod auth;
mod functions;
mod md_handlers;
mod routes_config;
mod state;
mod symbols;
mod tasks;
mod user;

// import functions.rs
use merckx::functions::{
    authenticate_user, axum_ws_handler, currency_pairs, fallback, forward_request, get_state, root,
    URIs,
};
use merckx::md_handlers::rest_cost_calculator_v1;
use merckx::state::ConnectionState;
use merckx::tasks::start_pull_symbols_task;

#[derive(FromArgs)]
/// Merckx is a market data handler
struct Args {
    /// the uri for the cbag. Can be cbag load balancer
    #[argh(option, default = "String::from(\"none\")")]
    cbag_uri: String,

    /// the uri for the authentication server. This is normally portal.coinroutes.com
    #[argh(option, default = "String::from(\"none\")")]
    auth_uri: String,

    /// auth token used to pull currency pairs. This is a token that can be authenticated on portal
    #[argh(option, default = "String::from(\"none\")")]
    token: String,

    /// optional: specify port for gRPC server. 5050 by default
    #[argh(option, default = "5050")]
    port: u16,

    /// optional: if you want to run merckx in production mode. Will serve on 0.0.0.0
    #[argh(switch)]
    prod: bool,
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
    if args.auth_uri == "none" {
        panic!("auth-uri is required")
    }
    if args.token == "none" {
        panic!("token is required")
    }
    let uris = URIs {
        cbag_uri: args.cbag_uri.clone(),
        auth_uri: args.auth_uri.clone(),
    };
    // let cbag_uri = args.cbag_uri.clone();
    // let auth_uri = args.auth_uri.clone();
    let port = args.port;

    info!("Running Merckx");
    info!("CBAG Uri  : {}", uris.cbag_uri);
    info!("Auth Server Uri  : {}", uris.auth_uri);
    info!("Proxy port: {}", port);

    // connection state which will hold a state of all subscriptions
    // and their respective clients
    let connection_state = ConnectionState::default();
    // will hold the number of subscriptions per client. Useful for knowing
    // when to disconnect from the websocket session

    //start the pull symbols task•
    let pull_symbols_task =
        start_pull_symbols_task(connection_state.clone(), uris.auth_uri.clone(), args.token).await;

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        // REST endpoints
        .route("/version", get(forward_request))
        .route("/state", get(get_state))
        .route("/authenticate_user", get(authenticate_user))
        .route("/api/currency_pairs", get(currency_pairs))
        .route("/api/currency_pairs/", get(currency_pairs))
        // .route("/book/:symbol", get(forward_request))
        // .route("/properties/:symbol", get(forward_request))
        // .route("/legacy-cbbo/:symbol", get(forward_request))
        // .route("/cost-calculator/:symbol", get(forward_request))
        .route(
            "/api/cost_calculator",
            post(rest_cost_calculator_v1::handle_request),
        )
        // WS Endpoints
        // .route("/ws/snapshot/:subscription", get(axum_ws_handler))
        // .route("/ws/bookstats/:symbol", get(axum_ws_handler))
        // .route("/ws/legacy-cbbo/:symbol", get(axum_ws_handler))
        // .route("/ws/cost-calculator/:symbol", get(axum_ws_handler))
        .route("/api/streaming/cbbo", get(axum_ws_handler))
        .route("/api/streaming/cbbo/", get(axum_ws_handler))
        .route("/api/streaming/market_depth", get(axum_ws_handler))
        .route("/api/streaming/market_depth/", get(axum_ws_handler))
        .fallback(fallback)
        .with_state(connection_state.clone())
        .layer(Extension(uris))
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    let addr = if args.prod {
        SocketAddr::from(([0, 0, 0, 0], port))
    } else {
        SocketAddr::from(([127, 0, 0, 1], port))
    };

    info!("Starting Merckx Server on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
