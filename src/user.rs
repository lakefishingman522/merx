use chrono::prelude::*;
use chrono::Duration;
//TODO: remove chrono and use tokio::time::Instant
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::error;

use crate::error::ErrorCode;
use crate::error::MerxErrorResponse;
use crate::md_handlers::helper::exchange_to_cbag_market;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExchangeResponse {
    slug: String,
    name: String,
    is_real_time: bool,
    has_fok: bool,
    has_post: bool,
    non_aggregated_prices: bool,
    public: bool,
    customer_specific: bool,
}

// the json response from the auth server
#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse {
    username: String,
    is_staff: Option<bool>,
    organization: String,
    client_id: String,
    exchanges: Vec<ExchangeResponse>,
}

#[allow(dead_code)]
struct Exchange {
    slug: String,
    name: String,
    is_real_time: bool,
    has_fok: bool,
    has_post: bool,
    non_aggregated_prices: bool,
    public: bool,
    customer_specific: bool,
}

impl Exchange {
    pub fn to_cbag_market(&self, client_id: &str, market_data_id: Option<String>) -> String {
        exchange_to_cbag_market(
            &self.slug,
            client_id,
            self.non_aggregated_prices,
            self.customer_specific,
            self.public,
            market_data_id,
        )
    }
}

#[allow(dead_code)]
pub struct User {
    username: String,
    token: String,
    organization: String,
    client_id: String,
    exchanges: HashMap<String, Exchange>,
    time_validated: DateTime<Utc>,
}

#[derive(Default)]
pub struct Users {
    users: HashMap<String, User>,
    invalid_tokens: HashMap<String, DateTime<Utc>>,
    attempted_auths: HashMap<String, DateTime<Utc>>,
}

impl Users {
    pub fn add_or_update_user(
        &mut self,
        token: &str,
        user_response: &UserResponse,
    ) -> Result<String, String> {
        let mut exchanges = HashMap::new();
        for exchange_response in &user_response.exchanges {
            let exchange = Exchange {
                slug: exchange_response.slug.clone(),
                name: exchange_response.name.clone(),
                is_real_time: exchange_response.is_real_time,
                has_fok: exchange_response.has_fok,
                has_post: exchange_response.has_post,
                non_aggregated_prices: exchange_response.non_aggregated_prices,
                public: exchange_response.public,
                customer_specific: exchange_response.customer_specific,
            };

            exchanges.insert(exchange.slug.clone(), exchange);
        }

        let user = User {
            username: user_response.username.clone(),
            token: token.to_string(),
            organization: user_response.organization.clone(),
            client_id: user_response.client_id.clone(),
            exchanges,
            time_validated: Utc::now(),
        };

        let username = user.username.clone();
        self.users.insert(user.username.clone(), user);
        self.invalid_tokens.remove(token);
        Ok(username)
    }

    pub fn check_user_in_state(
        &self,
        token: &str,
        validated_since_duration: Option<Duration>,
    ) -> Option<String> {
        //TODO: This is not the most efficient way to tod this
        let user = self.users.values().find(|user| user.token == token);
        match user {
            Some(user) => {
                let now = Utc::now();
                let time_validated = user.time_validated;
                // if a validated since duration has been provided,
                // check if validated within duration
                if let Some(duration) = validated_since_duration {
                    let duration_since_validated = now.signed_duration_since(time_validated);
                    if duration_since_validated > duration {
                        return None;
                    }
                }
                Some(user.username.clone())
            }
            None => None,
        }
    }

    pub fn invalidate_token(&mut self, token: &str) {
        self.invalid_tokens.insert(token.to_string(), Utc::now());
    }

    pub fn add_attempted_auth(&mut self, token: &str) {
        self.attempted_auths.insert(token.to_string(), Utc::now());
    }

    pub fn check_if_attempted_auth(&self, token: &str, duration_window: Option<Duration>) -> bool {
        let now = Utc::now();
        let attempted_auth = self.attempted_auths.get(token);
        match attempted_auth {
            Some(attempted_auth) => {
                let duration_since_attempted_auth = now.signed_duration_since(*attempted_auth);
                if let Some(duration) = duration_window {
                    if duration_since_attempted_auth > duration {
                        return false;
                    }
                }
                true
            }
            None => false,
        }
    }

    pub fn check_token_known_to_be_invalid(
        &self,
        token: &str,
        duration_window: Option<Duration>,
    ) -> bool {
        let now = Utc::now();
        let invalid_token = self.invalid_tokens.get(token);
        match invalid_token {
            Some(invalid_token) => {
                let duration_since_invalidated = now.signed_duration_since(*invalid_token);
                if let Some(duration) = duration_window {
                    if duration_since_invalidated > duration {
                        return false;
                    }
                }
                true
            }
            None => false,
        }
    }

    pub fn validate_exchanges_vector(
        &self,
        username: &str,
        exchanges_vec: &Vec<String>,
        market_data_id: Option<String>,
    ) -> Result<Vec<String>, MerxErrorResponse> {
        let user = self.users.get(username);
        match user {
            Some(user) => {
                let mut cbag_markets = Vec::new();
                for exchange in exchanges_vec {
                    if let Some(exchange) = user.exchanges.get(exchange) {
                        cbag_markets.push(
                            exchange
                                .to_cbag_market(user.client_id.as_str(), market_data_id.clone())
                                .clone(),
                        );
                    } else {
                        return Err(MerxErrorResponse::new_and_override_error_text(
                            ErrorCode::InvalidExchanges,
                            &format!(
                                "Exchange `{}` invalid or user does not have access",
                                exchange
                            ),
                        ));
                    }
                }
                Ok(cbag_markets)
            }
            None => {
                error!(
                    "User {} not found whilst attempting to validate exchanges",
                    username
                );
                Err(MerxErrorResponse::new(ErrorCode::ServerError))
            }
        }
    }

    pub fn get_client_id(&self, username: &str) -> Result<String, String> {
        let user = self.users.get(username);
        match user {
            Some(user) => Ok(user.client_id.clone()),
            None => Err(format!("User {} not found", username)),
        }
    }
}
