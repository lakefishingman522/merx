use phf::phf_map;

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub enum MarketDataType {
    CbboV1,
    MarketDepthV1,
    RestCostCalculatorV1,
    Direct, // for pass through requests, used on proxy only
}

//TODO The maps below may not be required?

#[derive(Debug, Clone)]
pub enum SubscriptionType {
    Direct,
    Subscription,
    PublicSubscription,
    AuthenticatedRest,
}
pub static ROUTES: phf::Map<&'static str, MarketDataType> = phf_map! {
    // "/legacy-cbbo/:symbol" => Routes::CBBO,
    "/api/streaming/cbbo" => MarketDataType::CbboV1,
    "/api/streaming/market_depth" => MarketDataType::MarketDepthV1,
    "/api/cost_calculator" => MarketDataType::RestCostCalculatorV1,
    "/api/public/streaming/cbbo" => MarketDataType::CbboV1,
    "/api/public/streaming/market_depth" => MarketDataType::MarketDepthV1,
};

pub static SUB_TYPE: phf::Map<&'static str, SubscriptionType> = phf_map! {
    // "/legacy-cbbo/:symbol" => Routes::CBBO,
    "/api/streaming/cbbo" => SubscriptionType::Subscription,
    "/api/streaming/market_depth" => SubscriptionType::Subscription,
    "/api/cost_calculator" => SubscriptionType::AuthenticatedRest,
    "/api/public/streaming/cbbo" => SubscriptionType::PublicSubscription,
    "/api/public/streaming/market_depth" => SubscriptionType::PublicSubscription,
};

#[derive(Eq, Hash, PartialEq, Clone, Debug)]
pub enum WebSocketLimitType {
    IP,
    Token,
}
#[derive(Eq, Hash, PartialEq, Clone, Debug)]
pub struct WebSocketLimitRoute {
    pub path: &'static str,
    pub limit_type: WebSocketLimitType,
    pub limit_number: u32,
}

pub static WS_LIMIT_ROUTES: phf::Map<&'static str, WebSocketLimitRoute> = phf_map! {
    "/api/streaming/cbbo" => WebSocketLimitRoute {
        path: "/api/streaming/cbbo",
        limit_type: WebSocketLimitType::Token,
        limit_number: 100
    },
    "/api/streaming/market_depth" => WebSocketLimitRoute {
        path: "/api/streaming/market_depth",
        limit_type: WebSocketLimitType::Token,
        limit_number: 100
    },
};
