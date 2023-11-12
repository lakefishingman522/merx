use serde::{Deserialize, Serialize};
use std::{f32::consts::E, fmt};

#[derive(Debug, Serialize)]
pub struct MerxErrorResponse {
    error_code: ErrorCode,
    error: String,
    error_text: String,
}

//TODO: This should be put into an enum that contains all sendable messages
impl MerxErrorResponse {
    pub fn new(error_code: ErrorCode) -> Self {
        Self {
            error_code: error_code,
            error: error_code.to_string(),
            error_text: error_code.default_error_text().to_string(),
        }
    }

    pub fn new_and_override_error_text(error_code: ErrorCode, error_text: &str) -> Self {
        Self {
            error_code: error_code,
            error: error_code.to_string(),
            error_text: error_text.to_string(),
        }
    }

    pub fn to_json_str(&self) -> String {
        serde_json::to_string(self).unwrap_or(
            serde_json::json!(
            {   "error_code": 101001,
                "error": "RESPONSE_ERROR",
                "error_text": "Unable to generate error response"
            })
            .to_string(),
        )
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
pub enum ErrorCode {
    // subscription errors
    InvalidCurrencyPair = 100001,
    InvalidSubscriptionMessage = 100002,
    InvalidSizeFilter = 100003,
    // system errors
    ResponseError = 101001,
    InvalidToken = 101002,
    AuthServiceUnavailable = 101003,
    AuthInternalError = 101004,
    AwaitingSymbolData = 101005,
}

// write a custom serializer for ErrorCode that uses the number as the value
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
            ErrorCode::ResponseError => write!(f, "RESPONSE_ERROR"),
            ErrorCode::InvalidToken => write!(f, "INVALID_TOKEN"),
            ErrorCode::AuthServiceUnavailable => write!(f, "AUTH_SERVICE_UNAVAILABLE"),
            ErrorCode::AuthInternalError => write!(f, "AUTH_INTERNAL_ERROR"),
            ErrorCode::AwaitingSymbolData => write!(f, "AWAITING_SYMBOL_DATA"),
        }
    }
}

impl ErrorCode {
    fn default_error_text(&self) -> &'static str {
        match self {
            ErrorCode::InvalidCurrencyPair => "Please check currency pair",
            ErrorCode::InvalidSubscriptionMessage => "Unable to parse subscription message",
            ErrorCode::InvalidSizeFilter => "Please check size filter is valid",
            ErrorCode::ResponseError => "Unable to generate error response",
            ErrorCode::InvalidToken => "Invalid Token. Please try again with a valid token",
            ErrorCode::AuthServiceUnavailable => {
                "Authentication service unavailable. Please try again later"
            }
            ErrorCode::AuthInternalError => {
                "Authentication service internal error. Please try again later"
            }
            ErrorCode::AwaitingSymbolData => {
                "Server is initializing and awaiting metadata. Please try again later"
            }
        }
    }
}
