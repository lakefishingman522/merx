use axum::{extract::Query, http::HeaderMap};
use reqwest::{Client, Response};
use std::collections::HashMap;

// import UserResponse struct
use crate::{state::ConnectionState, user::UserResponse};

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
        .header(reqwest::header::USER_AGENT, "merckx")
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
