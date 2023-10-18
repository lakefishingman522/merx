use phf::phf_map;

#[derive(Debug)]
pub enum MarketDataType {
    CbboV1,
    RestCostCalculatorV1,
}

#[derive(Debug, Clone)]
pub enum SubscriptionType {
    DIRECT,
    SUBSCRIPTION,
    AUTHENTICATED_REST,
}
pub static ROUTES: phf::Map<&'static str, MarketDataType> = phf_map! {
    // "/legacy-cbbo/:symbol" => Routes::CBBO,
    "/api/streaming/cbbo" => MarketDataType::CbboV1,
    "/api/cost-calculator" => MarketDataType::CbboV1,
};

pub static SUB_TYPE: phf::Map<&'static str, SubscriptionType> = phf_map! {
    // "/legacy-cbbo/:symbol" => Routes::CBBO,
    "/api/streaming/cbbo" => SubscriptionType::SUBSCRIPTION,
    "/api/cost-calculator" => SubscriptionType::AUTHENTICATED_REST,
};
