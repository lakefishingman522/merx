// use chrono::prelude::*;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, RwLock},
    time::Instant,
};
use tokio::task::JoinHandle;

// use futures_channel::mpsc::{UnboundedSender};
use futures_util::StreamExt;
use tokio::sync::mpsc::Sender;

use axum::extract::ws::CloseFrame;
use axum::extract::ws::Message as axum_Message;
use chrono::{Duration as chrono_Duration, Utc};
use reqwest::Client;
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, tungstenite};
use tracing::{error, info, warn};

use crate::{
    error::{ErrorCode, MerxErrorResponse},
    md_handlers::{cbbo_v1, market_depth_v1, rest_cost_calculator_v1},
    routes_config::{MarketDataType, WebSocketLimitRoute, WebSocketLimitType},
    subscriptions::{SubTraits, Subscription, TimeLog},
    symbols::Symbols,
    user::{UserResponse, Users},
};

pub type Tx = Sender<axum::extract::ws::Message>;

pub type SubscriptionState = HashMap<Subscription, HashMap<SocketAddr, Tx>>;
pub type SubscriptionCount = HashMap<SocketAddr, u32>;
pub type SubscriptionIPCount = HashMap<IpAddr, u32>;
pub type SubscriptionBad = HashMap<Subscription, TimeLog>;

pub type WebSocketLimitState = HashMap<WebSocketLimitRoute, HashMap<String, u32>>;

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

#[derive(Default)]
pub struct ConnectionStateStruct {
    pub subscription_state: RwLock<SubscriptionState>,
    pub subscription_count: RwLock<SubscriptionCount>,
    pub subscription_ip_count: RwLock<SubscriptionIPCount>,

    pub subscription_bad: RwLock<SubscriptionBad>,

    pub websocket_limit_state: RwLock<WebSocketLimitState>,

    pub users: RwLock<Users>,
    pub symbols: RwLock<Symbols>,
    pub symbols_whitelist: Vec<String>,
    pub chart_exchanges: Vec<String>,
    product_to_chart: RwLock<HashMap<String, Chart>>,
}

pub type ConnectionState = Arc<ConnectionStateStruct>;

struct Chart {
    data: Option<String>,
}
impl ConnectionStateStruct {
    pub fn new(symbols_whitelist: Vec<String>, chart_exchanges: Vec<String>) -> Self {
        Self {
            subscription_state: RwLock::new(HashMap::new()),
            subscription_count: RwLock::new(HashMap::new()),
            subscription_ip_count: RwLock::new(HashMap::new()),
            subscription_bad: RwLock::new(HashMap::new()),

            websocket_limit_state: RwLock::new(HashMap::new()),

            users: RwLock::new(Users::default()),
            symbols: RwLock::new(Symbols::default()),
            product_to_chart: RwLock::new(HashMap::new()),
            symbols_whitelist: symbols_whitelist,
            chart_exchanges: chart_exchanges,
        }
    }

    pub fn get_ohlc_chart(&self, product: &str) -> Option<String> {
        let product_to_chart = self.product_to_chart.read().unwrap();
        product_to_chart
            .get(product)
            .map(|chart| chart.data.clone())
            .flatten()
    }

    pub fn subscribe_ohlc_chart(
        &self,
        product: String,
        connection_state: ConnectionState,
        chart_uri: String,
    ) {
        let mut product_to_chart = self.product_to_chart.write().unwrap();
        if !product_to_chart.contains_key(&product) {
            product_to_chart.insert(product.clone(), Chart { data: None });
            tokio::spawn(async move {
                loop {
                    // TODO consider cleaning up the task based on a LastRequested field in the Chart struct
                    let chart = fetch_chart(
                        product.as_str(),
                        chart_uri.as_str(),
                        &connection_state.chart_exchanges,
                    )
                    .await;
                    match chart {
                        Some(response) => {
                            connection_state.product_to_chart.write().unwrap().insert(
                                product.clone().to_string(),
                                Chart {
                                    data: Some(response),
                                },
                            );
                        }
                        None => {
                            info!("Error getting chart for {}", product);
                        }
                    };
                    sleep(Duration::from_millis(30 * 1000)).await;
                }
            });
        }
    }

    pub fn is_pair_whitelisted(&self, pair: &str) -> Result<(), MerxErrorResponse> {
        if self.symbols_whitelist.contains(&pair.to_string()) {
            return Ok(());
        }
        Err(MerxErrorResponse::new(ErrorCode::InvalidCurrencyPair))
    }

    pub fn add_client_to_subscription(
        &self,
        client_address: &SocketAddr,
        subscription: Subscription, //this is the endpoint str
        cbag_uri: String,
        sender: Tx,
        connection_state: ConnectionState,
        websocketlimit_type: Option<&WebSocketLimitRoute>,
        username: String,
    ) -> Result<(), MerxErrorResponse> {
        let now = Instant::now();
        let already_subscribed: bool;
        let mut bad_request = false;
        {
            let mut subscription_bad = self.subscription_bad.write().unwrap();
            if let Some(time_log) = subscription_bad.get_mut(&subscription) {
                if let Some(locked_time) = time_log.locked {
                    if Utc::now() - locked_time < chrono_Duration::minutes(30) {
                        bad_request = true;
                    } else {
                        subscription_bad.remove(&subscription);
                    }
                }
            }
            if bad_request {
                return Err(MerxErrorResponse::new(ErrorCode::LockedSubscription));
            }
        }
        {
            let mut subscription_state = self.subscription_state.write().unwrap();
            let mut subscription_count = self.subscription_count.write().unwrap();
            let mut subscription_ip_count = self.subscription_ip_count.write().unwrap();

            let mut websocket_limit_state = self.websocket_limit_state.write().unwrap();

            // Define websocket_limit_route as mutable
            let mut websocket_limit_route: Option<&mut HashMap<String, u32>> = None;
            let mut key: Option<String> = None;

            if let Some(websocketlimit_type) = websocketlimit_type {
                key = Some(match websocketlimit_type.limit_type {
                    WebSocketLimitType::IP => client_address.ip().to_string(),
                    WebSocketLimitType::Token => username,
                });

                websocket_limit_route = Some(
                    websocket_limit_state
                        .entry(websocketlimit_type.clone())
                        .or_insert(HashMap::new()),
                );
                if let Some(websocket_limit_route) = websocket_limit_route.as_mut() {
                    if let Some(key) = &key {
                        if websocket_limit_route.contains_key(key) {
                            if let Some(count_number) = websocket_limit_route.get_mut(key) {
                                if *count_number >= websocketlimit_type.limit_number {
                                    warn!("Can not add client to subscription, because of websocket limit, it has reached to {}, {}", count_number, client_address);
                                    return Err(MerxErrorResponse::new(
                                        ErrorCode::ReachedWebSocketLimitNumber,
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            already_subscribed = subscription_state.contains_key(&subscription);
            // extract client's ip address from client's websocket address
            let client_ip = client_address.ip(); // extract the IP

            if let Some(subscription_clients) = subscription_state.get_mut(&subscription) {
                if subscription_clients.contains_key(client_address) {
                    return Err(MerxErrorResponse::new(ErrorCode::AlreadySubscribed));
                }
                subscription_clients.insert(*client_address, sender);
            } else {
                let mut new_subscription_clients = HashMap::new();
                new_subscription_clients.insert(*client_address, sender);
                subscription_state.insert(subscription.clone(), new_subscription_clients);
            }

            // increment websocket count
            if let Some(websocket_limit_route) = websocket_limit_route.as_mut() {
                if let Some(key) = &key {
                    if websocket_limit_route.contains_key(key) {
                        if let Some(count_number) = websocket_limit_route.get_mut(key) {
                            *count_number += 1;
                            info!("Increased websocket connection number about {} , key is {}, current connection number is {}", client_address,key,count_number);
                        }
                    } else {
                        websocket_limit_route.insert(key.clone(), 1);
                        info!("Increased websocket connection number about {} , key is {}, current connection number is {}", client_address, key,1);
                    }
                }
            }

            // increment subscription count
            if let Some(count) = subscription_count.get_mut(client_address) {
                *count += 1;
            } else {
                subscription_count.insert(*client_address, 1);
            }

            // increment subscription ip count

            if let Some(ip_count) = subscription_ip_count.get_mut(&client_ip) {
                info!(
                    "Adding the subscription ip count for {}, it was {}",
                    &client_ip, ip_count
                );
                *ip_count += 1;
            } else {
                subscription_ip_count.insert(client_ip, 1);
            }
        }
        let elapsed = now.elapsed();
        info!("Adding client to subscription took {:?}", elapsed);
        if !already_subscribed {
            let _handle = subscribe_to_market_data(subscription, connection_state, cbag_uri);
        }
        Ok(())
    }

    pub fn remove_client_from_subscription(
        &self,
        client_address: &SocketAddr,
        subscription: &Subscription,
    ) -> Result<(), String> {
        let mut subscription_state = self.subscription_state.write().unwrap();
        let mut subscription_count = self.subscription_count.write().unwrap();
        let mut subscription_ip_count = self.subscription_ip_count.write().unwrap();

        if let Some(subscription_clients) = subscription_state.get_mut(subscription) {
            if subscription_clients.contains_key(client_address) {
                subscription_clients.remove(client_address);
            } else {
                return Err("Client not subscribed to this subscription".to_string());
            }
        } else {
            return Err("Subscription does not exist".to_string());
        }

        // decrement subscription count
        if let Some(count) = subscription_count.get_mut(client_address) {
            *count -= 1;
        } else {
            return Err("Client not subscribed to any subscriptions".to_string());
        }
        let client_ip = client_address.ip(); // extract the IP
        if let Some(count) = subscription_ip_count.get_mut(&client_ip) {
            if *count > 0 {
                *count -= 1;
            } else {
                return Err("Count has already reached zero for this client".to_string());
            }
        } else {
            return Err("Client not subscribed to any subscriptions".to_string());
        }

        // remove client from subscription count if count is 0
        if subscription_count.get(client_address).unwrap() == &0 {
            subscription_count.remove(client_address);
        }

        // remove client from subscription ip count if count is 0
        if subscription_ip_count.get(&client_ip).unwrap() == &0 {
            info!(
                "Removing the subscription ip count for {} as it is 0",
                &client_ip
            );
            subscription_ip_count.remove(&client_ip);
        }

        Ok(())
    }

    pub fn remove_client_from_state(
        &self,
        &client_address: &SocketAddr,
        websocketlimit_type: Option<&WebSocketLimitRoute>,
        username: String,
    ) {
        let mut subscription_state = self.subscription_state.write().unwrap();
        let mut subscription_count = self.subscription_count.write().unwrap();
        let mut subscription_ip_count = self.subscription_ip_count.write().unwrap();

        let mut websocket_limit_state = self.websocket_limit_state.write().unwrap();

        let mut removal_count = 0;
        // remove client from subscription state
        for (_, subscription_clients) in subscription_state.iter_mut() {
            if subscription_clients.contains_key(&client_address) {
                subscription_clients.remove(&client_address);
                removal_count += 1;
            }
        }

        info!(
            "Removed {} total websocket connection from {}",
            removal_count, client_address
        );

        // code will execute only when removal_count equals 1
        if removal_count == 1 {
            if let Some(websocketlimit_type) = websocketlimit_type {
                // create a string `key` using a match statement based on the `limit_type` of `websocketlimit_type`
                let key = match websocketlimit_type.limit_type {
                    WebSocketLimitType::IP => client_address.ip().to_string(),
                    WebSocketLimitType::Token => username,
                };

                let mut should_remove = false;

                if let Some(route) = websocket_limit_state.get_mut(websocketlimit_type) {
                    if let Some(count_number) = route.get_mut(&key) {
                        if *count_number < 1 {
                            // log an error message if count_number is less than `1`
                            error!("Removed {} from subscription state, but the number was not decreased because that was 0", client_address);
                        } else {
                            // if count_number is not less than `1`, decrease it by `1`
                            *count_number -= 1;
                            info!("Decreased 1 websocket number about {} from websocket limit number, key was {}", client_address, key);
                        }

                        if *count_number == 0 {
                            should_remove = true;
                        }
                    }
                } else {
                    // log an error message if unable to access `websocket_limit_route` with `key`
                    error!("Removed {} from subscription state, but the number was not decreased because can not access to websocket_limit_route(key)", client_address)
                }

                if should_remove {
                    if let Some(route) = websocket_limit_state.get_mut(websocketlimit_type) {
                        route.remove(&key);
                        info!("Removed websocket number about {} from websocket limit number, because that is 0", client_address);
                    }
                }
            }
        }

        // and from the count
        let count = subscription_count.get(&client_address).map(|c| *c);
        subscription_count.remove(&client_address);
        info!(
            "Removing the subscription count for {} as it is 0",
            client_address
        );

        // regarding ip count, decrese count of clientaddress
        let client_ip = client_address.ip(); // extract the IP
        if let Some(ip_count) = subscription_ip_count.get_mut(&client_ip) {
            match count {
                Some(count) => {
                    // count is of type u32, so no need to dereference it.
                    if *ip_count > count {
                        *ip_count -= count;
                    } else {
                        subscription_ip_count.remove(&client_ip);
                        info!("Removing the subscription ip count for {}", &client_ip);
                    }
                }
                None => {
                    warn!("Count is None");
                }
            }
        }
    }

    pub fn is_client_still_active(&self, &client_address: &SocketAddr) -> bool {
        let subscription_count = self.subscription_count.read().unwrap();
        if let Some(count) = subscription_count.get(&client_address) {
            if count > &0 {
                return true;
            }
        }
        false
    }

    pub fn to_json(&self) -> String {
        // generate a hashmap of from subscription_state of subscription to a vector of client addresses
        let subscription_state = self.subscription_state.read().unwrap();
        let mut subscription_state_json = HashMap::new();
        for (subscription, subscription_clients) in subscription_state.iter() {
            let mut client_addresses = Vec::new();
            for (client_address, _) in subscription_clients.iter() {
                client_addresses.push(client_address.to_string());
            }
            subscription_state_json.insert(subscription.get_url(), client_addresses);
        }

        //create a hashmap of client to subscription count
        let mut subscription_count_json = HashMap::new();
        let subscription_count = self.subscription_count.read().unwrap();
        for (client_address, count) in subscription_count.iter() {
            subscription_count_json.insert(client_address.to_string(), count);
        }

        let subscription_state_json = serde_json::json!({
            "subscription_state": subscription_state_json,
            "subscription_count": subscription_count_json
        });

        serde_json::to_string(&subscription_state_json).unwrap()
    }

    pub fn add_or_update_user(
        &self,
        token: &str,
        user_response: &UserResponse,
    ) -> Result<String, String> {
        let mut users = self.users.write().unwrap();
        users.add_or_update_user(token, user_response)
    }

    pub fn check_user_in_state(
        &self,
        token: &str,
        valid_since_duration: Option<chrono::Duration>,
    ) -> Option<String> {
        let users = self.users.read().unwrap();
        users.check_user_in_state(token, valid_since_duration)
    }

    pub fn validate_exchanges_vector(
        &self,
        username: &str,
        exchanges: &Vec<String>,
        market_data_id: Option<String>,
    ) -> Result<Vec<String>, MerxErrorResponse> {
        let users = self.users.read().unwrap();
        users.validate_exchanges_vector(username, exchanges, market_data_id)
    }

    pub fn add_or_update_symbols(
        &self,
        symbols_update: Symbols,
        response_json_string: String,
    ) -> Result<(), String> {
        let mut symbols_lock = self.symbols.write().unwrap();
        symbols_lock.add_or_update_symbols(symbols_update, response_json_string)
    }

    // general function to check state has what it needs to start
    // accepting subscriptions
    pub fn is_ready(&self) -> bool {
        let symbols_lock = self.symbols.read().unwrap();
        symbols_lock.has_symbols()
    }

    pub fn is_pair_valid(&self, pair: &str) -> Result<(), MerxErrorResponse> {
        let symbols_lock = self.symbols.read().unwrap();
        symbols_lock.is_pair_valid(pair)
    }

    pub fn is_size_filter_valid(
        &self,
        pair: &str,
        size_filter: f64,
    ) -> Result<(), MerxErrorResponse> {
        let symbols_lock = self.symbols.read().unwrap();
        symbols_lock.is_size_filter_valid(pair, size_filter)
    }

    pub fn get_currency_pairs_json(&self) -> Result<String, String> {
        let symbols_lock = self.symbols.read().unwrap();
        symbols_lock.get_currency_pairs_json()
    }

    pub fn check_token_known_to_be_invalid(
        &self,
        token: &str,
        duration_window: Option<chrono::Duration>,
    ) -> bool {
        let users = self.users.read().unwrap();
        users.check_token_known_to_be_invalid(token, duration_window)
    }

    pub fn invalidate_token(&self, token: &str) {
        let mut users = self.users.write().unwrap();
        users.invalidate_token(token)
    }

    pub fn add_attempted_auth(&self, token: &str) {
        let mut users = self.users.write().unwrap();
        users.add_attempted_auth(token)
    }

    pub fn check_if_attempted_auth(
        &self,
        token: &str,
        duration_window: Option<chrono::Duration>,
    ) -> bool {
        let users = self.users.read().unwrap();
        users.check_if_attempted_auth(token, duration_window)
    }

    pub fn get_client_id(&self, username: &str) -> Result<String, String> {
        let users = self.users.read().unwrap();
        users.get_client_id(username)
    }

    pub fn add_or_update_cached_response(&self, endpoint: &str, response: String) {
        let mut symbols_lock = self.symbols.write().unwrap();
        symbols_lock.add_or_update_cached_response(endpoint, response)
    }

    pub fn get_cached_response(&self, endpoint: &str) -> Result<String, String> {
        let symbols_lock = self.symbols.read().unwrap();
        symbols_lock.get_cached_response(endpoint)
    }
}

fn parse_tung_response_body_to_str(body: &Option<Vec<u8>>) -> Result<String, String> {
    //parse body into a string
    match body {
        Some(body) => match std::str::from_utf8(body) {
            Ok(body) => Ok(body.to_string()),
            Err(_) => Err("Unable to parse body".to_string()),
        },
        None => Err("Empty body".to_string()),
    }
}

pub async fn fetch_chart(
    product: &str,
    chart_uri: &str,
    exchanges: &Vec<String>,
) -> Option<String> {
    let now = Utc::now();
    let end_time = now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let start_time = now - Duration::from_secs(60 * 60 * 24);
    let start_time = start_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let interval = String::from("m");
    let size = String::from("0");
    let exchanges_str = exchanges.join(",");
    let endpoint = format!("interval={interval}&product={product}&start_time={start_time}&end_time={end_time}&size={size}&exchanges={exchanges_str}");
    let client = Client::new();
    let target_res = client
        .get(&format!("http://{chart_uri}/ohlc/?{endpoint}"))
        .send()
        .await;
    return match target_res {
        Ok(response) => match response.text().await {
            Ok(response) => Some(response),
            Err(_) => None,
        },
        Err(_err) => {
            info!("Error getting chart for {}", product);
            None
        }
    };
}

#[allow(unused_assignments)]
pub fn subscribe_to_market_data(
    subscription: Subscription,
    connection_state: ConnectionState,
    cbag_uri: String,
) -> JoinHandle<()> {
    let ws_endpoint = subscription.get_url();
    info!("Starting a new subscription to {}", ws_endpoint);
    let full_url = format!("ws://{}{}", cbag_uri, ws_endpoint);

    tokio::spawn(async move {
        // info!("Attempting to connect to {}", &ws_endpoint);
        let url = url::Url::parse(&full_url).unwrap();
        let mut closing_down = false;
        let mut consecutive_errors = 0;
        let mut bad_request = false;
        let mut last_time_listeners_checked = Instant::now() - Duration::from_secs(60);
        let mut active_listeners: Vec<Tx> = Vec::new();
        let mut number_of_active_listeners: usize = 0;
        let timeout_duration_ms = subscription.get_timeout_duration_ms();
        // TODO: after x number of timeouts, should check if clients are still connected
        loop {
            if bad_request || consecutive_errors >= 5 {
                // disconnect from all sockets
                warn!(
                    "Unable to connect, closing down subscription {}",
                    &ws_endpoint
                );
                let mut locked_subscription_state =
                    connection_state.subscription_state.write().unwrap(); //TODO: maybe only need a read lock when sending out the messages
                let mut locked_subscription_count =
                    connection_state.subscription_count.write().unwrap();
                let mut locked_subscription_ip_count =
                    connection_state.subscription_ip_count.write().unwrap();

                {
                    //
                    let mut locked_subscription_bad =
                        connection_state.subscription_bad.write().unwrap(); // TODO: lock requests for some time, 30 mins
                    let current_time = Utc::now();
                    if let Some(time_log) = locked_subscription_bad.get_mut(&subscription) {
                        if (current_time - time_log.temp) < chrono_Duration::minutes(5) {
                            info!("(current_time - time_log.temp) < chrono_Duration::minutes(5)");
                        } else {
                            time_log.locked = Some(current_time);
                            error!("locked!");
                        }
                    } else {
                        locked_subscription_bad.insert(
                            subscription.clone(),
                            TimeLog {
                                temp: current_time,
                                // tried: 0,
                                locked: None,
                            },
                        );
                        warn!("added subscription to temp store: {}", &ws_endpoint);
                    }
                }

                let listener_hash_map = locked_subscription_state.get(&subscription);
                let active_listeners = match listener_hash_map {
                    Some(listeners) => listeners.iter().map(|(_, ws_sink)| ws_sink),
                    None => {
                        info!("Subscription {} no longer required", &ws_endpoint);
                        closing_down = true;
                        break;
                    }
                };
                warn!("Disconnecting clients {}", active_listeners.len());
                //TODO: send something about the subscription here if it wasn't valid
                for recp in active_listeners {
                    // let clients know the subscription was invalid
                    let ms = axum_Message::Text(
                        MerxErrorResponse::new(ErrorCode::InvalidRequest).to_json_str(),
                    );
                    match recp.try_send(ms) {
                        Ok(_) => (),
                        Err(_try_send_error) => {
                            // warn!("Buffer probably full.");
                        }
                    }
                }

                // clean up subscription counts for those clients who were connected
                // for this subscription
                let client_subscriptions = locked_subscription_state
                    .get(&subscription)
                    .unwrap()
                    .clone();
                for (client_address, _) in client_subscriptions.iter() {
                    if let Some(count) = locked_subscription_count.get_mut(client_address) {
                        *count -= 1;
                        if *count == 0 {
                            locked_subscription_count.remove(client_address);
                            info!(
                                "Removing the subscription count for {} as it is 0",
                                client_address
                            );
                        }
                    }
                }

                // clean up subscription ip counts for those clients who were connected for this subscription
                for (client_address, _) in client_subscriptions.iter() {
                    let client_ip = client_address.ip(); // extract the IP

                    if let Some(count) = locked_subscription_count.get(&client_address) {
                        if let Some(ip_count) = locked_subscription_ip_count.get_mut(&client_ip) {
                            if *ip_count > *count {
                                *ip_count -= *count;
                            } else {
                                *ip_count = 0;
                            }

                            if *ip_count == 0 {
                                locked_subscription_ip_count.remove(&client_ip);
                                info!(
                                    "Removing the subscription ip count for {} as it is 0",
                                    &client_ip
                                );
                            }
                        }
                    }
                }

                //remove the subscription from the connection state
                locked_subscription_state.remove(&subscription);
                info!(
                    "Number of subscriptions left: {}",
                    locked_subscription_state.len()
                );
                break;
            }
            let result = match timeout(Duration::from_secs(3), connect_async(url.clone())).await {
                Ok(response) => response,
                Err(err) => {
                    // Error occurred or connection timed out
                    error!("Timeout Error: {:?} for {}", err, &ws_endpoint);
                    consecutive_errors += 1;
                    continue;
                }
            };

            let (ws_stream, _) = match result {
                Ok(whatever) => whatever,
                Err(err) => {
                    // get the error response code
                    match err {
                        tungstenite::Error::Http(response) => {
                            if response.status() == 400 {
                                let body_str =
                                    match parse_tung_response_body_to_str(response.body()) {
                                        Ok(body_str) => body_str,
                                        Err(err) => {
                                            error!("Error parsing body: {}", err);
                                            "Unable to parse body".to_string()
                                        }
                                    };
                                //convert body to a string
                                warn!("Connection Error Status 400: {:?}", body_str);
                                bad_request = true;
                                continue;
                            }
                        }
                        _ => {
                            // another error which was not a HTTP error
                            error!("Connection Error connecting to {}: {:?}", &ws_endpoint, err);
                        }
                    }

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
                let message =
                    timeout(Duration::from_millis(timeout_duration_ms), read.next()).await;

                let msg = match message {
                    Ok(Some(msg)) => msg,
                    Ok(None) => {
                        error!(
                            "Unable to read message, restarting subscription {}",
                            ws_endpoint
                        );
                        break;
                    }
                    Err(_) => {
                        error!("Timed out receiving data from subscription {}", ws_endpoint);
                        break;
                    }
                };
                match msg {
                    Ok(message_text) => {
                        // transform the raw message in accordance with its market data type
                        let message_transform_result: Result<Message, String> =
                            match subscription.get_market_data_type() {
                                MarketDataType::CbboV1 => cbbo_v1::transform_message(message_text),
                                MarketDataType::MarketDepthV1 => {
                                    market_depth_v1::transform_message(message_text, &subscription)
                                }
                                MarketDataType::Direct => Ok(message_text),
                                MarketDataType::RestCostCalculatorV1 => {
                                    rest_cost_calculator_v1::transform_message(message_text)
                                    // Ok(message_text)
                                    // Err("Unexpected Market Data Type".into())
                                }
                            };

                        let message_text = match message_transform_result {
                            Ok(message) => message,
                            Err(err) => {
                                error!("Error transforming message: {}", err);
                                continue;
                            }
                        };

                        // let now = Instant::now();
                        if last_time_listeners_checked.elapsed() > Duration::from_secs(2) {
                            // this is a read lock only
                            {
                                let locked_subscription_state =
                                    connection_state.subscription_state.read().unwrap();
                                active_listeners = locked_subscription_state
                                    .get(&subscription)
                                    .unwrap()
                                    .iter()
                                    .map(|(_, ws_sink)| ws_sink.clone())
                                    .collect::<Vec<Tx>>();
                            }
                            number_of_active_listeners = active_listeners.len();
                            last_time_listeners_checked = Instant::now();
                        }
                        // let elapsed = now.elapsed();
                        // info!("Sending message took {:?}", elapsed);

                        for recp in &active_listeners {
                            let ms = from_tungstenite(message_text.clone()).unwrap();
                            match recp.try_send(ms) {
                                Ok(_) => (),
                                Err(_try_send_error) => {
                                    // warn!("Sending error, client likely disconnected.");
                                }
                            }
                        }

                        // when there are no more clients subscribed to this ws subscription,
                        // we can close in a write lock to avoid race conditions agains
                        // new clients subscriptions that might come in whilst the
                        // subscription is being removed from the connection state
                        if number_of_active_listeners == 0 {
                            let mut locked_subscription_state =
                                connection_state.subscription_state.write().unwrap();
                            // check again there are no new clients for the subsctiption
                            // client_subscriptions = connection_state_lock.
                            let listener_hash_map = locked_subscription_state.get(&subscription);
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
                                locked_subscription_state.remove(&subscription);
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
