use axum::{extract::Query, http::HeaderMap};
use chrono::Duration;
use reqwest::Client;
use std::{collections::HashMap, u8};

// import UserResponse struct
use crate::{
    error::ErrorCode,
    state::ConnectionState,
    symbols::{CurrencyPairsResponse, CurrencyPairsV2, Symbols},
    user::UserResponse,
};

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub async fn check_token_and_authenticate(
    headers: &HeaderMap,
    Query(params): &Query<HashMap<String, String>>,
    auth_uri: &str,
    connection_state: ConnectionState,
) -> Result<String, String> {
    let token = if let Some(token) = headers.get("Authorization") {
        let token = if let Ok(tok) = token.to_str() {
            tok
        } else {
            warn!("Unable to parse token");
            return Err("Unable to parse token".to_string());
        };
        token
    } else if let Some(token) = params.get("token") {
        token
    } else {
        return Err("No token provided".to_string());
    };
    let token = if token.starts_with("Token ") {
        token.trim_start_matches("Token ")
    } else {
        token
    };
    // we check if the token is known to be invalid within the last 2 minutes
    if connection_state.check_token_known_to_be_invalid(token, Some(Duration::minutes(2))) {
        warn!("Token known to be invalid {}", token);
        return Err("Invalid token".to_string());
    }
    // we check if the token is already validated within the last 5 minutes
    if let Some(username) = connection_state.check_user_in_state(token, Some(Duration::minutes(15)))
    {
        return Ok(username);
    }

    match authenticate_token(auth_uri, token, connection_state.clone()).await {
        Ok(user) => Ok(user),
        Err(error_code) => {
            match error_code {
                ErrorCode::InvalidToken => {
                    connection_state.invalidate_token(token);
                    warn!("Invalid token");
                    Err("Invalid token".to_string())
                }
                _ => {
                    // if we were unable to authenticate the token for any other reason
                    // we recheck the cached tokens without a validity duration and
                    // allow an authentication even if its been cached for a while
                    if let Some(username) = connection_state.check_user_in_state(token, None) {
                        return Ok(username);
                    }
                    warn!("Auth service unavailable");
                    Err("Auth service unavailable".to_string())
                }
            }
        }
    }
}

pub async fn authenticate_token(
    auth_uri: &str,
    token: &str,
    connection_state: ConnectionState,
) -> Result<String, ErrorCode> {
    // first we check that no other thread is currently trying to authenticate
    // if so we can wait instead of spamming auth server with simultanious requests
    let start = std::time::Instant::now();
    while connection_state.check_if_attempted_auth(token, Some(Duration::seconds(5))) {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        //check if the token is invalid
        if connection_state.check_token_known_to_be_invalid(token, Some(Duration::minutes(2))) {
            warn!("Token known to be invalid {}", token);
            return Err(ErrorCode::InvalidToken);
        }
        //check if token is in state
        if let Some(username) =
            connection_state.check_user_in_state(token, Some(Duration::minutes(5)))
        {
            return Ok(username);
        }
        //check if we have waited for more than 15 seconds
        if start.elapsed().as_secs() > 15 {
            return Err(ErrorCode::AuthServiceUnavailable);
        }
    }

    connection_state.add_attempted_auth(token);
    let client = Client::new();
    let auth_address = format!("http://{}/api/user/", auth_uri);

    let mut attempts: u8 = 0;
    let max_attempts = 4;
    loop {
        attempts += 1;
        let res = client
            .get(auth_address.clone())
            .header("Authorization", format!("Token {}", token))
            .header(reqwest::header::USER_AGENT, "merx")
            .send()
            .await;

        match res {
            Ok(res) => match res.status() {
                reqwest::StatusCode::OK => {
                    // parse the body into a user response but match incase it fails
                    let user_response: Result<UserResponse, _> = res.json().await;
                    match user_response {
                        Ok(user_response) => {
                            match connection_state.add_or_update_user(token, &user_response) {
                                Ok(username) => {
                                    info!("Added user to connection state: {}", username);
                                    return Ok(username);
                                }
                                Err(e) => {
                                    error!("Unable to add user to connection state: {:?}", e);
                                    return Err(ErrorCode::AuthInternalError);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Unable to parse user response: {:?}", e);
                            return Err(ErrorCode::AuthInternalError);
                        }
                    }
                }
                reqwest::StatusCode::UNAUTHORIZED => {
                    warn!("unauthorized");
                    return Err(ErrorCode::InvalidToken);
                }
                _ => {
                    warn!("Auth failed for with status {}", res.status());
                    if attempts < max_attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        continue;
                    }
                    return Err(ErrorCode::AuthServiceUnavailable);
                }
            },
            Err(e) => {
                println!("Authentication error: {:?}", e);
                if attempts < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    continue;
                }
                return Err(ErrorCode::AuthServiceUnavailable);
            }
        }
    }
}

// we parse the response to currency pairs here so as not to hold a
// write lock on the connection state for too long
fn parse_currency_pairs_response(
    currency_pairs_response_vec: &Vec<CurrencyPairsResponse>,
) -> Result<Symbols, String> {
    let mut symbols = Symbols::default();
    for currency_pair in currency_pairs_response_vec {
        //ignore currency pairs that are not on any exchange
        if currency_pair.exchanges.is_empty() {
            continue;
        }
        let mut cbbo_sizes_vec = Vec::new();
        for cbbo_size in &currency_pair.target_currency.cbbo_sizes {
            let cbbo_size = match cbbo_size.size.parse::<f64>() {
                Ok(cbbo_size) => cbbo_size,
                Err(e) => {
                    info!("Unable to parse cbbo_size: {:?}", e);
                    return Err("Unable to parse cbbo_size".to_string());
                }
            };
            cbbo_sizes_vec.push(cbbo_size);
        }
        symbols
            .cbbo_sizes
            .insert(currency_pair.slug.clone(), cbbo_sizes_vec);
    }
    Ok(symbols)
}

pub async fn get_data_from_auth_server(
    auth_uri: &str,
    token: &str,
    endpoint: &str,
) -> Result<String, String> {
    let client = Client::new();
    let auth_address = format!("http://{}{}", auth_uri, endpoint);

    let res = client
        .get(auth_address.clone())
        .header("Authorization", format!("Token {}", token))
        .header(reqwest::header::USER_AGENT, "merx")
        .send()
        .await;

    match res {
        Ok(res) => {
            // check the response code
            if res.status() != reqwest::StatusCode::OK {
                return Err(format!("Unable to get {}: {}", endpoint, res.status()));
            }

            let json_string = match res.text().await {
                Ok(json_string) => json_string,
                Err(e) => {
                    error!("Unable to get {}: {:?}", endpoint, e);
                    return Err(format!("Unable to get {}", endpoint));
                }
            };
            Ok(json_string)
        }
        Err(e) => Err(format!("Unable to get {}: {:?}", endpoint, e)),
    }
}

pub async fn get_currency_pairs_v2(
    auth_uri: &str,
    token: &str,
) -> Result<Vec<CurrencyPairsResponse>, String> {
    let client = Client::new();

    let mut still_paginating = true;
    let mut page_number: u8 = 1;

    // print the address
    let mut currency_pairs_vec: Vec<CurrencyPairsResponse> = Vec::new();
    let start_time = std::time::Instant::now();
    while still_paginating {
        let mut url_builder =
            match reqwest::Url::parse(&format!("https://{}/api/currency_pairs_v2", auth_uri)) {
                Ok(url_builder) => url_builder,
                Err(e) => {
                    println!("error: {:?}", e);
                    return Err("Error getting currency pairsV2 from auth server".to_string());
                }
            };
        url_builder
            .query_pairs_mut()
            .append_pair("page", page_number.to_string().as_str());

        let res = client
            .get(url_builder)
            .header("Authorization", format!("Token {}", token))
            .header(reqwest::header::USER_AGENT, "merx")
            .send()
            .await;

        match res {
            Ok(res) => {
                // check the response code
                if res.status() != reqwest::StatusCode::OK {
                    return Err(format!("Unable to get currency pairs: {}", res.status()));
                }

                let json_string = match res.text().await {
                    Ok(json_string) => json_string,
                    Err(e) => {
                        error!("Unable to get currency pairs: {:?}", e);
                        return Err("Unable to get currency pairs".to_string());
                    }
                };

                // parse into a vector of currencyPairsResponse
                let currency_pairs_v2_page: Result<CurrencyPairsV2, _> =
                    serde_json::from_str(json_string.as_str());
                match currency_pairs_v2_page {
                    Ok(currency_pairs_v2_response) => {
                        for currency_pair in currency_pairs_v2_response.results {
                            currency_pairs_vec.push(currency_pair);
                        }

                        // print the size of currency pairs vec
                        info!(
                            "Paginating currency pairs v2. Got {} symbols",
                            currency_pairs_vec.len()
                        );

                        // info!("Got response from auth server: {:?}", currency_pairs_v2_response);

                        match currency_pairs_v2_response.next {
                            Some(_next_page) => {
                                page_number += 1;
                            }
                            None => {
                                still_paginating = false;
                            }
                        }
                    }
                    Err(e) => {
                        println!("error: {:?}", e);
                        return Err("Unable to parse currency pairs response".to_string());
                    }
                }
            }
            Err(e) => {
                println!("error: {:?}", e);
                return Err("Error getting currency pairsV2 from auth server".to_string());
            }
        }
    }
    info!("Got all currency_pairs_v2 in {:?}", start_time.elapsed());
    Ok(currency_pairs_vec)
}

pub async fn get_and_cache_currency_pairs_v2(
    auth_uri: &str,
    token: &str,
    connection_state: ConnectionState,
) -> Result<(), String> {
    let currency_pairs = match get_currency_pairs_v2(auth_uri, token).await {
        Ok(currency_pairs) => currency_pairs,
        Err(e) => {
            println!("error: {:?}", e);
            return Err("Error getting currency pairsV2 from auth server".to_string());
        }
    };

    let symbols = match parse_currency_pairs_response(&currency_pairs) {
        Ok(symbols) => symbols,
        Err(e) => {
            println!("error: {:?}", e);
            return Err("Error parsing currency pairsV2 from auth server".to_string());
        }
    };

    let full_currency_pairs_v2_response = CurrencyPairsV2 {
        count: currency_pairs.len() as i32,
        next: None,
        previous: None,
        results: currency_pairs,
    };

    let full_response_string = match serde_json::to_string(&full_currency_pairs_v2_response) {
        Ok(full_response_string) => full_response_string,
        Err(e) => {
            println!("error: {:?}", e);
            return Err("Error serializing currency pairsV2 from auth server".to_string());
        }
    };

    match connection_state.add_or_update_symbols(symbols, full_response_string) {
        Ok(_) => {
            info!("Added symbols to connection state");
            Ok(())
        }
        Err(e) => {
            error!("Unable to add symbols to connection state: {:?}", e);
            Err("Unable to add symbols to connection state".to_string())
        }
    }
}

pub async fn get_symbols(
    auth_uri: &str,
    token: &str,
    connection_state: ConnectionState,
) -> Result<(), String> {
    let client = Client::new();

    //TODO: can be refactored to use the get_data_from_auth_server function
    let currency_pairs_address = format!("http://{}/api/currency_pairs/", auth_uri);

    let res = client
        .get(currency_pairs_address)
        .header("Authorization", format!("Token {}", token))
        .header(reqwest::header::USER_AGENT, "merx")
        .send()
        .await;

    match res {
        Ok(res) => {
            // check the response code
            if res.status() != reqwest::StatusCode::OK {
                return Err(format!("Unable to get currency pairs: {}", res.status()));
            }

            let json_string = match res.text().await {
                Ok(json_string) => json_string,
                Err(e) => {
                    error!("Unable to get currency pairs: {:?}", e);
                    return Err("Unable to get currency pairs".to_string());
                }
            };

            // parse into a vector of currencyPairsResponse
            let currency_pairs_response: Result<Vec<CurrencyPairsResponse>, _> =
                serde_json::from_str(json_string.as_str());
            match currency_pairs_response {
                Ok(currency_pairs_response) => {
                    let symbols_update = if let Ok(symbols) =
                        parse_currency_pairs_response(&currency_pairs_response)
                    {
                        symbols
                    } else {
                        return Err("Unable to parse currency pairs response".to_string());
                    };

                    match connection_state.add_or_update_symbols(symbols_update, json_string) {
                        Ok(_) => {
                            info!("Added symbols to connection state");
                            return Ok(());
                        }
                        Err(e) => {
                            error!("Unable to add symbols to connection state: {:?}", e);
                            return Err("Unable to add symbols to connection state".to_string());
                        }
                    }
                }
                Err(e) => {
                    println!("error: {:?}", e);
                    return Err("Unable to parse currency pairs response".to_string());
                }
            }
        }
        Err(e) => {
            println!("error: {:?}", e);
        }
    }

    Ok(())
}
