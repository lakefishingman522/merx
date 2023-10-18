pub fn cbag_market_to_exchange(market: &str) -> String {
    match market.to_lowercase().as_str() {
        "coinbase" => String::from("gdax"),
        "huobi" => String::from("huobipro"),
        _ => market.to_lowercase(),
    }
}
