pub fn cbag_market_to_exchange(market: &str) -> String {
    match market.to_lowercase().as_str() {
        "coinbase" => String::from("gdax"),
        "huobi" => String::from("huobipro"),
        _ => market.to_lowercase(),
    }
}

pub fn exchange_to_cbag_market(exchange: &str) -> String {
    match exchange.to_lowercase().as_str() {
        "gdax" => String::from("COINBASE"),
        "huobipro" => String::from("HUOBI"),
        _ => exchange.to_uppercase(),
    }
}

