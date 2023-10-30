use chrono::prelude::*;
use chrono::Duration;
use std::collections::HashMap;

use log::info;
use serde::{Deserialize, Serialize};

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
    organization: String,
    client_id: String,
    exchanges: Vec<ExchangeResponse>,
}

struct Exchange {
    slug: String,
    name: String,
    is_real_time: bool,
    has_fok: bool,
    has_post: bool,
    non_aggregated_prices: bool,
    public: bool,
    customer_specific: bool,
    cbag_market: String,
}

pub struct User {
    username: String,
    token: String,
    organization: String,
    client_id: String,
    exchanges: HashMap<String, Exchange>,
    all_cbag_markets: String,
    time_validated: DateTime<Utc>,
}

#[derive(Default)]
pub struct Users {
    users: HashMap<String, User>,
}

impl Users {
    pub fn add_or_update_user(
        &mut self,
        token: &str,
        user_response: &UserResponse,
    ) -> Result<String, String> {
        let mut exchanges = HashMap::new();
        for exchange_response in &user_response.exchanges {
            let cbag_market = exchange_to_cbag_market(
                &exchange_response.slug,
                &user_response.organization,
                exchange_response.non_aggregated_prices,
                exchange_response.customer_specific,
                exchange_response.public,
            );

            let exchange = Exchange {
                slug: exchange_response.slug.clone(),
                name: exchange_response.name.clone(),
                is_real_time: exchange_response.is_real_time,
                has_fok: exchange_response.has_fok,
                has_post: exchange_response.has_post,
                non_aggregated_prices: exchange_response.non_aggregated_prices,
                public: exchange_response.public,
                customer_specific: exchange_response.customer_specific,
                cbag_market: cbag_market,
            };

            exchanges.insert(exchange.slug.clone(), exchange);
        }

        let cbag_markets_string = exchanges
            .iter()
            .map(|(_, exchange)| exchange.cbag_market.clone())
            .collect::<Vec<String>>()
            .join(",");
        let mut user = User {
            username: user_response.username.clone(),
            token: token.to_string(),
            organization: user_response.organization.clone(),
            client_id: user_response.client_id.clone(),
            exchanges: exchanges,
            time_validated: Utc::now(),
            all_cbag_markets: cbag_markets_string,
        };

        let username = user.username.clone();
        self.users.insert(user.username.clone(), user);
        Ok(username)
    }

    pub fn check_user_in_state(&self, token: &str) -> Option<String> {
        let user = self.users.values().find(|user| user.token == token);
        match user {
            Some(user) => {
                info!("User found in state");
                let now = Utc::now();
                let time_validated = user.time_validated;
                let duration = now.signed_duration_since(time_validated);
                if duration > Duration::minutes(5) {
                    //TODO: change this, it is hardcoded for now
                    None
                } else {
                    Some(user.username.clone())
                }
            }
            None => None,
        }
    }

    pub fn validate_exchanges_string(
        &self,
        username: &str,
        exchanges_string: &str,
    ) -> Result<String, String> {
        let user = self.users.get(username);
        match user {
            Some(user) => {
                let exchanges: Vec<&str> = exchanges_string.split(",").collect();
                let mut cbag_markets = Vec::new();
                for exchange in exchanges {
                    if let Some(exchange) = user.exchanges.get(exchange) {
                        cbag_markets.push(exchange.cbag_market.clone());
                    } else {
                        return Err(format!(
                            "User {} does not have access to exchange {}",
                            username, exchange
                        ));
                    }
                }
                Ok(cbag_markets.join(","))
            }
            None => Err(format!("User {} not found", username)),
        }
    }

    pub fn validate_exchanges_vector(
        &self,
        username: &str,
        exchanges_vec: &Vec<String>,
    ) -> Result<String, String> {
        let user = self.users.get(username);
        match user {
            Some(user) => {
                let mut cbag_markets = Vec::new();
                for exchange in exchanges_vec {
                    if let Some(exchange) = user.exchanges.get(exchange) {
                        cbag_markets.push(exchange.cbag_market.clone());
                    } else {
                        return Err(format!(
                            "User {} does not have access to exchange {}",
                            username, exchange
                        ));
                    }
                }
                Ok(cbag_markets.join(","))
            }
            None => Err(format!("User {} not found", username)),
        }
    }

    pub fn get_all_cbag_markets_string(&self, username: &str) -> Result<String, String> {
        let user = self.users.get(username);
        match user {
            Some(user) => Ok(user.all_cbag_markets.clone()),
            None => Err(format!("User {} not found", username)),
        }
    }
}