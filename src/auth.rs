use axum::{extract::Query, http::HeaderMap};
use reqwest::Client;
use std::collections::HashMap;

#[allow(unused_imports)]
use tracing::{error, info, warn};

pub async fn check_token_and_authenticate(
    headers: &HeaderMap,
    Query(params): &Query<HashMap<String, String>>,
    auth_uri: &str,
) -> Result<(), String> {
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
    match authenticate_token(auth_uri, token).await {
        Ok(_) => {
            // authenticated ok
            Ok(())
        }
        Err(_) => {
            warn!("Unable to authenticate token");
            Err("Unable to authenticate token".to_string())
        }
    }
}

pub async fn authenticate_token(auth_uri: &str, token: &str) -> Result<(), ()> {
    let client = Client::new();
    let token = if token.starts_with("Token ") {
        token.trim_start_matches("Token ")
    } else {
        token
    };

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
                Ok(())
            }
            _ => {
                println!(
                    "auth failed for token {} with status {}",
                    token,
                    res.status()
                );
                Err(())
            }
        },
        Err(e) => {
            println!("error: {:?}", e);
            Err(())
        }
    }
}
