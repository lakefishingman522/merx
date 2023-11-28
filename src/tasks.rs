use crate::{auth::get_and_cache_currency_pairs_v2, cached_routes::CACHED_ENDPOINTS};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::{auth::get_data_from_auth_server, state::ConnectionState};
use tracing::{error, info, warn};

pub async fn start_pull_symbols_task(
    connection_state: ConnectionState,
    auth_uri: String,
    token: String,
    http_scheme: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting the pull symbols task");
        for _ in 0..5 {
            // When merx starts up, try to pull currency pairs directly
            // from merx prod as its much faster than portal and allows merx
            // to start serving requests much quicker
            info!("Attempting to symbols directly from merx");
            match get_and_cache_currency_pairs_v2(
                "merx.coinroutes.com",
                &token,
                connection_state.clone(),
                &http_scheme,
            )
            .await
            {
                Ok(_) => {
                    break;
                }
                Err(e) => {
                    error!("Unable to get symbols directly from merx: {}", e);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
            };
        }
        loop {
            // for api/currency_pairs
            match get_and_cache_currency_pairs_v2(&auth_uri, &token, connection_state.clone(), &http_scheme).await
            {
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
                match get_data_from_auth_server(&auth_uri, &token, endpoint, &http_scheme).await {
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
