use ethers::providers::ProviderError;
use serde_json::{Value, json};
use std::{fmt, io};
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("price error: {0}")]
    Price(String),
    #[error("swap error: {0}")]
    Swap(String),
    #[error("wallet error: {0}")]
    Wallet(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug)]
pub struct JsonRpcErrorPayload {
    pub code: i32,
    pub message: String,
    pub data: Value,
}

impl JsonRpcErrorPayload {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: json!({}),
        }
    }
}

impl AppError {
    pub fn to_json_rpc(&self) -> JsonRpcErrorPayload {
        match self {
            AppError::Config(msg) => JsonRpcErrorPayload::new(-32001, msg.clone()),
            AppError::InvalidInput(msg) => JsonRpcErrorPayload::new(-32602, msg.clone()),
            AppError::Rpc(msg) => JsonRpcErrorPayload::new(-32002, msg.clone()),
            AppError::Price(msg) => JsonRpcErrorPayload::new(-32010, msg.clone()),
            AppError::Swap(msg) => JsonRpcErrorPayload::new(-32020, msg.clone()),
            AppError::Wallet(msg) => JsonRpcErrorPayload::new(-32030, msg.clone()),
            AppError::Io(msg) => JsonRpcErrorPayload::new(-32040, msg.clone()),
            AppError::Serialization(msg) => JsonRpcErrorPayload::new(-32700, msg.clone()),
            AppError::Internal(msg) => JsonRpcErrorPayload::new(-32603, msg.clone()),
        }
    }
}

impl From<ProviderError> for AppError {
    fn from(err: ProviderError) -> Self {
        AppError::Rpc(err.to_string())
    }
}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        AppError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Serialization(err.to_string())
    }
}

impl fmt::Display for JsonRpcErrorPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}
