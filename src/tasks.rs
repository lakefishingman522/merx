use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::{auth::get_symbols, state::ConnectionState};
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
            let symbols = match get_symbols(&auth_uri, &token, connection_state.clone()).await {
                Ok(symbols) => symbols,
                Err(e) => {
                    error!("Unable to get symbols: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
        }
    })
}
