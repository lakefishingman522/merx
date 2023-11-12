use crate::cached_routes::CACHED_ENDPOINTS;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::{
    auth::{get_data_from_auth_server, get_symbols},
    state::ConnectionState,
};
use tracing::{error, info, warn};

pub async fn start_pull_symbols_task(
    connection_state: ConnectionState,
    auth_uri: String,
    token: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting the pull symbols task");
        loop {
            // for api/currency_pairs
            match get_symbols(&auth_uri, &token, connection_state.clone()).await {
                Ok(symbols) => symbols,
                Err(e) => {
                    error!("Unable to get symbols: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            // for all other cached endpoints
            for endpoint in CACHED_ENDPOINTS.iter() {
                tokio::time::sleep(Duration::from_secs(5)).await;
                match get_data_from_auth_server(&auth_uri, &token, endpoint).await {
                    Ok(response) => {
                        connection_state.add_or_update_cached_response(endpoint, response);
                        info!("Updated cached response for endpoint {}", endpoint)
                    }
                    Err(e) => {
                        warn!(
                            "Unable to get data from auth server for endpoint {}: {}",
                            endpoint, e
                        );
                    }
                };
            }
            tokio::time::sleep(Duration::from_secs(40)).await;
        }
    })
}
