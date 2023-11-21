use serde::{Serialize, Serializer};
use std::fmt;

#[derive(Debug, Serialize)]

pub struct MerxErrorResponse {
    #[serde(serialize_with = "serialize_type")]
    r#type: &'static str,
    error_code: ErrorCode,
    error: String,
    error_text: String,
}

//TODO: This should be put into an enum that contains all sendable messages
impl MerxErrorResponse {
    pub fn new(error_code: ErrorCode) -> Self {
        Self {
            r#type: "error",
            error_code,
            error: error_code.to_string(),
            error_text: error_code.default_error_text().to_string(),
        }
    }

    pub fn new_and_override_error_text(error_code: ErrorCode, error_text: &str) -> Self {
        Self {
            r#type: "error",
            error_code,
            error: error_code.to_string(),
            error_text: error_text.to_string(),
        }
    }

    pub fn to_json_str(&self) -> String {
        serde_json::to_string(self).unwrap_or(
            serde_json::json!(
                {"type": "error",
                "error_code": 101001,
                "error": "RESPONSE_ERROR",
                "error_text": "Unable to generate error response",
            })
            .to_string(),
        )
    }
}

fn serialize_type<S>(_: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str("error")
}

#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
pub enum ErrorCode {
    // subscription errors
    InvalidCurrencyPair = 100001,
    InvalidSubscriptionMessage = 100002,
    InvalidSizeFilter = 100003,
    InvalidDepthLimit = 100004,
    InvalidExchanges = 100005,
    InvalidRequest = 100006,
    InvalidSample = 100007,
    // system errors
    ResponseError = 101001,
    InvalidToken = 101002,
    AuthServiceUnavailable = 101003,
    AuthInternalError = 101004,
    ServerInitializing = 101005,
    AlreadySubscribed = 101006,
    ServerError = 101007,
}

impl serde::ser::Serialize for ErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_u32(*self as u32)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorCode::InvalidCurrencyPair => write!(f, "INVALID_CURRENCY_PAIR"),
            ErrorCode::InvalidSubscriptionMessage => write!(f, "INVALID_SUBSCRIPTION_MESSAGE"),
            ErrorCode::InvalidSizeFilter => write!(f, "INVALID_SIZE_FILTER"),
            ErrorCode::InvalidDepthLimit => write!(f, "INVALID_DEPTH_LIMIT"),
            ErrorCode::InvalidExchanges => write!(f, "INVALID_EXCHANGES"),
            ErrorCode::InvalidRequest => write!(f, "INVALID_REQUEST"),
            ErrorCode::InvalidSample => write!(f, "INVALID_SAMPLE"),
            ErrorCode::ResponseError => write!(f, "RESPONSE_ERROR"),
            ErrorCode::InvalidToken => write!(f, "INVALID_TOKEN"),
            ErrorCode::AuthServiceUnavailable => write!(f, "AUTH_SERVICE_UNAVAILABLE"),
            ErrorCode::AuthInternalError => write!(f, "AUTH_INTERNAL_ERROR"),
            ErrorCode::ServerInitializing => write!(f, "AWAITING_SYMBOL_DATA"),
            ErrorCode::AlreadySubscribed => write!(f, "ALREADY_SUBSCRIBED"),
            ErrorCode::ServerError => write!(f, "SERVER_ERROR"),
        }
    }
}

impl ErrorCode {
    fn default_error_text(&self) -> &'static str {
        match self {
            ErrorCode::InvalidCurrencyPair => "Please check currency pair",
            ErrorCode::InvalidSubscriptionMessage => {
                "Unable to parse subscription message. Please check all parameter types are correct"
            }
            ErrorCode::InvalidSizeFilter => "Please check size filter is valid",
            ErrorCode::InvalidDepthLimit => "Please check depth limit is valid",
            ErrorCode::InvalidExchanges => "Please check exchanges are valid",
            ErrorCode::InvalidRequest => "Unable to get data. Please check request parameters",
            ErrorCode::InvalidSample => "Please check sample parameter is valid",
            ErrorCode::ResponseError => "Unable to generate error response",
            ErrorCode::InvalidToken => "Invalid Token. Please try again with a valid token",
            ErrorCode::AuthServiceUnavailable => {
                "Authentication service unavailable. Please try again later"
            }
            ErrorCode::AuthInternalError => {
                "Authentication service internal error. Please try again later"
            }
            ErrorCode::ServerInitializing => {
                "Server is initializing and awaiting metadata. Please try again later"
            }
            ErrorCode::AlreadySubscribed => "Already subscribed to this subscription",
            ErrorCode::ServerError => "Unexpected Server Error",
        }
    }
}
