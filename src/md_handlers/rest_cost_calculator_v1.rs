use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{OriginalUri, Query}, // Path,Json,
    http::Response,                // , Uri Request,
    response::IntoResponse,
    Extension,
};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::functions::URIs;
use tracing::info;

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct RestCostCalculatorV1RequestBody {
    currency_pair: String,
    exchanges: Vec<String>,
    quantity: String,
    side: String,
    use_fees: Option<bool>,
    use_funding_currency: Option<bool>,
}

pub async fn handle_request(
    original_uri: OriginalUri,
    // Path(currency_pair): Path<String>,
    _uris: Extension<URIs>,
    Query(params): Query<HashMap<String, String>>,
    // req: Request<Body>,
    // Json(_body): Json<RestCostCalculatorV1RequestBody>,
) -> impl IntoResponse {
    info!(
        "Received Authenticated REST Forward Request {}",
        original_uri.0
    );
    let uri = original_uri.0.clone();

    let mut target_url = url::Url::parse(&format!(
        "http://{}/cost-calculator/{}",
        _uris.cbag_uri,
        uri.path().trim_start_matches("/api/cost_calculator/")
    ))
    .unwrap();

    for (key, value) in &params {
        target_url.query_pairs_mut().append_pair(key, value);
    }

    info!("Authenticated REST Forward Request to {}", target_url);

    // Make a GET request to the target server
    let client = Client::new();
    let target_res = client.get(target_url).send().await;

    match target_res {
        Ok(response) => {
            // Extract the status code
            let status = StatusCode::from_u16(response.status().as_u16()).unwrap();
            // Extract the headers
            let headers = response.headers().clone();
            // Extract the body as bytes
            let body_bytes = response.bytes().await.unwrap();
            // Create an Axum response using the extracted parts
            let mut axum_response = Response::new(Body::from(body_bytes));
            *axum_response.status_mut() = status;
            axum_response.headers_mut().extend(headers);

            axum_response
        }
        Err(_err) => {
            // Build a 404 response with a body of type `axum::http::response::Body`
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not Found"))
                .unwrap()
        }
    }
}
