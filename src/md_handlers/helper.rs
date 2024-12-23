pub fn cbag_market_to_exchange(market: &str) -> String {
    //change market to lowercase
    let market = market.to_lowercase();
    let market = match market.strip_suffix("#na") {
        Some(market) => market,
        None => &market,
    };
    let market = match market.strip_suffix("%23na") {
        Some(market) => market,
        None => market,
    };
    let market = match market.find('-') {
        Some(index) => &market[index + 1..],
        None => market,
    };
    match market.to_lowercase().as_str() {
        "coinbase" => String::from("gdax"),
        "huobi" => String::from("huobipro"),
        _ => market.to_lowercase(),
    }
}

pub fn exchange_to_cbag_market(
    exchange: &str,
    client_id: &str,
    non_agg_prices: bool,
    customer_specific: bool,
    _public: bool,
    market_data_id: Option<String>,
) -> String {
    let mut exchange_str = match exchange.to_lowercase().as_str() {
        "gdax" => String::from("COINBASE"),
        "huobipro" => String::from("HUOBI"),
        _ => exchange.to_uppercase(),
    };
    if customer_specific {
        let client_id = match market_data_id {
            Some(id) => client_id.to_string() + "_" + id.as_str(),
            None => client_id.to_string(),
        };
        exchange_str = format!("{}-{}", client_id, exchange_str);
    }
    if non_agg_prices {
        exchange_str.push_str("%23NA");
    }
    exchange_str
}
