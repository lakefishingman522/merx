use serde::{Deserialize, Serialize};
use std::{f32::consts::E, fmt};

#[derive(Debug, Serialize)]
pub struct MerxErrorResponse {
    error_code: ErrorCode,
    error: String,
    error_text: String,
}

//create a constructor for errorresponse
impl MerxErrorResponse {
    pub fn new(error_code: ErrorCode, override_error_text: Option<&str>) -> Self {
        let err_text = match override_error_text {
            Some(text) => text.to_string(),
            None => error_code.default_error_text().to_string(),
        };

        let error_str = error_code.to_string();

        Self {
            error_code: error_code,
            error: error_str,
            error_text: err_text,
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
        }
    }
}
