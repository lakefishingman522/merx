use axum::{extract::Query, http::HeaderMap};
use reqwest::Client;
use std::collections::HashMap;

// import UserResponse struct
use crate::{
    state::ConnectionState,
    symbols::{CurrencyPairsResponse, Symbols},
    user::UserResponse,
};

#[allow(unused_imports)]
use tracing::{error, info, warn};

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
        // info!("Authorization: {}", token);
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
    if let Some(username) = connection_state.check_user_in_state(token) {
        return Ok(username);
    }

    match authenticate_token(auth_uri, token, connection_state).await {
        Ok(user) => {
            // authenticated ok
            Ok(user)
        }
        Err(_) => {
            warn!("Unable to authenticate token");
            Err("Unable to authenticate token".to_string())
        }
    }
}

pub async fn authenticate_token(
    auth_uri: &str,
    token: &str,
    connection_state: ConnectionState,
) -> Result<String, String> {
    let client = Client::new();
    let auth_address = format!("https://{}/api/user/", auth_uri);

    let res = client
        .get(auth_address)
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
                                info!("Added user to connection state: {} {}", username, token);
                                return Ok(username);
                            }
                            Err(e) => {
                                error!("Unable to add user to connection state: {:?}", e);
                                return Err("Unable to add user to connection state".to_string());
                            }
                        }
                    }
                    Err(e) => {
                        error!("Unable to parse user response: {:?}", e);
                        return Err("Unable to parse user response".to_string());
                    }
                }
            }
            _ => {
                warn!(
                    "auth failed for token {} with status {}",
                    token,
                    res.status()
                );
                Err("auth failed".to_string())
            }
        },
        Err(e) => {
            println!("error: {:?}", e);
            Err("error: {:?}".to_string())
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

pub async fn get_symbols(
    auth_uri: &str,
    token: &str,
    connection_state: ConnectionState,
) -> Result<(), String> {
    let client = Client::new();
    let currency_pairs_address = format!("https://{}/api/currency_pairs/", auth_uri);

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

                    // match connection_state.add_or_update_symbols(&currency_pairs_response){
                    //     Ok(_) => {
                    //         info!("Added symbols to connection state");
                    //         return Ok(());
                    //     }
                    //     Err(e) => {
                    //         error!("Unable to add symbols to connection state: {:?}", e);
                    //         return Err("Unable to add symbols to connection state".to_string());
                    //     }
                    // }
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
