use std::{fmt, f32::consts::E};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct MerxErrorResponse{
    error_code: ErrorCode,
    error: String,
    error_text: String
}

//create a constructor for errorresponse
impl MerxErrorResponse{
    pub fn new(error_code: ErrorCode, override_error_text: Option<&str>) -> Self{

        let err_text = match override_error_text{
            Some(text) => text.to_string(),
            None => error_code.default_error_text().to_string()
        };

        let error_str = error_code.to_string();

        Self{
            error_code: error_code,
            error: error_str,
            error_text: err_text
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
pub enum ErrorCode{
    InvalidCurrencyPair = 100001,
    InvalidSubscriptionMessage = 100002,
    InvalidSizeFilter = 100003,
}

// write a custom serializer for ErrorCode that uses the number as the value
impl serde::ser::Serialize for ErrorCode{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer
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
        }
    }
}

impl ErrorCode{
    fn default_error_text(&self) -> &'static str{
        match self {
            ErrorCode::InvalidCurrencyPair => "Please check currency pair",
            ErrorCode::InvalidSubscriptionMessage => "Unable to parse subscription message",
            ErrorCode::InvalidSizeFilter => "Please check size filter is valid",
        }
    }
}

