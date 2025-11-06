use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tracing::{error, warn};

use crate::{
    error::{AppError, AppResult},
    layers::service::ServiceLayer,
    types::{
        BalanceOut, GetBalanceParams, GetTokenPriceParams, PriceOut, SwapSimOut, SwapTokensParams,
    },
};

/// Runtime that speaks JSON-RPC 2.0 over stdin/stdout as required by MCP hosts.
pub struct McpServer {
    service: ServiceLayer,
}

impl McpServer {
    pub fn new(service: ServiceLayer) -> Self {
        Self { service }
    }

    /// Start processing JSON-RPC requests until EOF on stdin.
    pub async fn run_stdio(self) -> AppResult<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut writer = BufWriter::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            let bytes = reader.read_line(&mut line).await?;
            if bytes == 0 {
                break;
            }

            if line.trim().is_empty() {
                continue;
            }

            let request: Result<RpcRequest, _> = serde_json::from_str(&line);
            match request {
                Ok(req) => {
                    let response = self.handle_request(req).await;
                    let payload = serde_json::to_vec(&response).map_err(AppError::from)?;
                    writer.write_all(&payload).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                Err(err) => {
                    warn!("failed to parse JSON-RPC request: {err}");
                    let response =
                        RpcResponse::error(Value::Null, -32700, format!("parse error: {err}"));
                    let payload = serde_json::to_vec(&response).map_err(AppError::from)?;
                    writer.write_all(&payload).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&self, req: RpcRequest) -> RpcResponse {
        let RpcRequest {
            method, params, id, ..
        } = req;

        match method.as_str() {
            "get_balance" => {
                self.dispatch::<GetBalanceParams, BalanceOut, _, _>(
                    id,
                    params,
                    |service, parsed| async move { service.get_balance(parsed).await },
                )
                .await
            }
            "get_token_price" => {
                self.dispatch::<GetTokenPriceParams, PriceOut, _, _>(
                    id,
                    params,
                    |service, parsed| async move { service.get_token_price(parsed).await },
                )
                .await
            }
            "swap_tokens" => {
                self.dispatch::<SwapTokensParams, SwapSimOut, _, _>(
                    id,
                    params,
                    |service, parsed| async move { service.swap_tokens(parsed).await },
                )
                .await
            }
            other => {
                warn!("received unknown method {other}");
                RpcResponse::error(id, -32601, format!("method not found: {other}"))
            }
        }
    }

    async fn dispatch<P, T, F, Fut>(
        &self,
        id: Value,
        params_value: Value,
        handler: F,
    ) -> RpcResponse
    where
        P: DeserializeOwned,
        T: Serialize,
        F: Fn(ServiceLayer, P) -> Fut,
        Fut: std::future::Future<Output = AppResult<T>>,
    {
        match parse_params::<P>(params_value) {
            Ok(parsed) => match handler(self.service.clone(), parsed).await {
                Ok(result) => match serde_json::to_value(result) {
                    Ok(value) => RpcResponse::success(id, value),
                    Err(err) => {
                        error!("serialization error: {err}");
                        RpcResponse::error(id, -32603, format!("serialization error: {err}"))
                    }
                },
                Err(err) => {
                    error!("handler error: {err}");
                    let payload = err.to_json_rpc();
                    RpcResponse::error_with_data(id, payload.code, payload.message, payload.data)
                }
            },
            Err(err) => {
                warn!("invalid params: {err}");
                RpcResponse::error(id, -32602, err.to_string())
            }
        }
    }
}

fn parse_params<T: DeserializeOwned>(value: Value) -> Result<T, AppError> {
    serde_json::from_value(value)
        .map_err(|err| AppError::InvalidInput(format!("invalid params: {err}")))
}

fn default_null() -> Value {
    Value::Null
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default = "default_null")]
    params: Value,
    #[serde(default = "default_null")]
    id: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
    id: Value,
}

impl RpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError {
                code,
                message,
                data: json!({}),
            }),
            id,
        }
    }

    fn error_with_data(id: Value, code: i32, message: String, data: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError {
                code,
                message,
                data,
            }),
            id,
        }
    }
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
    data: Value,
}
